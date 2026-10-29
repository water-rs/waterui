//! Native tests that need a running `UIApplication`.
//!
//! `UIKit` drives part of its machinery — the animation behind
//! `UIScrollView.setContentOffset(_:animated: true)` among it — only from
//! the application's update cycle, which the bare `simctl spawn`ed `native`
//! suite never reaches. The `water test-runner` target runner launches
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
    // The precondition a GPU trial relies on: an awake display — a
    // sleeping one presents nothing (#3529). `--list` mounts nothing and
    // skips it.
    #[cfg(target_os = "macos")]
    let _display_awake = (!arguments.list).then(cocoa_ui::native_test::DisplayAwake::hold);
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

#[cfg(target_os = "ios")]
fn find<T: cocoa_ui::objc2::DowncastTarget>(
    view: &cocoa_ui::PlatformView,
) -> Option<cocoa_ui::Retained<cocoa_ui::PlatformView>> {
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

/// Every `native_app` case: trials that need a running `UIApplication`.
#[cfg(target_os = "ios")]
fn trials() -> Vec<libtest_mimic::Trial> {
    let mut trials = scroll_animation::trials();
    trials.extend(main_menu::trials());
    trials.extend(key_commands::trials());
    trials.extend(embedded_menus::trials());
    trials.extend(gesture_events::trials());
    trials.extend(navigation::trials());
    trials.extend(list_spacing::trials());
    trials.extend(capture_regions::trials());
    trials.extend(text_selection_ios::trials());
    trials
}

#[cfg(target_os = "ios")]
mod embedded_menus {
    use std::cell::Cell;
    use std::rc::Rc;

    use cocoa_ui::objc2::{Message, msg_send, sel};
    use cocoa_ui::objc2_core_foundation::CGPoint;
    use cocoa_ui::objc2_foundation::NSCopying;
    use cocoa_ui::objc2_ui_kit::{
        UIApplication, UIButtonType, UIContextMenuConfiguration, UIContextMenuInteraction,
        UIKeyCommand, UIMenu, UIResponder, UIView,
    };
    use cocoa_ui::uikit::{CocoaUiButton, HostView, KeyCommands};
    use cocoa_ui::{Retained, view};
    use libtest_mimic::Trial;
    use waterui::component::menu::{Command, Menu, MenuItem, Shortcut};
    use waterui::reactive::{SignalExt, binding};
    use waterui::{AnyView, ViewExt};
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, mount_uikit_window, render_environment,
    };

    use crate::{find, mtm, native_test};

    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "embedded_menus::menu_routes_and_retires_on_its_button",
                || {
                    menu_routes_and_retires_on_its_button();
                    Ok(())
                },
            ),
            Trial::test(
                "embedded_menus::context_menu_routes_and_retires_on_its_host",
                || {
                    context_menu_routes_and_retires_on_its_host();
                    Ok(())
                },
            ),
        ]
    }

    fn command(fired: &Rc<Cell<usize>>) -> MenuItem {
        let fired = Rc::clone(fired);
        MenuItem::Command(
            Command::builder("Fire")
                .action(move || fired.set(fired.get() + 1))
                .shortcut(Shortcut::new('k').command()),
        )
    }

    fn commands(owner: &UIResponder) -> Vec<Retained<UIKeyCommand>> {
        owner
            .keyCommands()
            .map_or_else(Vec::new, |commands| commands.to_vec())
    }

    fn can(owner: &UIResponder, command: &UIKeyCommand) -> bool {
        can_with_sender(owner, command, Some(command.as_ref()))
    }

    fn can_with_sender(
        owner: &UIResponder,
        command: &UIKeyCommand,
        sender: Option<&objc2::runtime::AnyObject>,
    ) -> bool {
        // SAFETY: this is the responder's declared query signature.
        unsafe {
            owner.canPerformAction_withSender(
                command.action().expect("an armed command has an action"),
                sender,
            )
        }
    }

    fn fire(owner: &UIResponder, command: &UIKeyCommand) {
        assert!(can(owner, command));
        assert!(can_with_sender(owner, command, None));
        // SAFETY: the command supplies the action, sender and owning responder.
        unsafe {
            let action = command.action().expect("an armed command has an action");
            let target = owner
                .targetForAction_withSender(action, Some(command.as_ref()))
                .expect("the owner resolves its command");
            assert!(std::ptr::eq::<objc2::runtime::AnyObject>(
                &raw const *target,
                owner.as_ref()
            ));
            assert!(
                UIApplication::sharedApplication(mtm()).sendAction_to_from_forEvent(
                    action,
                    Some(&target),
                    Some(command.as_ref()),
                    None,
                )
            );
        }
    }

    fn foreign_menu(scope: &KeyCommands) -> Retained<UIMenu> {
        cocoa_ui::uikit::menu(
            mtm(),
            scope,
            &cocoa_ui::menu::Command::default(),
            &[cocoa_ui::menu::MenuTreeNode::Command(
                cocoa_ui::menu::Command {
                    key_equivalent: "f".into(),
                    ..Default::default()
                },
                Rc::new(|| {}),
            )],
        )
    }

    fn menu_routes_and_retires_on_its_button() {
        let mtm = mtm();
        let fired = Rc::new(Cell::new(0));
        let items = binding(vec![command(&fired)]);
        let mount = mount_uikit_window(
            mtm,
            AnyView::new(Menu::new("More", items.computed())),
            &render_environment(),
            native_test::window(mtm, cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0)),
        );
        let button = find::<CocoaUiButton>(&mount.host)
            .expect("Menu mounts a kit button")
            .downcast::<CocoaUiButton>()
            .expect("the found view is a kit button");
        assert_eq!(button.armed_command_count(), 1);
        let key = commands(&button)
            .pop()
            .expect("the button vends its shortcut");
        fire(&button, &key);
        assert_eq!(fired.get(), 1);
        // A copied command keeps its property-list token and still routes
        // to this registry.
        let copied = key.copy();
        fire(&button, &copied);
        assert_eq!(fired.get(), 2);
        for foreign in [
            CocoaUiButton::new(mtm),
            CocoaUiButton::with_type(mtm, UIButtonType::System),
        ] {
            assert_eq!(foreign.armed_command_count(), 0);
            assert!(!can_with_sender(&foreign, &key, None));
            let scope = foreign.menu_commands();
            let _menu = foreign_menu(&scope);
            assert_eq!(foreign.armed_command_count(), 1);
            assert!(!can(&foreign, &key));
            assert!(can_with_sender(&foreign, &key, None));
        }
        items.set(vec![command(&fired)]);
        cocoa_ui::uikit::native_test::pump_main_until(MAIN_QUEUE_DEADLINE, || !can(&button, &key));
        assert!(!can(&button, &key));
        assert_eq!(button.armed_command_count(), 1);
        let rebuilt = commands(&button)
            .pop()
            .expect("the rebuilt shortcut is vended");
        fire(&button, &rebuilt);
        assert_eq!(fired.get(), 3);
        items.set(vec![]);
        cocoa_ui::uikit::native_test::pump_main_until(MAIN_QUEUE_DEADLINE, || {
            button.armed_command_count() == 0
        });
        assert_eq!(button.armed_command_count(), 0);
        assert!(!can(&button, &rebuilt));
        assert!(!can_with_sender(&button, &rebuilt, None));
        assert_eq!(commands(&button), [] as [Retained<UIKeyCommand>; 0]);
        // The mount owns the widget tree: dropping it retires the scope
        // even while the button and its last command stay retained.
        items.set(vec![command(&fired)]);
        cocoa_ui::uikit::native_test::pump_main_until(MAIN_QUEUE_DEADLINE, || {
            button.armed_command_count() == 1
        });
        let live = commands(&button)
            .pop()
            .expect("the repopulated shortcut is vended");
        drop(mount);
        cocoa_ui::uikit::native_test::pump_main_until(MAIN_QUEUE_DEADLINE, || {
            button.armed_command_count() == 0
        });
        assert_eq!(button.armed_command_count(), 0);
        assert!(!can(&button, &live));
        assert!(
            commands(&button).iter().all(|command| {
                // SAFETY: `UIKeyCommand.action`'s declared getter.
                (unsafe { command.action() }) != Some(sel!(cocoaUiButtonMenuCommandFired:))
            }),
            "a retired private command is still vended"
        );
    }

    fn context_host(
        view: &UIView,
    ) -> Option<(Retained<HostView>, Retained<UIContextMenuInteraction>)> {
        if let Some(host) = view.downcast_ref::<HostView>()
            && let Some(interaction) = host
                .interactions()
                .iter()
                .find_map(|interaction| interaction.downcast::<UIContextMenuInteraction>().ok())
        {
            return Some((host.retain(), interaction));
        }
        view::subviews(view)
            .iter()
            .find_map(|child| context_host(child))
    }

    fn configure(
        interaction: &UIContextMenuInteraction,
    ) -> Option<Retained<UIContextMenuConfiguration>> {
        let delegate = interaction
            .delegate()
            .expect("the installed interaction has a delegate");
        // SAFETY: the installed delegate implements this UIKit protocol method.
        unsafe {
            msg_send![&*delegate, contextMenuInteraction: interaction, configurationForMenuAtLocation: CGPoint::ZERO]
        }
    }

    fn context_menu_routes_and_retires_on_its_host() {
        let mtm = mtm();
        let fired = Rc::new(Cell::new(0));
        let items = binding(vec![command(&fired)]);
        let mount = mount_uikit_window(
            mtm,
            AnyView::new(waterui::text::Text::new("Context").context_menu(items.computed())),
            &render_environment(),
            native_test::window(mtm, cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0)),
        );
        let (host, interaction) =
            context_host(&mount.host).expect("ContextMenu installs an interaction on a host");
        assert_eq!(host.armed_command_count(), 0);
        let _configuration = configure(&interaction).expect("the installed handler builds a menu");
        assert_eq!(host.armed_command_count(), 1);
        let key = commands(&host).pop().expect("the host vends its shortcut");
        fire(&host, &key);
        assert_eq!(fired.get(), 1);
        let copied = key.copy();
        fire(&host, &copied);
        assert_eq!(fired.get(), 2);
        let foreign = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        assert!(!can(&foreign, &key));
        assert!(!can_with_sender(&foreign, &key, None));
        let scope = foreign.menu_commands();
        let _menu = foreign_menu(&scope);
        assert_eq!(foreign.armed_command_count(), 1);
        assert!(!can(&foreign, &key));
        assert!(can_with_sender(&foreign, &key, None));
        let _rebuilt = configure(&interaction).expect("a new presentation rebuilds the menu");
        assert!(!can(&host, &key));
        assert_eq!(host.armed_command_count(), 1);
        let rebuilt = commands(&host)
            .pop()
            .expect("the rebuilt shortcut is vended");
        fire(&host, &rebuilt);
        assert_eq!(fired.get(), 3);
        items.set(vec![]);
        assert!(configure(&interaction).is_none());
        assert_eq!(host.armed_command_count(), 0);
        assert!(!can(&host, &rebuilt));
        assert!(!can_with_sender(&host, &rebuilt, None));
        assert_eq!(commands(&host), [] as [Retained<UIKeyCommand>; 0]);
        // Repopulate, configure, then drop the mount: the scope retires
        // while the retained host and its command outlive it.
        items.set(vec![command(&fired)]);
        let _repopulated = configure(&interaction).expect("repopulated items build a menu");
        let live = commands(&host)
            .pop()
            .expect("the repopulated shortcut is vended");
        drop(mount);
        cocoa_ui::uikit::native_test::pump_main_until(MAIN_QUEUE_DEADLINE, || {
            host.armed_command_count() == 0
        });
        assert_eq!(host.armed_command_count(), 0);
        assert!(!can(&host, &live));
        assert!(
            commands(&host).iter().all(|command| {
                // SAFETY: `UIKeyCommand.action`'s declared getter.
                (unsafe { command.action() }) != Some(sel!(cocoaUiContextMenuCommandFired:))
            }),
            "a retired private command is still vended"
        );
    }
}

/// The main menu — the iPad menu bar and the keyboard-shortcut list — is
/// built by `UIKit`'s main menu system inside a running application. The
/// harness application hands `ApplicationHandlers::build_menus` a handler
/// that counts the builds it receives.
#[cfg(target_os = "ios")]
mod main_menu {
    use cocoa_ui::uikit::native_test::{main_menu_builds, pump_main_until};
    use libtest_mimic::Trial;

    use waterui_apple::native_test_support::MAIN_QUEUE_DEADLINE;

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
    /// built the main menu through it once the application is up. The
    /// build time is `UIKit`'s, not a launch-time guarantee, so the case
    /// waits for the build.
    fn the_main_menu_builds_through_the_build_menus_handler() {
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || main_menu_builds() > 0),
            "`UIKit` built the main menu through the handler"
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
    use std::cell::Cell;
    use std::rc::Rc;

    use cocoa_ui::Rect;
    use cocoa_ui::Retained;
    use cocoa_ui::menu::{Command, KeyModifiers, MenuTreeNode};
    use cocoa_ui::objc2_ui_kit::{
        UIAction, UIApplication, UIButton, UIKeyCommand, UIKeyModifierFlags, UIMenuElement,
        UIMenuElementAttributes, UIMenuElementState, UITextField, UIView, UIViewController,
        UIWindow,
    };
    use cocoa_ui::uikit::{
        self, KeyCommands, Menu, MenuAction, MenuButton, MenuElement, native_test,
    };
    use libtest_mimic::Trial;
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadMarker, Message as _, sel};

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
        let build = move |fired: &Rc<Cell<&'static str>>, label: &'static str| {
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
        let (menu, mut key_commands) = build(&fired, "Old");
        trigger.set_menu(&menu);
        let button = trigger
            .view()
            .downcast_ref::<UIButton>()
            .expect("a `MenuButton` is a `UIButton`")
            .retain();
        with_open_menu(&button, move |open| {
            assert_eq!(open.titles(), ["Old"]);

            // The `Menu` component's rebuild: the button takes the new
            // menu, then the previous menu's scope retires.
            let (menu, rebuilt) = build(&fired, "New");
            trigger.set_menu(&menu);
            drop(std::mem::replace(&mut key_commands, rebuilt));
            #[cfg(not(target_abi = "macabi"))]
            {
                assert_eq!(
                    open.titles(),
                    ["New"],
                    "the menu stays open and shows the rebuilt command as soon as `setMenu:` returns"
                );
                assert!(
                    open.press(mtm, 0),
                    "the visible command's action reaches a responder"
                );
                assert_eq!(fired.get(), "New", "picking runs the rebuilt callback");
            }
            // The `AppKit` popup keeps the rows it had, so the trigger ends
            // its tracking before replacing the menu: the harness fails the
            // case if the session is still running after the continuation,
            // and no retired command fired (a fired one panics the
            // registry).
            #[cfg(target_abi = "macabi")]
            {
                open.expect_tracking_to_end();
                assert_eq!(fired.get(), "", "no command fired during the rebuild");
            }
            drop(scene);
        });
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
        with_open_menu(&button, move |open| {
            assert_eq!(open.titles(), ["Keys"]);

            drop(mounted);
            // The `AppKit` popup ends only when control returns to its
            // tracking loop; the harness fails the case if the trigger did
            // not cancel it.
            #[cfg(target_abi = "macabi")]
            open.expect_tracking_to_end();
            #[cfg(not(target_abi = "macabi"))]
            assert!(
                open.is_closed(),
                "the menu closes as its leaf unmounts, before the leaf's commands retire"
            );
            drop(scene);
        });
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

    use open_menu::with_open_menu;

    /// How an open menu is observed and driven. `UIKit` presents a button
    /// menu as a context menu on iOS, and `UIContextMenuInteraction` vends
    /// its visible elements. Under Mac Catalyst it is an `AppKit` `NSMenu`
    /// popup whose tracking runs modally on the main thread, so its cases
    /// run *inside* the tracking session (water-rs/waterui#3560). Either
    /// way [`with_open_menu`] opens the menu the way a tap does, runs the
    /// case's continuation against the open menu, and leaves no
    /// presentation behind.
    #[cfg(not(target_abi = "macabi"))]
    mod open_menu {
        use std::cell::RefCell;
        use std::ptr::NonNull;

        use block2::StackBlock;
        use cocoa_ui::Retained;
        use cocoa_ui::objc2_ui_kit::{UIButton, UIContextMenuInteraction, UIMenu, UIMenuElement};
        use cocoa_ui::uikit::native_test::pump_main_until;
        use objc2::MainThreadMarker;

        use super::{ChordScene, MENU_DEADLINE};

        /// A menu `UIKit` shows.
        pub struct OpenMenu {
            interaction: Retained<UIContextMenuInteraction>,
        }

        impl OpenMenu {
            /// The elements the menu shows; none once it is closed or
            /// closing.
            fn elements(&self) -> Vec<Retained<UIMenuElement>> {
                let elements = RefCell::new(Vec::new());
                let block = StackBlock::new(|menu: NonNull<UIMenu>| {
                    // SAFETY: `UIKit` hands the block a live copy of the
                    // visible menu.
                    elements
                        .borrow_mut()
                        .extend(unsafe { menu.as_ref() }.children().iter());
                    menu
                });
                // SAFETY: the block returns the menu it is given, leaving
                // the visible menu as it is.
                unsafe { self.interaction.updateVisibleMenuWithBlock(&block) };
                elements.into_inner()
            }

            /// The titles of the elements the menu shows.
            pub fn titles(&self) -> Vec<String> {
                self.elements()
                    .iter()
                    .map(|element| element.title().to_string())
                    .collect()
            }

            /// Whether the menu is closed.
            pub fn is_closed(&self) -> bool {
                self.elements().is_empty()
            }

            /// Picks the element at `index` the way a chord does, and
            /// answers whether a responder took its action.
            pub fn press(&self, mtm: MainThreadMarker, index: usize) -> bool {
                ChordScene::press(mtm, &self.elements()[index])
            }
        }

        /// Dismisses the menu when a case ends, however it ends.
        struct Dismiss(Retained<UIContextMenuInteraction>);

        impl Drop for Dismiss {
            fn drop(&mut self) {
                self.0.dismissMenu();
            }
        }

        /// Opens `button`'s menu, waits until `UIKit` shows it and runs
        /// `inside` against it.
        pub fn with_open_menu(button: &UIButton, inside: impl FnOnce(&OpenMenu) + 'static) {
            button.performPrimaryAction();
            let interaction = button
                .contextMenuInteraction()
                .expect("a button with a menu has a context-menu interaction");
            let _dismiss = Dismiss(interaction.clone());
            let open = OpenMenu { interaction };
            assert!(
                pump_main_until(MENU_DEADLINE, || !open.is_closed()),
                "the button's primary action opens its menu"
            );
            inside(&open);
        }
    }

    /// The Catalyst realization: the presented `NSMenu`, observed through
    /// `cocoa_ui::uikit::catalyst_menu`.
    #[cfg(target_abi = "macabi")]
    mod open_menu {
        use cocoa_ui::Retained;
        use cocoa_ui::objc2_ui_kit::{UIButton, UIContextMenuInteraction};
        use cocoa_ui::uikit::catalyst_menu::{PresentedMenu, within_presented_menu};

        use super::MENU_DEADLINE;

        /// The `NSMenu` `AppKit` is tracking.
        pub struct OpenMenu<'a>(&'a PresentedMenu);

        impl OpenMenu<'_> {
            /// The titles of the menu's items.
            pub fn titles(&self) -> Vec<String> {
                self.0.titles()
            }

            /// Declares that the code under test ends the tracking itself
            /// once the continuation returns.
            pub fn expect_tracking_to_end(&self) {
                self.0.expect_tracking_to_end();
            }
        }

        /// Dismisses a presentation `AppKit` has not shown yet when a
        /// case ends, so it cannot appear inside a later case's pump.
        struct Dismiss(Option<Retained<UIContextMenuInteraction>>);

        impl Drop for Dismiss {
            fn drop(&mut self) {
                if let Some(interaction) = &self.0 {
                    interaction.dismissMenu();
                }
            }
        }

        /// Opens `button`'s menu, waits until `AppKit` begins tracking it
        /// and runs `inside` within the tracking session.
        pub fn with_open_menu(
            button: &UIButton,
            inside: impl for<'a> FnOnce(&OpenMenu<'a>) + 'static,
        ) {
            let mut dismiss = Dismiss(None);
            within_presented_menu(
                MENU_DEADLINE,
                || {
                    button.performPrimaryAction();
                    dismiss.0 = button.contextMenuInteraction();
                },
                move |presented| inside(&OpenMenu(presented)),
            );
        }
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

/// `NavigationSplitView` chrome inside the real `UIApplication` scene:
/// the leaf mounts under a `UIViewController` the window adopts, so the
/// split sees `parentViewController` and a live trait collection — the
/// shape a bare `initWithFrame` window never gives it.
#[cfg(target_os = "ios")]
mod navigation {
    use cocoa_ui::objc2_ui_kit::{
        UINavigationController, UISplitViewController, UISplitViewControllerColumn,
        UITraitEnvironment, UIUserInterfaceSizeClass, UIViewController,
    };
    use cocoa_ui::uikit::native_test;
    use cocoa_ui::uikit::view_controller::enclosing_controller;
    use cocoa_ui::{PlatformView, Rect, Retained, view};
    use waterui::background::Material;
    use waterui::component::{ButtonStyle, button, label};
    use waterui::icon::SystemIcon;
    use waterui::navigation::{
        NavigationSplitView, NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
        NavigationView, ToolbarItemIcon,
    };
    use waterui::prelude::text;
    use waterui::reactive::binding;
    use waterui::{Str, ViewExt as _};
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until, render};

    use super::mtm;

    /// The `navigation::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![libtest_mimic::Trial::test(
            "navigation::a_split_detail_column_carries_navigation_item_chrome",
            || {
                a_split_detail_column_carries_navigation_item_chrome();
                Ok(())
            },
        )]
    }

    /// The `UISplitViewController` a leaf's subtree lives under.
    fn split_controller_in(view: &PlatformView) -> Option<Retained<UISplitViewController>> {
        if let Some(controller) = enclosing_controller(view)
            && let Ok(split) = controller.downcast()
        {
            return Some(split);
        }
        for sub in view::subviews(view) {
            if let Some(found) = split_controller_in(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// The compact split fixture includes both a platform symbol and a hosted SVG label.
    fn navigation_fixture() -> waterui_apple::contract::NativeLeaf {
        let selection = binding(Some(0_i32));
        let query = binding(Str::default());
        render(NavigationSplitView::new(
            &selection,
            || text("sidebar"),
            move |_id: i32| {
                let mut item = NavigationToolbarItem::new(
                    NavigationToolbarPlacement::PrimaryAction,
                    button("").style(ButtonStyle::Plain),
                );
                item.icon = Some(ToolbarItemIcon::System(SystemIcon::new("plus")));
                NavigationView::new("Today", text("detail").background(Material::Regular))
                    .searchable(&query, "Search reminders")
                    .navigation_toolbar(NavigationToolbar::new(vec![
                        item,
                        NavigationToolbarItem::new(
                            NavigationToolbarPlacement::PrimaryAction,
                            button(label("").icon(waterui::svg::Svg::from_path(
                                "M19,13H13V19H11V13H5V11H11V5H13V11H19V13Z",
                                24.0,
                                24.0,
                            )))
                            .style(ButtonStyle::Borderless),
                        ),
                    ]))
            },
        ))
    }

    /// Native chrome must keep hosted icon and page content visible after collapse.
    fn assert_content_shown(
        window: &cocoa_ui::objc2_ui_kit::UIWindow,
        item: &cocoa_ui::objc2_ui_kit::UINavigationItem,
        page: &UIViewController,
    ) {
        let hosted = item
            .rightBarButtonItems()
            .expect("the page retains its actions")
            .iter()
            .find_map(|item| item.customView())
            .expect("arbitrary button content remains hosted");
        let icon = super::find::<cocoa_ui::uikit::ImageView>(&hosted)
            .expect("the hosted button retains its label icon");
        assert!(
            icon.downcast_ref::<cocoa_ui::uikit::ImageView>()
                .expect("picture image view")
                .image()
                .is_some(),
            "the hosted SVG produces a UIKit image"
        );
        let icon_shown = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            window.layoutIfNeeded();
            let bounds = view::bounds(&icon);
            icon.window().is_some()
                && view::has_visible_ancestry(&icon)
                && bounds.size.width > 0.0
                && bounds.size.height > 0.0
        });
        assert!(
            icon_shown,
            "the hosted toolbar icon remains visible with a nonempty frame: {:?}; wrapper: {:?}",
            view::bounds(&icon),
            view::bounds(&hosted),
        );
        let detail = super::find::<cocoa_ui::uikit::Label>(&page.view().expect("page view"))
            .expect("the detail page retains its rendered label");
        let content_shown = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            window.layoutIfNeeded();
            let bounds = view::bounds(&detail);
            detail.window().is_some()
                && view::has_visible_ancestry(&detail)
                && bounds.size.width > 0.0
                && bounds.size.height > 0.0
        });
        assert!(
            content_shown,
            "the collapsed detail content remains visible with a nonempty frame: {:?}",
            view::bounds(&detail)
        );
        let field = super::find::<cocoa_ui::objc2_ui_kit::UITextField>(window)
            .expect("the visible search text field")
            .downcast::<cocoa_ui::objc2_ui_kit::UITextField>()
            .expect("search text field type");
        // SAFETY: UIKit font accessors run on the test's main thread.
        let body = cocoa_ui::objc2_ui_kit::UIFont::preferredFontForTextStyle(unsafe {
            cocoa_ui::objc2_ui_kit::UIFontTextStyleBody
        });
        let configured = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            let actual = field.font().expect("the search field has a font");
            // SAFETY: UIKit font accessors run on the test's main thread.
            unsafe {
                actual.fontName() == body.fontName()
                    && (actual.pointSize() - body.pointSize()).abs() < 0.01
            }
        });
        assert!(
            configured,
            "the presented search field uses preferred body typography"
        );
    }

    /// A `NavigationSplitView` detail column is a stack-driven
    /// `UINavigationController`: the page's chrome lands on
    /// `navigationItem` — the `searchable` controller attaches, the
    /// action's symbol is a real bar image, the large-title gate is open —
    /// and `hidesBackButton` stays off, so a collapse onto compact pushes
    /// the page with the platform's own back affordance. The bare
    /// `NavContentController` column it replaces left the page drawing a
    /// standalone in-content bar: no back button, a stacked search field,
    /// an inline title and a hosted action drawn as an empty capsule
    /// (water-rs/waterui#2717).
    fn a_split_detail_column_carries_navigation_item_chrome() {
        let leaf = navigation_fixture();
        let mtm = mtm();
        let window = native_test::window(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
        let controller = UIViewController::new(mtm);
        window.setRootViewController(Some(&controller));
        let root_view = controller.view().expect("the controller's view is loaded");
        let mounted = leaf.mount(&root_view);
        view::set_frame(mounted.view(), view::bounds(&root_view));
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        let split =
            split_controller_in(&root_view).expect("the leaf mounts a UISplitViewController");
        assert!(
            split.parentViewController().is_some(),
            "the mounted scene adopts the split controller"
        );
        let detail_nav = split
            .viewControllerForColumn(UISplitViewControllerColumn::Secondary)
            .and_then(|controller| controller.downcast::<UINavigationController>().ok())
            .expect("the detail column is a UINavigationController");
        assert!(
            detail_nav.navigationBar().prefersLargeTitles(),
            "the column's bar leaves the large-title gate open"
        );
        let page = detail_nav
            .topViewController()
            .expect("the column stack has a root page");
        let item = page.navigationItem();
        assert!(
            !item.hidesBackButton(),
            "the column's root keeps the back affordance for the compact push"
        );
        assert!(
            item.searchController().is_some(),
            "the searchable field attaches as the item's search controller"
        );
        let trailing = item
            .rightBarButtonItems()
            .expect("the primary action is a bar item")
            .objectAtIndex(0);
        assert!(
            trailing.image().is_some() && trailing.customView().is_none(),
            "the action's symbol is the bar item's image, not a hosted view"
        );

        // The real scene's compact width already reports Compact: the
        // split collapses through the same trait transition production
        // responds to — no `traitOverrides` needed.
        // SAFETY: the trait collection is read on the main thread the
        // harness guarantees.
        let size_class = unsafe { split.traitCollection().horizontalSizeClass() };
        assert_eq!(
            size_class,
            UIUserInterfaceSizeClass::Compact,
            "the mounted scene carries compact width traits"
        );
        let collapsed = pump_main_until(MAIN_QUEUE_DEADLINE, || split.isCollapsed());
        assert!(collapsed, "compact traits collapse the split");
        let sidebar_nav = split
            .viewControllerForColumn(UISplitViewControllerColumn::Primary)
            .and_then(|controller| controller.downcast::<UINavigationController>().ok())
            .expect("the sidebar column stays a nav controller");
        let pushed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            sidebar_nav.viewControllers().count() == 2
        });
        assert!(
            pushed,
            "the collapsed split pushes the detail column onto the compact stack"
        );
        let top = sidebar_nav.topViewController().expect("a pushed column");
        assert!(
            core::ptr::eq(
                Retained::as_ptr(&top).cast::<u8>(),
                Retained::as_ptr(&detail_nav).cast::<u8>(),
            ),
            "the pushed column is the detail's own navigation stack"
        );
        assert!(
            !top.navigationItem().hidesBackButton(),
            "the pushed detail keeps the platform back button"
        );
        let bar_shown =
            pump_main_until(MAIN_QUEUE_DEADLINE, || !sidebar_nav.isNavigationBarHidden());
        assert!(
            bar_shown,
            "the pushed column unhides the compact stack's bar for the back button"
        );
        assert!(
            sidebar_nav.navigationBar().prefersLargeTitles(),
            "the compact stack's visible bar keeps the large-title gate open"
        );
        assert_content_shown(&window, &item, &page);
    }
}

#[cfg(all(target_os = "ios", feature = "native-test", feature = "text_field"))]
mod text_selection_ios {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cocoa_ui::Retained;
    use cocoa_ui::objc2::runtime::AnyObject;
    use cocoa_ui::objc2::sel;
    use cocoa_ui::objc2::{Message, msg_send};
    use cocoa_ui::objc2_foundation::{NSArray, NSRange, NSString, NSValue};
    use cocoa_ui::objc2_ui_kit::{
        UIAction, UIApplication, UIKeyCommand, UIMenu, UIMenuElement, UIMenuElementAttributes,
    };
    use waterui::Str;
    use waterui::component::menu::{Command, MenuItem, Shortcut};
    use waterui::component::text_field::TextField;
    use waterui::component::text_selection::TextSelection;
    use waterui::reactive::{Signal, SignalExt, binding};
    use waterui::text::StyledStr;

    use cocoa_ui::geometry::Rect;
    use cocoa_ui::objc2::MainThreadMarker;
    use cocoa_ui::uikit::native_test;
    use cocoa_ui::uikit::native_test::pump_main_until;
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, mount_uikit_window, render_environment,
    };

    use crate::{find, mtm};

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

/// Independent lists retain their own header spacing within a navigation page.
#[cfg(target_os = "ios")]
mod list_spacing {
    use super::mtm;
    use cocoa_ui::Retained;
    use cocoa_ui::objc2_ui_kit::{NSIndexPathUIKitAdditions, UITableView};
    use waterui::component::list::{List, ListItem};
    use waterui::id::SelfId;
    use waterui::prelude::*;
    use waterui::reactive::binding;
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, drain_main_queue, mount_uikit_window, pump_main_until,
        render_environment,
    };

    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "list_spacing::navigation_lists_keep_distinct_header_spacing_with_search",
                || {
                    check_spacing(true);
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "list_spacing::navigation_lists_keep_distinct_header_spacing_without_search",
                || {
                    check_spacing(false);
                    Ok(())
                },
            ),
        ]
    }

    fn tables(view: &cocoa_ui::PlatformView, out: &mut Vec<Retained<UITableView>>) {
        if let Some(table) = view.downcast_ref::<UITableView>() {
            out.push(Retained::from(table));
            return;
        }
        for child in cocoa_ui::view::subviews(view) {
            tables(&child, out);
        }
    }

    fn check_spacing(searchable: bool) {
        let list = || List::for_each(vec![SelfId::new(0_u32)], |_| ListItem::new(text("Row")));
        let query = binding(Str::default());
        let page = NavigationView::new(
            "Lists",
            vstack((list(), list())).background(waterui::background::Material::Regular),
        );
        let page = if searchable {
            page.searchable(&query, "Search")
        } else {
            page
        };
        let mount = mount_uikit_window(
            mtm(),
            AnyView::new(NavigationStack::new(page)),
            &render_environment(),
            cocoa_ui::uikit::native_test::window(
                mtm(),
                cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0),
            ),
        );
        let row = cocoa_ui::objc2_foundation::NSIndexPath::indexPathForRow_inSection(0, 0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                mount.window.layoutIfNeeded();
                let mut found = Vec::new();
                tables(&mount.host, &mut found);
                found.len() == 2
                    && found.iter().all(|t| {
                        t.bounds().size.height > 0.0
                            && t.rectForRowAtIndexPath(&row).size.height > 0.0
                    })
            }),
            "both lists must finish their native row layout"
        );
        assert!(drain_main_queue(mtm()), "pending row updates must finish");
        mount.window.layoutIfNeeded();
        let mut found = Vec::new();
        tables(&mount.host, &mut found);
        let expected = [0.0, 35.0];
        for (index, (table, expected)) in found.iter().zip(expected).enumerate() {
            let actual = table.rectForRowAtIndexPath(&row).origin.y;
            assert!(
                (actual - expected).abs() < 0.5,
                "list {index} with searchable={searchable}: first row begins at {actual}, expected {expected}"
            );
        }
    }
}

/// A view rehosted for capture lays out under the capture contract: the
/// capture window has no device regions, so a fixed container's child is
/// placed exactly as it is when the container is detached (#3704).
#[cfg(target_os = "ios")]
mod capture_regions {
    use cocoa_ui::objc2_ui_kit::UIEdgeInsets;
    use cocoa_ui::uikit::ImageView;
    use cocoa_ui::{Rect, view};
    use kurbo::Shape;
    use waterui::graphics::color::WorkingColor;
    use waterui::graphics::draw::Draw;
    use waterui::graphics::picture::Picture;
    use waterui::layout::frame::Frame;
    use waterui::reactive::constant;
    use waterui_apple::native_test_support::{render, with_capture_rehost};
    use waterui_core::layout::Size;

    use super::{find, mtm};

    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![libtest_mimic::Trial::test(
            "capture_regions::a_rehosted_fixed_container_places_its_child_at_full_size",
            || {
                a_rehosted_fixed_container_places_its_child_at_full_size();
                Ok(())
            },
        )]
    }

    fn a_rehosted_fixed_container_places_its_child_at_full_size() {
        let mtm = mtm();
        let picture = Picture::new(
            Size::new(24.0, 24.0),
            constant(Picture::record(|scene| {
                scene.fill(
                    kurbo::Rect::new(0.0, 0.0, 24.0, 24.0).to_path(0.1),
                    WorkingColor::BLACK,
                );
            })),
        );
        let leaf = render(
            Frame::new(picture)
                .ideal_width(24.0)
                .ideal_height(24.0)
                .max_width(24.0)
                .max_height(24.0),
        );
        let host = view::retain_base(leaf.view());
        // The detached state `tabs.rs` leaves a bar icon in: sized by its
        // parent-less owner, never laid out.
        view::set_frame(&host, Rect::new(0.0, 0.0, 24.0, 24.0));

        // The status-bar and home-indicator bands of a Dynamic Island
        // iPhone, as the iOS 27 runtimes that propagate them report them.
        let device_band = UIEdgeInsets {
            top: 62.0,
            left: 0.0,
            bottom: 34.0,
            right: 0.0,
        };
        with_capture_rehost(mtm, &host, device_band, |safe| {
            assert!(
                (host.safeAreaInsets().top - device_band.top).abs() < 0.01,
                "precondition: the device band reaches the rehosted host"
            );
            let image =
                find::<ImageView>(&host).expect("the picture leaf mounts the kit image view");
            let frame = view::frame(&image);
            assert!(
                (frame.size.width - 24.0).abs() < 0.01 && (frame.size.height - 24.0).abs() < 0.01,
                "the rehosted picture's image view is {:?}, not the 24x24 its container proposed",
                frame.size
            );
            assert_eq!(
                safe,
                view::bounds(&host),
                "a capture window has no regions: the host's safe rect is its bounds"
            );
        });
    }
}
