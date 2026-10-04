//! water-rs/hydrolysis#247 (water-rs/waterui#1245): `Command::shortcut` — the
//! chord renders as a trailing, right-aligned hint in menu rows and dispatches
//! on the window's scope while its `Menu` is mounted, `.context_menu` commands
//! while the menu is open.

use std::time::Instant;

use accesskit::Role;
use nami::Binding;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::prelude::{ContextMenu, Shortcut};
use waterui::widget::condition::when;
use waterui_controls::button::button;
use waterui_controls::menu::{CommandExt as _, Menu};
use waterui_controls::text_field::field;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::frame::Frame;
use waterui_layout::stack::vstack;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::engine::WidgetTheme;
use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind};

const WINDOW: (u32, u32) = (320, 240);

fn click(x: f32, y: f32, button: PointerButton) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button,
        },
    ]
}

fn key_chord(runtime: &mut HeadlessRuntime, key: &str, modifiers: Modifiers) {
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(InputEvent::Key {
            logical_key: keyboard_types::Key::Character(key.to_owned()),
            physical_code: keyboard_types::Code::Unidentified,
            repeat: false,
            key: KeyCode::Character(key.to_owned()),
            state,
            modifiers,
        });
    }
    let _ = runtime.pump(false);
}

fn ctrl() -> Modifiers {
    Modifiers {
        control: true,
        ..Modifiers::default()
    }
}

/// Pumps until the runtime settles (with a cap), then returns the merged
/// tree as of that settle — `None` when no window has ever produced one.
fn pump_until_settled(runtime: &mut HeadlessRuntime) -> Option<accesskit::TreeUpdate> {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
    runtime.accessibility_tree()
}

/// Same, capturing the composited frame on every pump — the pixel assertions
/// read the last one.
fn capture_until_settled(runtime: &mut HeadlessRuntime) -> crate::HeadlessSnapshot {
    let mut snapshot = None;
    for _ in 0..64 {
        let result = runtime.pump_at(true, Instant::now());
        if let Some(frame) = result.snapshot {
            snapshot = Some(frame);
        }
        if runtime.is_settled() {
            break;
        }
    }
    snapshot.expect("a settled runtime must capture a frame")
}

/// The a11y bounds `label` was last seen with under `role`.
fn bounds_of(runtime: &mut HeadlessRuntime, role: Role, label: &str) -> accesskit::Rect {
    pump_until_settled(runtime)
        .as_ref()
        .and_then(|update| find_by_label(update, role, label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{label} must emit a {role:?} with bounds"))
}

/// A click on `label`'s centre, followed by a settle.
fn click_label(runtime: &mut HeadlessRuntime, role: Role, label: &str, button: PointerButton) {
    let bounds = bounds_of(runtime, role, label);
    let (x, y) = (
        crate::num_cast::f64_as_f32(f64::midpoint(bounds.x0, bounds.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(bounds.y0, bounds.y1)),
    );
    for event in click(x, y, button) {
        runtime.push_input_event(event);
    }
    let _ = pump_until_settled(runtime);
}

fn is_ink(px: [u8; 4]) -> bool {
    px[..3].iter().any(|channel| *channel < 200)
}

/// The rightmost ink column inside `row` in the captured frame, if any.
fn rightmost_ink(snapshot: &crate::HeadlessSnapshot, row: kurbo::Rect) -> Option<u32> {
    let x0 = crate::num_cast::f64_as_u32(row.x0.max(0.0));
    let x1 = (crate::num_cast::f64_as_u32(row.x1)).min(snapshot.width - 1);
    let y0 = crate::num_cast::f64_as_u32(row.y0.max(0.0));
    let y1 = (crate::num_cast::f64_as_u32(row.y1)).min(snapshot.height - 1);
    (x0..=x1).rev().find(|x| {
        (y0..=y1).any(|y| {
            let index = ((y * snapshot.width + *x) * 4) as usize;
            is_ink(snapshot.rgba8[index..index + 4].try_into().expect("rgba"))
        })
    })
}

/// The hint a command's shortcut draws: a trailing, right-aligned piece of
/// text in the muted supporting style, present on every shortcut row and
/// absent on a row without one.
#[test]
fn command_shortcut_renders_a_trailing_aligned_hint() {
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(|| {
            AnyView::new(vstack((
                ().size(0.0, 20.0),
                Frame::new(
                    button("host").action(|| {}).context_menu(
                        ContextMenu::new(vec![
                            "Copy"
                                .action(|| {})
                                .shortcut(Shortcut::new("c").control().shift()),
                            "Paste".action(|| {}).shortcut(Shortcut::new("v").control()),
                            "Cut".action(|| {}),
                        ])
                        // An accessory lifts the menu into the drawn
                        // presentation so the rows mount in this window's
                        // scene.
                        .accessory(Frame::new(button("Like").action(|| {}))),
                    ),
                )
                .width(160.0)
                .height(80.0),
            )))
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    // The rows' frames come from the merged a11y update the open pump
    // publishes — the menu's pointer targets also cover the dimmed host and
    // the accessory, so `context_menu_row_frames` over-reports.
    let host = bounds_of(&mut runtime, Role::Button, "host");
    for event in click(
        crate::num_cast::f64_as_f32(f64::midpoint(host.x0, host.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(host.y0, host.y1)),
        PointerButton::Secondary,
    ) {
        runtime.push_input_event(event);
    }
    let update = pump_until_settled(&mut runtime).expect("the open publishes a tree update");
    assert!(
        runtime.context_menu_presentation_frames().is_some(),
        "the accessory menu opens the drawn presentation"
    );
    let rows: Vec<kurbo::Rect> = ["Copy", "Paste", "Cut"]
        .iter()
        .map(|label| {
            let (_, node) = find_by_label(&update, Role::Button, label)
                .unwrap_or_else(|| panic!("{label} must emit a menu row"));
            let rect = node.bounds().expect("a menu row has bounds");
            kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
        })
        .collect();
    let snapshot = capture_until_settled(&mut runtime);
    let row_inset = MinimalTestTheme::default()
        .text_context_menu_metrics()
        .horizontal_padding;
    let (menu_frame, _) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn presentation is open");

    // The a11y row bounds wrap the label content, so scan the full panel
    // width inside each row's band for the trailing ink edge.
    let scan_rows: Vec<kurbo::Rect> = rows
        .iter()
        .map(|row| kurbo::Rect::new(menu_frame.x0 + 1.0, row.y0, menu_frame.x1 - 1.0, row.y1))
        .collect();
    let trailing: Vec<u32> = scan_rows
        .iter()
        .map(|row| rightmost_ink(&snapshot, *row).expect("a menu row paints ink"))
        .collect();

    // The two shortcut rows share one trailing edge — right-aligned at the
    // panel's trailing inset — where the no-shortcut row's ink stops at its
    // label.
    assert!(
        (trailing[0].abs_diff(trailing[1])) <= 2,
        "shortcut hints share one trailing edge: {trailing:?}"
    );
    let hint_edge = crate::num_cast::f64_as_u32(menu_frame.x1 - row_inset);
    assert!(
        trailing[0].abs_diff(hint_edge) <= 2,
        "hint hugs the row's trailing inset: ink {trailing:?} edge {hint_edge}"
    );
    assert!(
        trailing[2] < trailing[0] - 8,
        "the no-shortcut row has no trailing hint: {trailing:?}"
    );
}

/// A `Menu` mounted in a window dispatches its chord on that window's scope:
/// Ctrl+B fires the command while a text field holds focus, and stops firing
/// once the menu unmounts — the early modifier return and the text input's
/// own key handling both come later.
#[test]
fn menu_shortcut_fires_while_text_field_focused_and_stops_after_unmount() {
    let hits = Binding::container(0_i32);
    let draft = Binding::container(waterui_core::Str::from(""));
    let mounted = Binding::bool(true);
    let view = {
        let hits = hits.clone();
        let draft = draft;
        let mounted = mounted.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let hits = hits.clone();
            let draft = draft.clone();
            let mounted = mounted.clone();
            AnyView::new(vstack((
                Frame::new(field("Draft", &draft)).width(200.0).height(40.0),
                when(mounted, move || {
                    let hits = hits.clone();
                    Menu::new(
                        "Actions",
                        "Bump"
                            .action(move || hits.set(hits.snapshot() + 1))
                            .shortcut(Shortcut::new("b").control()),
                    )
                }),
            )))
        })
    };
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        view,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    // Focus the field: its a11y bounds get the primary click.
    click_label(
        &mut runtime,
        Role::TextInput,
        "Draft",
        PointerButton::Primary,
    );

    key_chord(&mut runtime, "b", ctrl());
    assert_eq!(
        hits.snapshot(),
        1,
        "Ctrl+B runs the command with a text field focused"
    );

    mounted.set(false);
    let _ = pump_until_settled(&mut runtime);
    key_chord(&mut runtime, "b", ctrl());
    assert_eq!(
        hits.snapshot(),
        1,
        "an unmounted menu no longer claims Ctrl+B"
    );
}

/// A `.context_menu` chord dispatches only while the menu is open: before
/// opening nothing claims it, with the menu open it fires (and closes the
/// menu), and after the menu closes it is inert again.
#[test]
fn context_menu_shortcut_fires_only_while_open() {
    let hits = Binding::container(0_i32);
    let host = {
        let hits = hits.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let hits = hits.clone();
            AnyView::new(
                Frame::new(
                    button("host")
                        .action(|| {})
                        .context_menu(ContextMenu::new(vec![
                            "Copy"
                                .action(move || hits.set(hits.snapshot() + 1))
                                .shortcut(Shortcut::new("c").control().shift()),
                        ])),
                )
                .width(160.0)
                .height(80.0),
            )
        })
    };
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        host,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    // bounds_of consumes the first tree update — read the host's bounds
    // before any key pump discards it.
    let bounds = bounds_of(&mut runtime, Role::Button, "host");
    let (x, y) = (
        crate::num_cast::f64_as_f32(f64::midpoint(bounds.x0, bounds.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(bounds.y0, bounds.y1)),
    );
    let chord = Modifiers {
        control: true,
        shift: true,
        ..Modifiers::default()
    };
    key_chord(&mut runtime, "c", chord);
    assert_eq!(hits.snapshot(), 0, "a closed context menu is inert");

    for event in click(x, y, PointerButton::Secondary) {
        runtime.push_input_event(event);
    }
    let _ = pump_until_settled(&mut runtime);
    assert_eq!(runtime.popup_frames().len(), 1, "the context menu opens");

    key_chord(&mut runtime, "c", chord);
    assert_eq!(hits.snapshot(), 1, "the open context menu's chord fires");

    let _ = pump_until_settled(&mut runtime);
    key_chord(&mut runtime, "c", chord);
    assert_eq!(hits.snapshot(), 1, "the closed menu's chord is inert again");
}

/// When two mounted menus register the same chord, the most recently mounted
/// one wins.
#[test]
fn the_most_recently_mounted_menu_wins_a_chord_conflict() {
    let first = Binding::container(0_i32);
    let second = Binding::container(0_i32);
    let view = {
        let first = first.clone();
        let second = second.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let first = first.clone();
            let second = second.clone();
            AnyView::new(vstack((
                Menu::new(
                    "First",
                    "Bump"
                        .action(move || first.set(first.snapshot() + 1))
                        .shortcut(Shortcut::new("b").control()),
                ),
                Menu::new(
                    "Second",
                    "Bump"
                        .action(move || second.set(second.snapshot() + 1))
                        .shortcut(Shortcut::new("b").control()),
                ),
            )))
        })
    };
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        view,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let _ = pump_until_settled(&mut runtime);

    key_chord(&mut runtime, "b", ctrl());
    assert_eq!(
        first.snapshot(),
        0,
        "the older registration loses the chord"
    );
    assert_eq!(second.snapshot(), 1, "the newest registration wins it");
}

/// The app's `menu_bar` arms its chords with no `Menu` mounted: the runner
/// registers the resolved menus as an app-scoped source on the shared
/// registry, so `Ctrl+W` reaches the command in a window whose content is a
/// plain view. (watergram DOGFOOD r43-1)
#[test]
fn menu_bar_chord_fires_without_a_mounted_menu() {
    let fired = Binding::container(0_i32);
    let menu_bar = {
        let fired = fired.clone();
        nami::Computed::constant(vec![Menu::new(
            "App",
            "Quit"
                .action(move || fired.set(fired.snapshot() + 1))
                .shortcut(Shortcut::new("w").control()),
        )])
    };
    let env = test_environment();
    let _menu_bar_items = crate::runner::menu_bar::register_menu_bar(&menu_bar, &env);
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        AnyViewBuilder::<AnyView>::new(|| AnyView::new(button("plain").action(|| {}))),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    let _ = pump_until_settled(&mut runtime);

    key_chord(&mut runtime, "w", ctrl());
    assert_eq!(
        fired.snapshot(),
        1,
        "the app menu bar's chord dispatches with no Menu mounted"
    );
}

/// With the app bar and a mounted `Menu` claiming the same chord, the
/// mounted menu — the newer registration — wins while it is up, and the
/// app bar answers again once it unmounts (watergram DOGFOOD r43-1;
/// `the_most_recently_mounted_menu_wins_a_chord_conflict` for the
/// mounted-vs-mounted order).
#[test]
fn a_mounted_menu_wins_the_app_bars_chord_while_mounted() {
    let bar_fired = Binding::container(0_i32);
    let menu_fired = Binding::container(0_i32);
    let menu_bar = {
        let bar_fired = bar_fired.clone();
        nami::Computed::constant(vec![Menu::new(
            "App",
            "Quit"
                .action(move || bar_fired.set(bar_fired.snapshot() + 1))
                .shortcut(Shortcut::new("w").control()),
        )])
    };
    let mounted = Binding::container(false);
    let env = test_environment();
    let _menu_bar_items = crate::runner::menu_bar::register_menu_bar(&menu_bar, &env);
    let view = {
        let menu_fired = menu_fired.clone();
        let mounted = mounted.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let menu_fired = menu_fired.clone();
            let mounted = mounted.clone();
            AnyView::new(vstack((
                button("plain").action(|| {}),
                when(mounted, move || {
                    let menu_fired = menu_fired.clone();
                    Menu::new(
                        "Conflicting",
                        "Bump"
                            .action(move || menu_fired.set(menu_fired.snapshot() + 1))
                            .shortcut(Shortcut::new("w").control()),
                    )
                }),
            )))
        })
    };
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, view, WINDOW.0, WINDOW.1, MinimalTestTheme::default());
    let _ = pump_until_settled(&mut runtime);

    mounted.set(true);
    let _ = pump_until_settled(&mut runtime);
    key_chord(&mut runtime, "w", ctrl());
    assert_eq!(
        menu_fired.snapshot(),
        1,
        "the freshly mounted menu wins the shared chord"
    );
    assert_eq!(bar_fired.snapshot(), 0, "the app bar yields to it");

    mounted.set(false);
    let _ = pump_until_settled(&mut runtime);
    key_chord(&mut runtime, "w", ctrl());
    assert_eq!(
        bar_fired.snapshot(),
        1,
        "the app bar answers again once the menu unmounts"
    );
    assert_eq!(menu_fired.snapshot(), 1, "the unmounted menu is inert");
}
