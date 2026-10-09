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

/// Every `native_app` case: trials that need a running `UIApplication`.
#[cfg(target_os = "ios")]
fn trials() -> Vec<libtest_mimic::Trial> {
    let mut trials = scroll_animation::trials();
    trials.extend(key_commands::trials());
    trials
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
    use cocoa_ui::objc2_ui_kit::UIWindow;
    use cocoa_ui::uikit::{ScrollView, native_test};
    use cocoa_ui::{MainThreadMarker, PlatformView, Rect, Retained, view};
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

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("native_app cases run on the main thread")
    }

    /// A key, visible window in the application's scene with `content`
    /// filling it, laid out.
    fn scene_window(mtm: MainThreadMarker, content: &PlatformView) -> Retained<UIWindow> {
        let window = native_test::window(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
        window.addSubview(content);
        view::set_frame(content, view::bounds(&window));
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        window
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
