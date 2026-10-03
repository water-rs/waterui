//! Nested-menu dispatch: a `Menu` nested inside a `.context_menu` or a
//! `TextField` selection menu must open its own popup and dispatch its
//! commands. The selection-menu case regressed water-rs/hydrolysis#317 —
//! a nested `Menu` in `.selection_menu` panicked while the menu was being
//! built; it now opens through the shared popup-submenu machinery.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::Role;
use nami::Binding;
use waterui::component::text;
use waterui::{AnyView, Str, ViewExt as _};
use waterui_controls::menu::{CommandExt as _, Menu};
use waterui_controls::text_field::field;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::input::{Code, Key};

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind};
use crate::renderer::input::PopupMenuNode;
use crate::renderer::input::text_editing::ActiveTextContextMenu;

const WINDOW: (u32, u32) = (400, 400);

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    )
}

fn pump_until_settled(runtime: &mut HeadlessRuntime) {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
}

fn click(runtime: &mut HeadlessRuntime, x: f32, y: f32, button: PointerButton) {
    for event in [
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
    ] {
        runtime.push_input_event(event);
    }
    pump_until_settled(runtime);
}

/// `label`'s bounds in the topmost popup window, translated into the main
/// window's coordinate space (`push_input_event` routes on those).
fn popup_bounds_of(runtime: &mut HeadlessRuntime, label: &str) -> accesskit::Rect {
    pump_until_settled(runtime);
    let frame = *runtime
        .popup_frames()
        .last()
        .expect("a popup window must be mounted");
    let item = runtime
        .accessibility_tree()
        .as_ref()
        .and_then(|update| find_by_label(update, Role::Button, label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{label} must emit a button with bounds"));
    accesskit::Rect::new(
        f64::from(frame.x()) + item.x0,
        f64::from(frame.y()) + item.y0,
        f64::from(frame.x()) + item.x1,
        f64::from(frame.y()) + item.y1,
    )
}

fn center(rect: accesskit::Rect) -> (f32, f32) {
    (
        f64::midpoint(rect.x0, rect.x1) as f32,
        f64::midpoint(rect.y0, rect.y1) as f32,
    )
}

/// Right-click the anchor, press the submenu row, then press the nested
/// command: every stage must land.
#[test]
fn nested_menu_command_dispatches() {
    let fired = Rc::new(Cell::new(0_u32));
    let fired_clone = fired.clone();
    let mut runtime = runtime(AnyViewBuilder::<AnyView>::new(move || {
        let deep = fired_clone.clone();
        AnyView::new(text("anchor").context_menu((
            "Top".action(|| {}),
            Menu::new("Sub", "Deep".action(move || deep.set(deep.get() + 1))),
        )))
    }));
    pump_until_settled(&mut runtime);

    click(&mut runtime, 40.0, 40.0, PointerButton::Secondary);

    let sub = popup_bounds_of(&mut runtime, "Sub");
    let (x, y) = center(sub);
    click(&mut runtime, x, y, PointerButton::Primary);

    let deep = popup_bounds_of(&mut runtime, "Deep");
    let (x, y) = center(deep);
    click(&mut runtime, x, y, PointerButton::Primary);

    assert_eq!(fired.get(), 1, "the nested command must dispatch");
}

/// Select-all in the focused field, right-click it, press the `More` row of
/// the drawn selection menu, then press the nested command in the submenu
/// popup the row opens (water-rs/hydrolysis#317).
#[test]
fn selection_menu_nested_command_dispatches() {
    let fired = Rc::new(Cell::new(0_u32));
    let value = Binding::container(Str::from("hello"));
    let fired_clone = fired.clone();
    let mut runtime = runtime(AnyViewBuilder::<AnyView>::new(move || {
        let deep = fired_clone.clone();
        AnyView::new(
            field("Name", &value)
                .selection_menu((
                    "Open".action(|| {}),
                    Menu::new("More", "Deep".action(move || deep.set(deep.get() + 1))),
                ))
                .size(300.0, 60.0),
        )
    }));
    pump_until_settled(&mut runtime);

    let field_bounds = runtime.renderer().text_editing.text_input_targets[0].bounds;
    let field_center = field_bounds.center();
    click(
        &mut runtime,
        field_center.x as f32,
        field_center.y as f32,
        PointerButton::Primary,
    );
    runtime.push_input_event(InputEvent::Key {
        key: KeyCode::Character("a".to_string()),
        logical_key: Key::Character("a".into()),
        physical_code: Code::KeyA,
        repeat: false,
        state: KeyState::Pressed,
        modifiers: Modifiers {
            super_key: true,
            ..Modifiers::default()
        },
    });
    pump_until_settled(&mut runtime);

    // The secondary press must land inside the selection — a press outside
    // it collapses the range before the menu builds, hiding the custom items.
    click(
        &mut runtime,
        (field_bounds.x0 + 8.0) as f32,
        field_center.y as f32,
        PointerButton::Secondary,
    );

    // The drawn selection-menu overlay emits no popup window, so the `More`
    // row's bounds come from the mounted overlay's row list.
    let submenu_row = {
        let menu = runtime
            .renderer()
            .text_editing
            .active_text_context_menu
            .as_ref()
            .expect("the selection menu must be open");
        let ActiveTextContextMenu::Overlay { overlay, .. } = menu else {
            panic!("the headless selection menu is a drawn overlay")
        };
        overlay
            .rows
            .iter()
            .find(|row| matches!(row.node, PopupMenuNode::Menu { .. }))
            .map(|row| row.bounds)
            .expect("the nested Menu must survive as a submenu row")
    };
    click(
        &mut runtime,
        f64::midpoint(submenu_row.x0, submenu_row.x1) as f32,
        f64::midpoint(submenu_row.y0, submenu_row.y1) as f32,
        PointerButton::Primary,
    );

    let deep = popup_bounds_of(&mut runtime, "Deep");
    let (x, y) = center(deep);
    click(&mut runtime, x, y, PointerButton::Primary);

    assert_eq!(fired.get(), 1, "the nested command must dispatch");
}
