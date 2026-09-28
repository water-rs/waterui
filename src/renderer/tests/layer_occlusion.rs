//! A layer mounted over its siblings — a `when` payload, an overlay sibling
//! in a `zstack` — paints above the content beneath it but registers no
//! hit-test occluder the way an anchored overlay or a context menu does.
//! Hit-testing must still stop at the topmost target the press lands on,
//! whichever engine (pointer targets or gesture recognizers) carries it —
//! and whatever kind of layer mounted it.
//!
//! Reproduced from the watergram media viewer: a full-window viewer shown
//! with `when(viewer_open, ...)` over a message list, where one click on the
//! viewer's next-arrow dispatched both the arrow's handler and the tap
//! handler of the list row underneath.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use accesskit::Role;
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::widget::condition::when;
use waterui::{AnyView, ViewExt as _};
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{vstack, zstack};

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const WINDOW: (u32, u32) = (400, 700);
const ROWS: usize = 4;
const ROW_H: f32 = 48.0;
/// The tail row's tap region covers the middle of the window, where the
/// overlay centers its contents.
const TAP_ROW_H: f32 = 500.0;

fn primary_click(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    for event in [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ] {
        runtime.push_input_event(event);
    }
}

fn pump_until_settled(runtime: &mut HeadlessRuntime) -> Option<accesskit::TreeUpdate> {
    let mut update = None;
    for _ in 0..64 {
        if let Some(tree) = runtime.pump_at(false, Instant::now()).tree_update {
            update = Some(tree);
        }
        if runtime.is_settled() {
            break;
        }
    }
    update
}

/// The a11y bounds `label`'s control was last seen with.
fn bounds_of(runtime: &mut HeadlessRuntime, role: Role, label: &str) -> accesskit::Rect {
    let mut found = None;
    for _ in 0..64 {
        if let Some(update) = runtime.pump_at(false, Instant::now()).tree_update
            && let Some((_, node)) = find_by_label(&update, role, label)
        {
            found = node.bounds();
        }
        if runtime.is_settled() {
            break;
        }
    }
    found.unwrap_or_else(|| panic!("{label} must emit a node with bounds"))
}

/// A `scroll`ed `List` whose last row is `tail` — the row the layer covers.
fn list_with_tail(tail: impl Fn() -> AnyView + 'static) -> AnyView {
    let tail = Rc::new(tail);
    AnyView::new(scroll(List::for_each(
        SignalCollection::new((0..ROWS).map(SelfId::new).collect::<Vec<_>>()),
        move |row| {
            let index = row.into_inner();
            let content = if index == ROWS - 1 {
                tail()
            } else {
                AnyView::new(vstack((text(format!("Row {index}")),)).size(f32::INFINITY, ROW_H))
            };
            ListItem::new(content)
        },
    )))
}

/// The tail row carries the `.on_tap` region the layer must occlude.
fn tap_tail_row(taps: Rc<RefCell<u32>>) -> AnyView {
    AnyView::new(
        vstack((text("Tail row"),))
            .size(f32::INFINITY, TAP_ROW_H)
            .on_tap(move || *taps.borrow_mut() += 1),
    )
}

/// The tail row carries a `button` — a pointer target the layer's gesture
/// must occlude.
fn button_tail_row(pressed: Rc<Cell<bool>>) -> AnyView {
    AnyView::new(
        vstack((button("Row button").action(move || pressed.set(true)),))
            .size(f32::INFINITY, TAP_ROW_H),
    )
}

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    )
}

/// The watergram repro: a `when`-mounted viewer's control is a pointer
/// target; the row underneath is a tap gesture. One press on the arrow must
/// dispatch exactly one handler — the pointer dispatch and the gesture
/// engine see one hit-test order, not two.
#[test]
fn when_layer_control_press_does_not_fall_through_to_the_row_tap() {
    let open = Binding::container(false);
    let next = Rc::new(Cell::new(false));
    let taps = Rc::new(RefCell::new(0_u32));
    let view_open = open.clone();
    let view_next = next.clone();
    let view_taps = taps.clone();
    let mut runtime = runtime(AnyViewBuilder::<AnyView>::new(move || {
        let open = view_open.clone();
        let next = view_next.clone();
        let taps = view_taps.clone();
        AnyView::new(zstack((
            list_with_tail(move || tap_tail_row(taps.clone())),
            when(open, move || {
                let next = next.clone();
                vstack((button("Next").action(move || next.set(true)),))
                    .size(f32::INFINITY, f32::INFINITY)
            }),
        )))
    }));
    assert!(pump_until_settled(&mut runtime).is_some());
    open.set(true);
    pump_until_settled(&mut runtime);

    let next_bounds = bounds_of(&mut runtime, Role::Button, "Next");
    let (x, y) = (
        ((next_bounds.x0 + next_bounds.x1) / 2.0) as f32,
        ((next_bounds.y0 + next_bounds.y1) / 2.0) as f32,
    );
    assert!(
        y > ROWS as f32 * ROW_H - ROW_H,
        "the layer's control must sit over the tail row's tap region"
    );

    primary_click(&mut runtime, x, y);
    pump_until_settled(&mut runtime);

    assert!(next.get(), "the layer's control must run its action");
    assert_eq!(
        *taps.borrow(),
        0,
        "the press must not reach the tap region underneath the layer"
    );
}

/// The mirror: the `when`-mounted layer is a tap gesture region; the row
/// underneath carries a pointer-target control. The layer's gesture still
/// outranks the row's control — topmost hittable wins in both directions.
#[test]
fn when_layer_tap_press_does_not_fall_through_to_the_row_button() {
    let open = Binding::container(false);
    let layer_taps = Rc::new(RefCell::new(0_u32));
    let row_presses = Rc::new(Cell::new(false));
    let view_open = open.clone();
    let view_layer_taps = layer_taps.clone();
    let view_row_presses = row_presses.clone();
    let mut runtime = runtime(AnyViewBuilder::<AnyView>::new(move || {
        let open = view_open.clone();
        let layer_taps = view_layer_taps.clone();
        let row_presses = view_row_presses.clone();
        AnyView::new(zstack((
            list_with_tail(move || button_tail_row(row_presses.clone())),
            when(open, move || {
                let layer_taps = layer_taps.clone();
                vstack((text("Viewer"),))
                    .size(f32::INFINITY, f32::INFINITY)
                    .on_tap(move || *layer_taps.borrow_mut() += 1)
            }),
        )))
    }));
    assert!(pump_until_settled(&mut runtime).is_some());
    open.set(true);
    pump_until_settled(&mut runtime);

    let button_bounds = bounds_of(&mut runtime, Role::Button, "Row button");
    let (x, y) = (
        ((button_bounds.x0 + button_bounds.x1) / 2.0) as f32,
        ((button_bounds.y0 + button_bounds.y1) / 2.0) as f32,
    );

    primary_click(&mut runtime, x, y);
    pump_until_settled(&mut runtime);

    assert_eq!(
        *layer_taps.borrow(),
        1,
        "the layer's tap region must take the press"
    );
    assert!(
        !row_presses.get(),
        "the press must not reach the control underneath the layer"
    );
}

/// The same rule when both sides are gestures: a tap on the layer's own
/// `.on_tap` region, off any control, still reaches exactly one handler.
#[test]
fn when_layer_tap_press_does_not_fall_through_to_the_row_tap() {
    let open = Binding::container(false);
    let layer_taps = Rc::new(RefCell::new(0_u32));
    let row_taps = Rc::new(RefCell::new(0_u32));
    let view_open = open.clone();
    let view_layer_taps = layer_taps.clone();
    let view_row_taps = row_taps.clone();
    let mut runtime = runtime(AnyViewBuilder::<AnyView>::new(move || {
        let open = view_open.clone();
        let layer_taps = view_layer_taps.clone();
        let row_taps = view_row_taps.clone();
        AnyView::new(zstack((
            list_with_tail(move || tap_tail_row(row_taps.clone())),
            when(open, move || {
                let layer_taps = layer_taps.clone();
                vstack((text("Viewer"),))
                    .size(f32::INFINITY, f32::INFINITY)
                    .on_tap(move || *layer_taps.borrow_mut() += 1)
            }),
        )))
    }));
    assert!(pump_until_settled(&mut runtime).is_some());
    open.set(true);
    pump_until_settled(&mut runtime);

    // A point on the viewer with no control of its own: the tail row's tap
    // region underneath must still not take it.
    let (x, y) = (WINDOW.0 as f32 / 2.0, WINDOW.1 as f32 / 2.0);
    primary_click(&mut runtime, x, y);
    pump_until_settled(&mut runtime);

    assert_eq!(
        *layer_taps.borrow(),
        1,
        "the layer's tap region must take the press"
    );
    assert_eq!(
        *row_taps.borrow(),
        0,
        "the press must not reach the tap region underneath the layer"
    );
}

/// A static `zstack` overlay sibling — the same layering without the `when`
/// mount — obeys the same topmost-hittable rule.
#[test]
fn zstack_overlay_sibling_press_does_not_fall_through_to_the_row_tap() {
    let next = Rc::new(Cell::new(false));
    let taps = Rc::new(RefCell::new(0_u32));
    let view_next = next.clone();
    let view_taps = taps.clone();
    let mut runtime = runtime(AnyViewBuilder::<AnyView>::new(move || {
        let next = view_next.clone();
        let taps = view_taps.clone();
        AnyView::new(zstack((
            list_with_tail(move || tap_tail_row(taps.clone())),
            vstack((button("Next").action(move || next.set(true)),))
                .size(f32::INFINITY, f32::INFINITY),
        )))
    }));
    pump_until_settled(&mut runtime);

    let next_bounds = bounds_of(&mut runtime, Role::Button, "Next");
    let (x, y) = (
        ((next_bounds.x0 + next_bounds.x1) / 2.0) as f32,
        ((next_bounds.y0 + next_bounds.y1) / 2.0) as f32,
    );

    primary_click(&mut runtime, x, y);
    pump_until_settled(&mut runtime);

    assert!(next.get(), "the overlay's control must run its action");
    assert_eq!(
        *taps.borrow(),
        0,
        "the press must not reach the tap region underneath the overlay"
    );
}
