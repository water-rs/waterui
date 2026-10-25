//! Native tests that need a running `UIApplication`.
//!
//! `UIKit` drives part of its machinery — the animation behind
//! `UIScrollView.setContentOffset(_:animated: true)` among it — only from
//! the application's update cycle, which the bare `simctl spawn`ed `native`
//! suite never reaches. The `nextest-ios-sim.sh` target runner launches
//! this binary into a real `UIApplication` with a connected window scene,
//! one case per launch, from the bundle manifest embedded below;
//! `cocoa_ui::uikit::native_test::run` refuses to start outside that
//! launch. A launch costs far more than a spawn, so only cases that need
//! the application live here; everything else stays in `native`.
//!
//! The cases are iOS-only: on macOS the binary lists no trials.

#[cfg(target_os = "ios")]
use cocoa_ui::{Rect, uikit::native_test, view};
use libtest_mimic::Arguments;

// The target runner launches this binary as an application from the
// bundle manifest embedded here.
#[cfg(target_os = "ios")]
cocoa_ui::native_test_info_plist!();

fn main() {
    let mut arguments = Arguments::from_args();
    // `UIKit` objects may only be built on the real main thread; a single
    // runner keeps every trial on the application's main thread.
    arguments.test_threads = Some(1);
    #[cfg(target_os = "ios")]
    cocoa_ui::uikit::native_test::run(arguments, trials);
    #[cfg(target_os = "macos")]
    libtest_mimic::run(&arguments, Vec::new()).exit();
}

/// A main-thread marker for cases that construct `UIKit` objects — the
/// harness application guarantees it.
#[cfg(target_os = "ios")]
fn mtm() -> cocoa_ui::MainThreadMarker {
    cocoa_ui::MainThreadMarker::new().expect("native_app cases run on the main thread")
}

/// A key, visible window in the application's scene with `content`
/// filling it, laid out.
#[cfg(target_os = "ios")]
fn scene_window(
    mtm: cocoa_ui::MainThreadMarker,
    content: &cocoa_ui::PlatformView,
) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow> {
    let window = native_test::window(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
    window.addSubview(content);
    view::set_frame(content, view::bounds(&window));
    window.makeKeyAndVisible();
    window.layoutIfNeeded();
    window
}

/// Every `native_app` case: trials that need a running `UIApplication`.
#[cfg(target_os = "ios")]
fn trials() -> Vec<libtest_mimic::Trial> {
    let mut trials = scroll_animation::trials();
    trials.extend(main_menu::trials());
    trials.extend(key_commands::trials());
    trials.extend(gesture_events::trials());
    trials.extend(text_selection_ios::trials());
    trials
}

/// The main menu — the iPad menu bar and the keyboard-shortcut list — is
/// built by `UIKit`'s main menu system inside a running application. The
/// harness application hands `ApplicationHandlers::build_menus` a handler
/// that counts the builds it receives.
#[cfg(target_os = "ios")]
mod main_menu {
    use cocoa_ui::uikit::native_test::main_menu_builds;
    use libtest_mimic::Trial;

    /// The `main_menu::` trials.
    pub fn trials() -> Vec<Trial> {
        vec![Trial::test(
            "main_menu::the_main_menu_builds_through_the_build_menus_handler",
            || {
                the_main_menu_builds_through_the_build_menus_handler();
                Ok(())
            },
        )]
    }

    /// `build_menus` is the main menu system's build handler: `UIKit` has
    /// built the main menu through it by the time the application is up.
    fn the_main_menu_builds_through_the_build_menus_handler() {
        assert!(
            main_menu_builds() > 0,
            "`UIKit` built the main menu through the handler as the application launched"
        );
    }
}

/// `UIKit`-driven animation (#2000): `setContentOffset(_:animated: true)`
/// and `scrollToRow(_:at:animated: true)` advance the model
/// `contentOffset` on the application's update cycle, so they only move
/// inside a running `UIApplication` with a connected scene. Each runs on
/// the platform's wall clock: the offset is still at its start when the
/// request returns, and it lands on the target no sooner than a native
/// animation takes; a process outside the application never moves it.
/// This is where the `UIKit` `Animation::Default` arm of the `native`
/// scroll suites lives.
#[cfg(target_os = "ios")]
mod scroll_animation {
    use cocoa_ui::geometry::{Point, Size};
    use cocoa_ui::uikit::ScrollView;

    use super::{mtm, scene_window};
    use waterui::animation::Animation;
    use waterui::component::list::ListItem;
    use waterui::layout::scroll::ScrollController;
    use waterui::reactive::binding;
    use waterui_apple::native_test_support::{
        APPROACH_LIST_ROWS, APPROACH_ROW, APPROACH_TARGET_ROW, assert_native_scroll,
        assert_native_scroll_from, list_row_top, row_item, row_list, scroll_surface,
    };
    use waterui_core::layout::Point as LayoutPoint;

    /// The list's rows — far taller than the window.
    const ROWS: usize = 200;
    /// The row the list case aims at: below the fold, far from the end.
    const TARGET_ROW: usize = 40;

    /// The `scroll_animation::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        let cases: [(&str, fn()); 4] = [
            (
                "animated_content_offset_lands_after_a_native_animation",
                animated_content_offset_lands_after_a_native_animation,
            ),
            (
                "a_default_surface_request_lands_after_a_native_animation",
                a_default_surface_request_lands_after_a_native_animation,
            ),
            (
                "a_default_list_request_lands_after_a_native_animation",
                a_default_list_request_lands_after_a_native_animation,
            ),
            (
                "a_far_default_list_request_jumps_to_the_approach_row_first",
                a_far_default_list_request_jumps_to_the_approach_row_first,
            ),
        ];
        cases
            .into_iter()
            .map(|(name, case)| {
                libtest_mimic::Trial::test(format!("scroll_animation::{name}"), move || {
                    case();
                    Ok(())
                })
            })
            .collect()
    }

    fn animated_content_offset_lands_after_a_native_animation() {
        let mtm = mtm();
        let scroll = ScrollView::new(mtm, true, false);
        let window = scene_window(mtm, &scroll);
        let viewport = scroll.viewport_size();
        scroll.set_content_extent(Size::new(viewport.width, viewport.height * 10.0));
        scroll.layout_if_needed();

        let start = scroll.content_offset();
        let target = Point::new(start.x, viewport.height.mul_add(3.0, start.y));
        assert_native_scroll(
            "setContentOffset(_:animated: true)",
            || scroll.set_content_offset(target, true),
            || scroll.content_offset(),
            || target,
        );
        window.setHidden(true);
    }

    /// `Animation::Default` on a scroll surface is `UIKit`'s own
    /// `setContentOffset(_:animated: true)`: driven through the
    /// controller, it does not jump, and it lands where the jump to the
    /// same target lands no sooner than a native animation takes.
    fn a_default_surface_request_lands_after_a_native_animation() {
        let mtm = mtm();
        let origin = LayoutPoint::new(0.0, 0.0);
        let target = LayoutPoint::new(0.0, 800.0);
        let controller = ScrollController::new(origin);
        let reported = binding(origin);
        let (leaf, surface) = scroll_surface(&controller, &reported);
        let window = scene_window(mtm, leaf.view());
        surface.set_needs_layout();
        surface.layout_if_needed();

        controller.scroll_to(target);
        let landing = surface.content_offset();
        controller.scroll_to(origin);
        assert_native_scroll(
            "a Default surface request",
            || controller.animate_to(target, Animation::Default),
            || surface.content_offset(),
            || landing,
        );
        window.setHidden(true);
    }

    /// `Animation::Default` on a list is `UIKit`'s own
    /// `scrollToRow(_:at:animated: true)`: driven through the controller
    /// on a cold table, it does not jump, and it lands with the row's top
    /// at the viewport's top no sooner than a native animation takes.
    /// Rows `UIKit` has not shown are sized by estimate, and the scroll
    /// measures them on the way, so the row's top is read as it stands when the offset lands —
    /// not taken from an earlier jump, which resolves against estimates.
    fn a_default_list_request_lands_after_a_native_animation() {
        let mtm = mtm();
        let controller = ScrollController::new(0usize);
        let (leaf, table) = row_list(vec![row_item as fn() -> ListItem; ROWS], &controller);
        let window = scene_window(mtm, leaf.view());
        table.layout_if_needed();
        let offset = || Point::from(table.contentOffset());

        assert_native_scroll(
            "a Default list request",
            || controller.animate_to(TARGET_ROW, Animation::Default),
            offset,
            || list_row_top(&table, TARGET_ROW),
        );
        window.setHidden(true);
    }

    /// A far `Animation::Default` list request animates only the final
    /// stretch: `UIKit`'s native row scroll toward a target further than
    /// the approach bound first jumps unanimated to the approach row's
    /// top — the offset right after the request — and lands from there
    /// with the target row's top at the viewport's top, read as it
    /// stands at landing since unseen rows are sized by estimate.
    fn a_far_default_list_request_jumps_to_the_approach_row_first() {
        let mtm = mtm();
        let controller = ScrollController::new(0usize);
        let (leaf, table) = row_list(
            vec![row_item as fn() -> ListItem; APPROACH_LIST_ROWS],
            &controller,
        );
        let window = scene_window(mtm, leaf.view());
        table.layout_if_needed();

        assert_native_scroll_from(
            "a far Default list request",
            || controller.animate_to(APPROACH_TARGET_ROW, Animation::Default),
            || list_row_top(&table, APPROACH_ROW),
            || Point::from(table.contentOffset()),
            || list_row_top(&table, APPROACH_TARGET_ROW),
        );
        window.setHidden(true);
    }
}

/// `UIKit` menu commands arm their shortcuts as `UIKeyCommand`s (#2117).
///
/// A chord parked on `UIAction.discoverabilityTitle` displayed its
/// `UIKeyInput*` constant and never fired, so a shortcut command builds a
/// `UIKeyCommand`. `UIKit` takes immutable copies of the menus it receives:
/// the command a pressed chord sends is a copy of the element the builder
/// made, and only what `-copy` preserves — its `propertyList` — can lead it
/// back to the command's callback. `UIKit` sends the action untargeted
/// through `sendAction:to:from:forEvent:`, the channel a hardware chord
/// takes, and the responder chain ends at `AppDelegate`, which owns the
/// registry the builders arm commands in. Both exist only inside a running
/// `UIApplication`, so these cases live here rather than in `native`'s bare
/// spawn.
#[cfg(target_os = "ios")]
mod key_commands {
    use std::cell::{Cell, RefCell};
    use std::ptr::NonNull;
    use std::rc::Rc;

    use block2::StackBlock;
    use cocoa_ui::Rect;
    use cocoa_ui::Retained;
    use cocoa_ui::menu::{Command, KeyModifiers, MenuTreeNode};
    use cocoa_ui::native_test::pump_main_until;
    use cocoa_ui::objc2_ui_kit::{
        UIAction, UIApplication, UIButton, UIContextMenuInteraction, UIKeyCommand,
        UIKeyModifierFlags, UIMenu, UIMenuElement, UIMenuElementAttributes, UIMenuElementState,
        UITextField, UIView, UIViewController, UIWindow,
    };
    use cocoa_ui::uikit::{
        self, KeyCommands, Menu, MenuAction, MenuButton, MenuElement, native_test,
    };
    use libtest_mimic::Trial;
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadMarker, sel};

    /// The trial cases.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "key_commands::a_command_with_a_shortcut_builds_a_key_command",
                || {
                    a_command_with_a_shortcut_builds_a_key_command();
                    Ok(())
                },
            ),
            Trial::test(
                "key_commands::a_command_without_a_shortcut_builds_a_uiaction",
                || {
                    a_command_without_a_shortcut_builds_a_uiaction();
                    Ok(())
                },
            ),
            Trial::test(
                "key_commands::the_menu_components_action_arms_the_shortcut",
                || {
                    the_menu_components_action_arms_the_shortcut();
                    Ok(())
                },
            ),
            Trial::test(
                "key_commands::a_chord_from_a_menu_uikit_copied_fires_the_commands_callback",
                || {
                    a_chord_from_a_menu_uikit_copied_fires_the_commands_callback();
                    Ok(())
                },
            ),
            Trial::test(
                "key_commands::a_rebuild_while_the_menu_is_open_replaces_the_visible_commands",
                || {
                    a_rebuild_while_the_menu_is_open_replaces_the_visible_commands();
                    Ok(())
                },
            ),
            Trial::test(
                "key_commands::unmounting_an_open_menu_closes_it_before_its_commands_retire",
                || {
                    unmounting_an_open_menu_closes_it_before_its_commands_retire();
                    Ok(())
                },
            ),
        ]
    }

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("the harness runs cases on the main thread")
    }

    /// Builds `nodes` into a `UIMenu` and reads back its only child.
    fn single_element(
        key_commands: &KeyCommands,
        nodes: &[MenuTreeNode],
    ) -> Retained<UIMenuElement> {
        let menu = uikit::menu(mtm(), key_commands, &Command::default(), nodes);
        let children = menu.children();
        assert_eq!(children.len(), 1, "the menu holds one element");
        children.objectAtIndex(0)
    }

    /// The menu-bar/context-menu builder arms the chord: the element is a
    /// `UIKeyCommand` whose `input`/`modifierFlags` are the command's
    /// shortcut and whose action routes `cocoaUiMenuCommandFired:` up the
    /// responder chain; the rest of the command's presentation stays
    /// intact.
    fn a_command_with_a_shortcut_builds_a_key_command() {
        let command = Command {
            label: "Share".into(),
            subtitle: Some("sends the selection".into()),
            destructive: true,
            enabled: false,
            selected: true,
            key_equivalent: "s".into(),
            modifiers: KeyModifiers::COMMAND | KeyModifiers::SHIFT,
            ..Command::default()
        };
        let key_commands = KeyCommands::new(mtm());
        let element = single_element(
            &key_commands,
            &[MenuTreeNode::Command(command, Rc::new(|| {}))],
        );
        let key = element
            .downcast_ref::<UIKeyCommand>()
            .expect("a shortcut arms a `UIKeyCommand`, not a `UIAction`");
        assert_eq!(key.input().unwrap().to_string(), "s");
        assert_eq!(
            key.modifierFlags(),
            UIKeyModifierFlags::Command | UIKeyModifierFlags::Shift
        );
        // SAFETY: `action` only reads the selector the element was built with.
        let action = unsafe { key.action() };
        assert_eq!(action, Some(sel!(cocoaUiMenuCommandFired:)));
        assert_eq!(key.title().to_string(), "Share");
        assert_eq!(
            key.subtitle()
                .map(|subtitle| subtitle.to_string())
                .as_deref(),
            Some("sends the selection")
        );
        assert_eq!(key.state(), UIMenuElementState::On);
        assert!(
            key.attributes().contains(UIMenuElementAttributes::Disabled)
                && key
                    .attributes()
                    .contains(UIMenuElementAttributes::Destructive),
            "the command's attributes carry onto the key command"
        );
    }

    /// Without a shortcut the element stays a `UIAction` and its
    /// discoverability title stays unset — a chord is never display text.
    fn a_command_without_a_shortcut_builds_a_uiaction() {
        let command = Command {
            label: "Plain".into(),
            enabled: true,
            ..Command::default()
        };
        let key_commands = KeyCommands::new(mtm());
        let element = single_element(
            &key_commands,
            &[MenuTreeNode::Command(command, Rc::new(|| {}))],
        );
        assert!(
            element.downcast_ref::<UIKeyCommand>().is_none(),
            "a command without a shortcut is not a `UIKeyCommand`"
        );
        let action = element
            .downcast_ref::<UIAction>()
            .expect("a command without a shortcut builds a `UIAction`");
        assert_eq!(action.title().to_string(), "Plain");
        assert!(action.discoverabilityTitle().is_none());
    }

    /// The `Menu` component's element path (`MenuAction`) arms the chord
    /// identically — every `KeyModifiers` flag lands on `modifierFlags`.
    fn the_menu_components_action_arms_the_shortcut() {
        let command = Command {
            label: "Keys".into(),
            enabled: true,
            key_equivalent: "k".into(),
            modifiers: KeyModifiers::COMMAND
                | KeyModifiers::OPTION
                | KeyModifiers::CONTROL
                | KeyModifiers::SHIFT,
            ..Command::default()
        };
        let key_commands = KeyCommands::new(mtm());
        let action = MenuAction::command(mtm(), &key_commands, &command, || {});
        let key = action
            .element()
            .downcast_ref::<UIKeyCommand>()
            .expect("the `Menu` component's element arms the shortcut");
        assert_eq!(key.input().unwrap().to_string(), "k");
        assert_eq!(
            key.modifierFlags(),
            UIKeyModifierFlags::Command
                | UIKeyModifierFlags::Alternate
                | UIKeyModifierFlags::Control
                | UIKeyModifierFlags::Shift
        );

        let plain = MenuAction::command(mtm(), &key_commands, &Command::default(), || {});
        assert!(
            plain.element().downcast_ref::<UIAction>().is_some(),
            "a `Menu` component row without a shortcut stays a `UIAction`"
        );
    }

    /// Hands the menu to a `UIButton` — `UIKit` keeps its own copy — then
    /// sends the copied key command's action the way a chord does.
    fn a_chord_from_a_menu_uikit_copied_fires_the_commands_callback() {
        let mtm = mtm();
        let _scene = ChordScene::new(mtm);
        let fired = Rc::new(Cell::new(false));
        let callback = {
            let fired = fired.clone();
            Rc::new(move || fired.set(true))
        };
        let key_commands = KeyCommands::new(mtm);
        let menu = uikit::menu(
            mtm,
            &key_commands,
            &Command::default(),
            &[MenuTreeNode::Command(shortcut_command("Fire"), callback)],
        );
        let built = menu.children().objectAtIndex(0);
        let button = UIButton::new(mtm);
        button.setMenu(Some(&menu));
        let held = button.menu().expect("the button holds a menu");
        let copied = held.children().objectAtIndex(0);
        assert_ne!(
            Retained::as_ptr(&copied),
            Retained::as_ptr(&built),
            "UIKit holds a copy of the element the builder made"
        );
        drop((menu, built));
        assert!(
            ChordScene::press(mtm, &copied),
            "the responder chain takes the command's action"
        );
        assert!(fired.get(), "the command's callback ran");
    }

    /// A `Menu` whose items change while its menu is open rebuilds the menu
    /// and drops the previous menu's `KeyCommands` once the button holds
    /// the new one. `UIKit` replaces an open menu's rows inside `setMenu:`
    /// itself, before the call returns and so before any later event can
    /// pick from it: the open menu shows the rebuilt command, picking it
    /// runs the rebuilt callback, and no command of the retired scope is
    /// reachable.
    fn a_rebuild_while_the_menu_is_open_replaces_the_visible_commands() {
        let mtm = mtm();
        let scene = ChordScene::new(mtm);
        let fired = Rc::new(Cell::new(""));
        let build = |label: &'static str| {
            let key_commands = KeyCommands::new(mtm);
            let fired = fired.clone();
            let action =
                MenuAction::command(mtm, &key_commands, &shortcut_command(label), move || {
                    fired.set(label);
                });
            (
                Menu::new(mtm, "", None, false, &[MenuElement::Action(action)]),
                key_commands,
            )
        };
        let trigger = MenuButton::new(mtm);
        scene.add(trigger.view());
        cocoa_ui::view::set_frame(trigger.view(), TRIGGER_FRAME);
        let (menu, mut key_commands) = build("Old");
        trigger.set_menu(&menu);
        let interaction = open_menu(
            trigger
                .view()
                .downcast_ref::<UIButton>()
                .expect("a `MenuButton` is a `UIButton`"),
        );
        assert_eq!(visible_titles(&interaction), ["Old"]);

        // The `Menu` component's rebuild: the button takes the new menu,
        // then the previous menu's scope retires.
        let (menu, rebuilt) = build("New");
        trigger.set_menu(&menu);
        drop(std::mem::replace(&mut key_commands, rebuilt));
        let visible = visible_elements(&interaction);
        assert_eq!(
            visible
                .iter()
                .map(|element| element.title().to_string())
                .collect::<Vec<_>>(),
            ["New"],
            "the menu stays open and shows the rebuilt command as soon as `setMenu:` returns"
        );
        assert!(
            ChordScene::press(mtm, &visible[0]),
            "the responder chain takes the visible command's action"
        );
        assert_eq!(fired.get(), "New", "picking runs the rebuilt callback");
        interaction.dismissMenu();
    }

    /// The `Menu` component's teardown: `Mounted` takes the trigger out of
    /// its window before the leaf's state, and with it the leaf's
    /// `KeyCommands`, drops. `UIKit` closes a menu whose presenting view
    /// leaves the window inside `removeFromSuperview` itself — its rows
    /// stop taking touches and it no longer reports a visible menu before
    /// the call returns — so no command the leaf armed can be picked once
    /// the leaf is gone.
    fn unmounting_an_open_menu_closes_it_before_its_commands_retire() {
        use waterui::component::menu::{CommandExt, Menu as MenuView, Shortcut};

        let mtm = mtm();
        let scene = ChordScene::new(mtm);
        let leaf = waterui_apple::native_test_support::render(MenuView::new(
            "Open",
            "Keys".action(|| {}).shortcut(Shortcut::new('k').command()),
        ));
        let mounted = leaf.mount(&scene.root());
        cocoa_ui::view::set_frame(mounted.view(), TRIGGER_FRAME);
        scene.window.layoutIfNeeded();
        let button =
            descendant_button(mounted.view()).expect("the `Menu` leaf's trigger is a `UIButton`");
        let interaction = open_menu(&button);
        assert_eq!(visible_titles(&interaction), ["Keys"]);

        drop(mounted);
        assert!(
            visible_elements(&interaction).is_empty(),
            "the menu closes as its leaf unmounts, before the leaf's commands retire"
        );
    }

    /// Where the cases place a menu trigger.
    const TRIGGER_FRAME: Rect = Rect::new(50.0, 100.0, 200.0, 44.0);

    /// How long a menu may take to open.
    const MENU_DEADLINE: f64 = 5.0;

    /// A command armed with the ⌘K shortcut.
    fn shortcut_command(label: &str) -> Command {
        Command {
            label: label.into(),
            enabled: true,
            key_equivalent: "k".into(),
            modifiers: KeyModifiers::COMMAND,
            ..Command::default()
        }
    }

    /// Opens `button`'s menu the way a tap does and waits until `UIKit`
    /// shows it.
    fn open_menu(button: &UIButton) -> Retained<UIContextMenuInteraction> {
        button.performPrimaryAction();
        let interaction = button
            .contextMenuInteraction()
            .expect("a button with a menu has a context-menu interaction");
        assert!(
            pump_main_until(MENU_DEADLINE, || !visible_elements(&interaction).is_empty()),
            "the button's primary action opens its menu"
        );
        interaction
    }

    /// The first `UIButton` in `view`'s subtree.
    fn descendant_button(view: &UIView) -> Option<Retained<UIButton>> {
        view.subviews().iter().find_map(|child| {
            child
                .clone()
                .downcast::<UIButton>()
                .ok()
                .or_else(|| descendant_button(&child))
        })
    }

    /// The elements of the menu `interaction` shows; none once the menu
    /// is closed or closing.
    fn visible_elements(interaction: &UIContextMenuInteraction) -> Vec<Retained<UIMenuElement>> {
        let elements = RefCell::new(Vec::new());
        let block = StackBlock::new(|menu: NonNull<UIMenu>| {
            // SAFETY: `UIKit` hands the block a live copy of the visible menu.
            elements
                .borrow_mut()
                .extend(unsafe { menu.as_ref() }.children().iter());
            menu
        });
        // SAFETY: the block returns the menu it is given, leaving the
        // visible menu as it is.
        unsafe { interaction.updateVisibleMenuWithBlock(&block) };
        elements.into_inner()
    }

    /// The titles of the elements `interaction` shows.
    fn visible_titles(interaction: &UIContextMenuInteraction) -> Vec<String> {
        visible_elements(interaction)
            .iter()
            .map(|element| element.title().to_string())
            .collect()
    }

    /// A key, visible window whose text field is first responder — the
    /// shape `UIKit` delivers a key event into, where an untargeted action
    /// travels the responder chain to `AppDelegate`.
    struct ChordScene {
        window: Retained<UIWindow>,
        controller: Retained<UIViewController>,
    }

    impl ChordScene {
        fn new(mtm: MainThreadMarker) -> Self {
            let window = native_test::window(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
            let controller = UIViewController::new(mtm);
            window.setRootViewController(Some(&controller));
            window.makeKeyAndVisible();
            let scene = Self { window, controller };
            let field = UITextField::new(mtm);
            scene.add(&field);
            assert!(
                field.becomeFirstResponder(),
                "the scene's responder leads the chain a chord travels"
            );
            scene
        }

        /// The view the scene's content goes in.
        fn root(&self) -> Retained<UIView> {
            self.controller
                .view()
                .expect("the controller's view is loaded")
        }

        fn add(&self, view: &UIView) {
            self.root().addSubview(view);
            self.window.layoutIfNeeded();
        }

        /// Sends `element`'s action untargeted, with `element` as the
        /// sender — the dispatch `UIKit` performs for a pressed chord or a
        /// picked key command — and answers whether a responder took it.
        fn press(mtm: MainThreadMarker, element: &UIMenuElement) -> bool {
            let key = element
                .downcast_ref::<UIKeyCommand>()
                .expect("a shortcut command builds a `UIKeyCommand`");
            // SAFETY: `action` only reads the selector the command carries.
            let action = unsafe { key.action() }.expect("the key command carries an action");
            let sender: &AnyObject = key.as_ref();
            // SAFETY: `action` is the command's own selector and `sender`
            // the command itself.
            unsafe {
                UIApplication::sharedApplication(mtm).sendAction_to_from_forEvent(
                    action,
                    None,
                    Some(sender),
                    None,
                )
            }
        }
    }

    impl Drop for ChordScene {
        fn drop(&mut self) {
            self.window.setHidden(true);
        }
    }
}

/// The shared gesture contract end to end inside the real `UIApplication`
/// scene: a completed gesture inserts the observed `Gesture` and its
/// typed event — `TapEvent`, `DragEvent`, `RotationEvent`,
/// `MagnificationEvent` — into the action's environment, so a `Use<T>`
/// handler extracts the same value Hydrolysis reports for the same input.
/// These cases cover the observer's state/getter mapping, not the OS's
/// gesture recognition: they drive the installed recognizer's documented
/// `setState:`/getter-seeding transitions, which dispatch the target
/// through the same `on_recognized` path a real completion does.
#[cfg(target_os = "ios")]
mod gesture_events {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cocoa_ui::Retained;
    use cocoa_ui::objc2::DowncastTarget;
    use cocoa_ui::objc2_ui_kit::{
        UIGestureRecognizerState, UIPanGestureRecognizer, UIPinchGestureRecognizer,
        UIRotationGestureRecognizer, UITapGestureRecognizer,
    };
    use waterui::ViewExt as _;
    use waterui::extract::Use;
    use waterui::gesture::{DragEvent, DragGesture, GesturePhase};
    use waterui::gesture::{MagnificationEvent, MagnificationGesture};
    use waterui::gesture::{PointerButton, RotationEvent, RotationGesture, TapEvent, TapGesture};
    use waterui::prelude::text;
    use waterui_apple::dispatch;
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, pump_main_until, render_environment,
    };

    use super::{mtm, scene_window};

    /// The `gesture_events::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        let cases: [(&str, fn()); 4] = [
            (
                "a_tap_gesture_delivers_its_event",
                a_tap_gesture_delivers_its_event,
            ),
            (
                "a_drag_gesture_delivers_its_events",
                a_drag_gesture_delivers_its_events,
            ),
            (
                "a_rotation_gesture_reports_native_radians",
                a_rotation_gesture_reports_native_radians,
            ),
            (
                "a_magnification_gesture_honours_the_shared_minimum",
                a_magnification_gesture_honours_the_shared_minimum,
            ),
        ];
        cases
            .into_iter()
            .map(|(name, case)| {
                libtest_mimic::Trial::test(format!("gesture_events::{name}"), move || {
                    case();
                    Ok(())
                })
            })
            .collect()
    }

    /// The host's recognizers — the first of `T` attached — as the
    /// test's handle on the concrete recognizer the observer wired.
    fn recognizer<T: DowncastTarget>(host: &cocoa_ui::PlatformView) -> Retained<T> {
        host.gestureRecognizers()
            .expect("the observer attaches a recognizer")
            .iter()
            .find_map(|recognizer| recognizer.downcast::<T>().ok())
            .expect("the observer attaches a recognizer of the requested kind")
    }

    /// Driving the tap recognizer's `Recognized` transition completes it;
    /// the action's environment carries a `TapEvent` whose count and
    /// button match the configured gesture.
    fn a_tap_gesture_delivers_its_event() {
        let taps = Rc::new(RefCell::new(Vec::new()));
        let leaf = dispatch::render(
            waterui::AnyView::new({
                let taps = Rc::clone(&taps);
                text("tap").gesture(TapGesture::new(), move |Use(event): Use<TapEvent>| {
                    taps.borrow_mut().push(event);
                })
            }),
            &render_environment(),
        );
        let host = leaf.view();
        let _window = scene_window(mtm(), host);
        let recognizer = recognizer::<UITapGestureRecognizer>(host);
        let before = taps.borrow().len();
        recognizer.setState(UIGestureRecognizerState::Recognized);
        pump_main_until(MAIN_QUEUE_DEADLINE, || taps.borrow().len() > before);
        let taps = taps.borrow();
        assert_eq!(taps.len(), before + 1, "one completion appends one event");
        let tap = &taps[before];
        assert_eq!(tap.count, 1);
        assert_eq!(tap.button, PointerButton::Primary);
    }

    /// Driving the pan recognizer's `Began`→`Changed`→`Ended` transitions
    /// delivers the phase sequence the shared contract names — the
    /// events the same input produces on Hydrolysis.
    fn a_drag_gesture_delivers_its_events() {
        let drags = Rc::new(RefCell::new(Vec::new()));
        let leaf = dispatch::render(
            waterui::AnyView::new({
                let drags = Rc::clone(&drags);
                text("drag").gesture(DragGesture::new(0.0), move |Use(e): Use<DragEvent>| {
                    drags.borrow_mut().push(e);
                })
            }),
            &render_environment(),
        );
        let host = leaf.view();
        let _window = scene_window(mtm(), host);
        let recognizer = recognizer::<UIPanGestureRecognizer>(host);
        for state in [
            UIGestureRecognizerState::Began,
            UIGestureRecognizerState::Changed,
            UIGestureRecognizerState::Ended,
        ] {
            let before = drags.borrow().len();
            recognizer.setState(state);
            pump_main_until(MAIN_QUEUE_DEADLINE, || drags.borrow().len() > before);
            assert_eq!(
                drags.borrow().len(),
                before + 1,
                "one transition appends one event"
            );
        }
        let drags = drags.borrow();
        assert_eq!(
            drags.iter().map(|e| e.phase).collect::<Vec<_>>(),
            vec![
                GesturePhase::Started,
                GesturePhase::Updated,
                GesturePhase::Ended
            ],
            "the pan's transitions are the contract's phase sequence"
        );
        assert!(
            drags.iter().all(|e| e.button == PointerButton::Primary),
            "a touches contact presses as the primary button"
        );
    }

    /// `UIRotationGestureRecognizer.rotation` is already radians — the
    /// event's angle is the raw getter value, phases map the state
    /// transitions, and `Ended` carries zero velocity (#2757).
    fn a_rotation_gesture_reports_native_radians() {
        let rotations = Rc::new(RefCell::new(Vec::new()));
        let leaf = dispatch::render(
            waterui::AnyView::new({
                let rotations = Rc::clone(&rotations);
                text("rotate").gesture(
                    RotationGesture::new(0.0),
                    move |Use(e): Use<RotationEvent>| {
                        rotations.borrow_mut().push(e);
                    },
                )
            }),
            &render_environment(),
        );
        let host = leaf.view();
        let _window = scene_window(mtm(), host);
        let recognizer = recognizer::<UIRotationGestureRecognizer>(host);
        let fire = |state, angle: f64| {
            recognizer.setRotation(angle);
            let before = rotations.borrow().len();
            recognizer.setState(state);
            pump_main_until(MAIN_QUEUE_DEADLINE, || rotations.borrow().len() > before);
            let rotations = rotations.borrow();
            assert_eq!(
                rotations.len(),
                before + 1,
                "one callback appends one event"
            );
            rotations[before].clone()
        };

        let e = fire(UIGestureRecognizerState::Began, 0.0);
        assert_eq!(
            (e.phase, e.angle, e.velocity),
            (GesturePhase::Started, 0.0, 0.0)
        );
        let e = fire(UIGestureRecognizerState::Changed, 0.75);
        assert_eq!(e.phase, GesturePhase::Updated);
        assert!(
            (e.angle - 0.75).abs() < 1e-6,
            "raw radians getter, saw {}",
            e.angle
        );
        let e = fire(UIGestureRecognizerState::Changed, 1.25);
        assert!((e.angle - 1.25).abs() < 1e-6);
        let e = fire(UIGestureRecognizerState::Ended, 1.25);
        assert_eq!((e.phase, e.velocity), (GesturePhase::Ended, 0.0));
        assert!((e.angle - 1.25).abs() < 1e-6);
    }

    /// The shared scale composition clamps at the contract's `.01`
    /// minimum each update — an `initial_scale` below it and a relative
    /// delta both stay inside the floor (#2757).
    fn a_magnification_gesture_honours_the_shared_minimum() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let leaf = dispatch::render(
            waterui::AnyView::new({
                let events = Rc::clone(&events);
                text("pinch").gesture(
                    MagnificationGesture::new(0.001),
                    move |Use(e): Use<MagnificationEvent>| {
                        events.borrow_mut().push(e);
                    },
                )
            }),
            &render_environment(),
        );
        let host = leaf.view();
        let _window = scene_window(mtm(), host);
        let recognizer = recognizer::<UIPinchGestureRecognizer>(host);
        let fire = |state, scale: f64| {
            recognizer.setScale(scale);
            let before = events.borrow().len();
            recognizer.setState(state);
            pump_main_until(MAIN_QUEUE_DEADLINE, || events.borrow().len() > before);
            let events = events.borrow();
            assert_eq!(events.len(), before + 1, "one callback appends one event");
            events[before].clone()
        };

        let e = fire(UIGestureRecognizerState::Began, 1.0);
        assert_eq!((e.phase, e.velocity), (GesturePhase::Started, 0.0));
        assert!((e.scale - 0.001).abs() < 1e-6, "saw {}", e.scale);
        let e = fire(UIGestureRecognizerState::Changed, 1.5);
        assert!(
            (e.scale - 0.01).abs() < 1e-6,
            "clamp to .01, saw {}",
            e.scale
        );
        let e = fire(UIGestureRecognizerState::Changed, 3.0);
        assert!(
            (e.scale - 0.02).abs() < 1e-6,
            "factor 2 on .01, saw {}",
            e.scale
        );
        let e = fire(UIGestureRecognizerState::Changed, 3.0);
        assert!((e.scale - 0.02).abs() < 1e-6 && e.velocity == 0.0);
        let e = fire(UIGestureRecognizerState::Ended, 3.0);
        assert_eq!((e.phase, e.velocity), (GesturePhase::Ended, 0.0));
        assert!((e.scale - 0.02).abs() < 1e-6);
    }
}

#[cfg(all(target_os = "ios", feature = "native-test", feature = "text_field"))]
mod text_selection_ios {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cocoa_ui::PlatformView;
    use cocoa_ui::objc2::runtime::AnyObject;
    use cocoa_ui::objc2::{DowncastTarget, sel};
    use cocoa_ui::objc2::{Message, msg_send};
    use cocoa_ui::objc2_foundation::{NSArray, NSRange, NSString, NSValue};
    use cocoa_ui::objc2_ui_kit::{
        UIAction, UIApplication, UIKeyCommand, UIMenu, UIMenuElement, UIMenuElementAttributes,
    };
    use cocoa_ui::{Retained, view};
    use waterui::Str;
    use waterui::component::menu::{Command, MenuItem, Shortcut};
    use waterui::component::text_field::TextField;
    use waterui::component::text_selection::TextSelection;
    use waterui::reactive::{Signal, SignalExt, binding};
    use waterui::text::StyledStr;

    use cocoa_ui::geometry::Rect;
    use cocoa_ui::native_test::pump_main_until;
    use cocoa_ui::objc2::MainThreadMarker;
    use cocoa_ui::uikit::native_test;
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, mount_uikit_window, render_environment,
    };

    use crate::mtm;

    fn find<T: DowncastTarget>(view: &PlatformView) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.downcast_ref::<T>().is_some() {
                return Some(sub);
            }
            if let Some(found) = find::<T>(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// The titles of a `UIMenu`'s flattened leaf actions.
    fn titles(menu: &UIMenu) -> Vec<String> {
        fn walk(out: &mut Vec<String>, el: &UIMenuElement) {
            if let Some(action) = el.downcast_ref::<UIAction>() {
                out.push(action.title().to_string());
            } else if let Some(sub) = el.downcast_ref::<UIMenu>() {
                for child in &sub.children() {
                    walk(out, &child);
                }
            }
        }
        let mut out = Vec::new();
        for el in &menu.children() {
            walk(&mut out, &el);
        }
        out
    }

    /// Invokes the menu action titled `title` through the public
    /// `UIControl.sendAction(_:)` API — the generated `UIAction` runs its
    /// handler as if the row had been picked.
    fn perform(field: &cocoa_ui::uikit::TextField, menu: &UIMenu, title: &str) {
        fn find_action(el: &UIMenuElement, title: &str) -> Option<Retained<UIAction>> {
            if let Some(action) = el.downcast_ref::<UIAction>() {
                if action.title().to_string() == title {
                    return Some(action.retain());
                }
            } else if let Some(sub) = el.downcast_ref::<UIMenu>() {
                for child in &sub.children() {
                    if let Some(a) = find_action(&child, title) {
                        return Some(a);
                    }
                }
            }
            None
        }
        let action = (0..menu.children().count())
            .find_map(|i| {
                menu.children()
                    .objectAtIndex(i)
                    .downcast::<UIMenuElement>()
                    .ok()
                    .and_then(|el| find_action(&el, title))
            })
            .unwrap_or_else(|| panic!("no menu action titled {title}"));
        field.sendAction(&action);
    }

    /// The field's edit menu — the delegate method invoked directly with
    /// empty suggestions.
    fn edit_menu(field: &cocoa_ui::uikit::TextField) -> Retained<UIMenu> {
        let suggested: Retained<NSArray<UIMenuElement>> = NSArray::new();
        let ranges: Retained<NSArray<NSValue>> = NSArray::new();
        // SAFETY: the plural `editMenuForCharactersInRanges:` selector is
        // `UITextFieldDelegate`'s declared signature.
        let menu: Option<Retained<UIMenu>> = unsafe {
            msg_send![&**field, textField: &**field, editMenuForCharactersInRanges: &*ranges, suggestedActions: &*suggested]
        };
        menu.expect("the field produced an edit menu")
    }

    /// A `selection_menu` command reads the live selection and edits it
    /// through `replaceRange:withText:` — the field's text and the value
    /// binding both move.
    pub fn a_selection_menu_command_edits_the_live_selection() {
        let mtm = mtm();
        let value = binding(Str::from("hé😀b"));
        let observed = Rc::new(RefCell::new((String::new(), 0usize, 0usize)));
        let optional_seen = Rc::new(RefCell::new(false));

        let observe = Command::builder("Observe").action({
            let observed = Rc::clone(&observed);
            move |selection: TextSelection| {
                let range = selection.range();
                *observed.borrow_mut() = (selection.text().to_string(), range.start, range.end);
            }
        });
        let optional = Command::builder("Optional").action({
            let optional_seen = Rc::clone(&optional_seen);
            move |selection: Option<TextSelection>| {
                *optional_seen.borrow_mut() = selection.is_some();
            }
        });
        let mark =
            Command::builder("Mark").action(|selection: TextSelection| selection.select(3..7));
        let wrap_fired: Rc<RefCell<Option<std::ops::Range<usize>>>> = Rc::new(RefCell::new(None));
        let wrap = Command::builder("Wrap").action({
            let wrap_fired = Rc::clone(&wrap_fired);
            move |selection: TextSelection| {
                *wrap_fired.borrow_mut() = Some(selection.range());
                selection.replace("《");
            }
        });
        let styled = Command::builder("Styled").action(|selection: TextSelection| {
            selection
                .styled_value()
                .expect("a plain field answers styled_value")
                .set(StyledStr::plain("ZZ"));
        });
        let brk = Command::builder("Break").action(|selection: TextSelection| {
            selection.replace("M\r\nN");
        });
        let off = Command::builder("Off").action(|| {}).disabled(true);
        let chord_seen = Rc::new(RefCell::new(None));
        let chord = Command::builder("Chord")
            .action({
                let chord_seen = Rc::clone(&chord_seen);
                move |selection: TextSelection| {
                    *chord_seen.borrow_mut() = Some(selection.range());
                }
            })
            .shortcut(Shortcut::new('x').command());

        let items = binding(vec![
            MenuItem::Command(observe),
            MenuItem::Command(optional),
            MenuItem::Command(mark),
            MenuItem::Command(wrap),
            MenuItem::Command(styled),
            MenuItem::Command(brk),
            MenuItem::Command(off),
            MenuItem::Command(chord),
        ]);
        let window = native_test::window(mtm, Rect::new(0.0, 0.0, 400.0, 300.0));
        let mount = mount_uikit_window(
            mtm,
            waterui::AnyView::new(TextField::new("Name", &value).selection_menu(items.computed())),
            &render_environment(),
            window,
        );
        let field_view = find::<cocoa_ui::uikit::TextField>(&mount.host)
            .expect("the leaf mounts a kit TextField");
        let field = field_view
            .downcast::<cocoa_ui::uikit::TextField>()
            .expect("the found view is the kit TextField");

        assert!(field.begin_editing(), "the field never began editing");

        // UTF-16 units: h=1 é=1 😀=2 b=1 — 😀 spans units 2..4.
        field.set_selected_range(NSRange::new(2, 2));
        let menu = edit_menu(&field);
        let built_titles = titles(&menu);
        for title in [
            "Observe", "Optional", "Mark", "Wrap", "Styled", "Break", "Off",
        ] {
            assert!(
                built_titles.contains(&title.to_owned()),
                "the {title} row is missing"
            );
        }

        field.set_selected_range(NSRange::new(0, 2));
        perform(&field, &menu, "Observe");
        let (text, start, end) = observed.borrow().clone();
        assert_eq!((text.as_str(), start, end), ("hé😀b", 0, 3));

        perform(&field, &menu, "Optional");
        assert!(*optional_seen.borrow());

        perform(&field, &menu, "Mark");
        assert_eq!(field.selected_range(), Some(NSRange::new(2, 2)));

        assert!(field.isEditing(), "the field is not editing");

        wrap_undo_and_single_line(&field, &menu, &value, &wrap_fired);

        perform(&field, &menu, "Styled");
        assert_eq!(value.snapshot(), Str::from("ZZ"));
        assert_eq!(field.string(), "ZZ");

        edit_menu_routes_and_retires(mtm, &field, &menu, &items, &chord_seen);
    }

    /// The `Wrap` edit's live range, caret, undo/redo roundtrip and the
    /// single-line newline invariant — kept out of the trial so each
    /// section reads as one step.
    fn wrap_undo_and_single_line(
        field: &cocoa_ui::uikit::TextField,
        menu: &UIMenu,
        value: &waterui::Binding<Str>,
        wrap_fired: &Rc<RefCell<Option<std::ops::Range<usize>>>>,
    ) {
        perform(field, menu, "Wrap");
        assert_eq!(
            *wrap_fired.borrow(),
            Some(3..7),
            "Wrap fired with the live byte range"
        );
        assert_eq!(field.string(), "hé《b");
        assert_eq!(value.snapshot(), Str::from("hé《b"));
        assert_eq!(
            field.selected_range(),
            Some(NSRange::new(3, 0)),
            "the caret landed after the replacement"
        );

        // The field's own undo manager rewinds the menu edit — undo
        // restores the original text on both the field and the binding,
        // redo brings the replacement back.
        let undo = field
            .undoManager()
            .expect("an editing field carries an undo manager");
        pump_main_until(MAIN_QUEUE_DEADLINE, || undo.canUndo());
        assert!(undo.canUndo(), "the native edit registered no undo");
        undo.undo();
        pump_main_until(MAIN_QUEUE_DEADLINE, || field.string() == "hé😀b");
        assert_eq!(field.string(), "hé😀b");
        assert_eq!(value.snapshot(), Str::from("hé😀b"));
        pump_main_until(MAIN_QUEUE_DEADLINE, || undo.canRedo());
        assert!(undo.canRedo(), "the native undo registered no redo");
        undo.redo();
        pump_main_until(MAIN_QUEUE_DEADLINE, || field.string() == "hé《b");
        assert_eq!(field.string(), "hé《b");
        assert_eq!(value.snapshot(), Str::from("hé《b"));

        // A single-line field accepts no line breaks through the native
        // edit path — an LF or a CRLF in the replacement normalizes away.
        perform(field, menu, "Break");
        let normalized = field.string();
        assert_eq!(
            value.snapshot(),
            Str::from(normalized.clone()),
            "the binding diverged from the normalized field text"
        );
        assert!(
            !normalized.contains(['\n', '\r']),
            "a line break survived the replacement"
        );
    }

    /// The `UIKeyCommand` route, the disabled row, suggested-action
    /// preservation and scope retirement — split out of the trial so the
    /// mounted-field assertions stay readable.
    fn edit_menu_routes_and_retires(
        mtm: MainThreadMarker,
        field: &cocoa_ui::uikit::TextField,
        menu: &UIMenu,
        items: &waterui::Binding<Vec<MenuItem>>,
        chord_seen: &Rc<RefCell<Option<std::ops::Range<usize>>>>,
    ) {
        // A shortcut row is a native `UIKeyCommand` armed on the field's
        // own registry; firing its declared action through the public
        // `sendAction:to:from:forEvent:` route lands on the field's own
        // registry — the application registry is never consulted.
        field.set_selected_range(NSRange::new(1, 1));
        let chord = find_key_command(menu, "Chord");
        let app = UIApplication::sharedApplication(mtm);
        // SAFETY: the selector is the field's armed-command route; the
        // target is the first-responder field, the sender the armed
        // `UIKeyCommand`, and `None` is a valid event.
        let delivered = unsafe {
            app.sendAction_to_from_forEvent(
                sel!(cocoaUiTextSelectionCommandFired:),
                Some(AsRef::<AnyObject>::as_ref(field)),
                Some(AsRef::<AnyObject>::as_ref(&*chord)),
                None,
            )
        };
        assert!(delivered, "the field declined its own command route");
        assert_eq!(
            chord_seen.borrow().as_ref().map(|r| (r.start, r.end)),
            Some((1, 2)),
            "the chord fired the live selection's handler"
        );

        // A disabled command renders a disabled native row.
        let off = find_element(menu, "Off")
            .downcast::<UIAction>()
            .expect("the Off row is a UIAction");
        assert!(
            off.attributes().contains(UIMenuElementAttributes::Disabled),
            "a disabled command is not disabled"
        );

        // A suggested system element survives beside the custom rows.
        // SAFETY: `actionWithTitle:...` is `UIAction`'s declared
        // constructor; the handler is a valid `RcBlock` it copies.
        let suggested_action = unsafe {
            UIAction::actionWithTitle_image_identifier_handler(
                &NSString::from_str("Suggested"),
                None,
                None,
                block2::RcBlock::as_ptr(&block2::RcBlock::new(
                    |_action: core::ptr::NonNull<UIAction>| {},
                ))
                .cast(),
                mtm,
            )
        };
        let suggested: Retained<NSArray<UIMenuElement>> =
            NSArray::from_retained_slice(&[suggested_action.into_super()]);
        let ranges: Retained<NSArray<NSValue>> = NSArray::new();
        // SAFETY: the plural delegate signature, as `edit_menu` uses.
        let with_suggested: Option<Retained<UIMenu>> = unsafe {
            msg_send![field, textField: field, editMenuForCharactersInRanges: &*ranges, suggestedActions: &*suggested]
        };
        let with_suggested = with_suggested.expect("the field produced an edit menu");
        assert!(
            titles(&with_suggested).contains(&"Suggested".to_owned()),
            "the suggested action was dropped"
        );

        // Emptying the items retires the armed scope outright — nothing
        // stays registered for a menu that is no longer presented.
        items.set(vec![]);
        // SAFETY: the plural delegate signature, as `edit_menu` uses.
        let empty: Option<Retained<UIMenu>> = unsafe {
            msg_send![field, textField: field, editMenuForCharactersInRanges: &*ranges, suggestedActions: &*suggested]
        };
        assert!(empty.is_none(), "an empty menu still presented rows");
        assert_eq!(
            field.armed_command_count(),
            0,
            "the retired menu's commands were left armed"
        );
    }

    /// Two fields each arm their own `KeyCommandRegistry`: a command from
    /// `A`'s menu is declined by `B` and accepted by `A`, and retiring `B`'s
    /// scope by emptying its items makes `B` decline its own command — a
    /// `UIKeyCommand` delivered to the wrong responder can never invoke a
    /// foreign action.
    pub fn two_fields_own_their_commands_and_retire_them() {
        let mtm = mtm();
        let items_a = binding(vec![MenuItem::Command(
            Command::builder("ChordA")
                .action(|_: TextSelection| {})
                .shortcut(Shortcut::new('a').command()),
        )]);
        let items_b = binding(vec![MenuItem::Command(
            Command::builder("ChordB")
                .action(|_: TextSelection| {})
                .shortcut(Shortcut::new('b').command()),
        )]);

        let mount_field = |items: &waterui::Binding<Vec<MenuItem>>, placeholder| {
            let window = native_test::window(mtm, Rect::new(0.0, 0.0, 400.0, 300.0));
            let mount = mount_uikit_window(
                mtm,
                waterui::AnyView::new(
                    TextField::new(placeholder, &binding(Str::from("t")))
                        .selection_menu(items.computed()),
                ),
                &render_environment(),
                window,
            );
            let field = find::<cocoa_ui::uikit::TextField>(&mount.host)
                .expect("the leaf mounts a kit TextField")
                .downcast::<cocoa_ui::uikit::TextField>()
                .expect("the found view is the kit TextField");
            (mount, field)
        };
        let (_mount_a, field_a) = mount_field(&items_a, "A");
        let (_mount_b, field_b) = mount_field(&items_b, "B");

        let menu_a = edit_menu(&field_a);
        let menu_b = edit_menu(&field_b);
        let chord_a = find_key_command(&menu_a, "ChordA");
        let chord_b = find_key_command(&menu_b, "ChordB");
        let action = sel!(cocoaUiTextSelectionCommandFired:);
        let can = |field: &cocoa_ui::uikit::TextField, command: &UIKeyCommand| -> bool {
            // SAFETY: `canPerformAction:withSender:` is `UIResponder`'s
            // declared signature; the sender is a `UICommand`.
            unsafe {
                msg_send![field, canPerformAction: action, withSender: AsRef::<AnyObject>::as_ref(command)]
            }
        };
        assert!(can(&field_a, &chord_a), "A declined its own command");
        assert!(can(&field_b, &chord_b), "B declined its own command");
        assert!(
            !can(&field_b, &chord_a),
            "B accepted a foreign field's command"
        );
        assert!(
            !can(&field_a, &chord_b),
            "A accepted a foreign field's command"
        );

        // Emptying B's items retires its scope — B then declines even its
        // own command.
        items_b.set(vec![]);
        // SAFETY: the plural delegate signature, as `edit_menu` uses.
        let retired: Option<Retained<UIMenu>> = unsafe {
            msg_send![&*field_b, textField: &*field_b, editMenuForCharactersInRanges: &*NSArray::<NSValue>::new(), suggestedActions: &*NSArray::<UIMenuElement>::new()]
        };
        assert!(retired.is_none(), "an empty menu still presented rows");
        assert!(
            !can(&field_b, &chord_b),
            "B still accepted a retired command"
        );
        assert_eq!(
            field_b.armed_command_count(),
            0,
            "B's retired commands stayed armed"
        );
    }

    /// The `UIMenuElement` titled `title` anywhere in `menu`.
    fn find_element(menu: &UIMenu, title: &str) -> Retained<UIMenuElement> {
        fn walk(el: &UIMenuElement, title: &str) -> Option<Retained<UIMenuElement>> {
            if let Some(action) = el.downcast_ref::<UIAction>() {
                if action.title().to_string() == title {
                    return Some(el.retain());
                }
            } else if let Some(command) = el.downcast_ref::<UIKeyCommand>() {
                if command.title().to_string() == title {
                    return Some(el.retain());
                }
            } else if let Some(sub) = el.downcast_ref::<UIMenu>() {
                for child in &sub.children() {
                    if let Some(found) = walk(&child, title) {
                        return Some(found);
                    }
                }
            }
            None
        }
        (0..menu.children().count())
            .find_map(|i| {
                menu.children()
                    .objectAtIndex(i)
                    .downcast::<UIMenuElement>()
                    .ok()
                    .and_then(|el| walk(&el, title))
            })
            .unwrap_or_else(|| panic!("no menu element titled {title}"))
    }

    /// The `UIKeyCommand` titled `title` anywhere in `menu`.
    fn find_key_command(menu: &UIMenu, title: &str) -> Retained<UIKeyCommand> {
        find_element(menu, title)
            .downcast::<UIKeyCommand>()
            .unwrap_or_else(|_| panic!("the {title} row is not a UIKeyCommand"))
    }

    /// The `text_selection_ios::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "text_selection_ios::two_fields_own_their_commands_and_retire_them",
                || {
                    two_fields_own_their_commands_and_retire_them();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "text_selection_ios::a_selection_menu_command_edits_the_live_selection",
                || {
                    a_selection_menu_command_edits_the_live_selection();
                    Ok(())
                },
            ),
        ]
    }
}
