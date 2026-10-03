//! Row keyboard-focus regression tests for water-rs/hydrolysis#220.
//!
//! A pointer press landing on a `List` row's tap content must leave the
//! semantic focus on the row's `ListItem` node so the ArrowUp/Down
//! navigation `list_row_context` drives has a row to start from. The
//! `.on_tap` the content carries never owns a node of its own — one
//! variant claims the row's selection press through a nested gesture
//! region, the other runs as a press-slot target under an
//! `InteractionStyle` — and before the fix either left
//! `accessibility.focus` on the tree root or parked `hit_test
//! .keyboard_focus` on a machinery-less link the post-flush liveness pass
//! declared dead, so every arrow fell through.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{NodeId, Role, TreeUpdate};
use nami::Binding;
use nami::Signal as _;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::{AnyView, Color, Environment, ViewExt as _};
use waterui_backend_core::widget::{ButtonMetrics, InteractionStyle};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::keyboard_types;
use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers, PointerButton, PointerKind};

const POINTER_ID: u64 = 7;
const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
const ROWS: u64 = 8;

fn runtime(env: Environment, builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        env,
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Pumps until the runtime reports quiet — never fewer than `min_frames`, so
/// a frame still in flight cannot be mistaken for settled — then returns the
/// settled merged tree (as the vec `find_node` reads).
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant, min_frames: u32) -> Vec<TreeUpdate> {
    let mut frame = 0;
    loop {
        frame += 1;
        *at += core::time::Duration::from_millis(16);
        let _ = runtime.pump_at(false, *at);
        if frame >= min_frames
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
    runtime.accessibility_tree().into_iter().collect()
}

/// The node labelled `label` with `role` in the settled tree.
fn find_node(updates: &[TreeUpdate], role: Role, label: &str) -> Option<(NodeId, accesskit::Rect)> {
    updates.iter().rev().find_map(|update| {
        update.nodes.iter().find_map(|(id, node)| {
            (node.role() == role && node.label() == Some(label))
                .then_some(*id)
                .zip(node.bounds())
        })
    })
}

fn click(runtime: &mut HeadlessRuntime, x: f64, y: f64) {
    for event in [
        InputEvent::PointerDown {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x: crate::num_cast::f64_as_f32(x),
            y: crate::num_cast::f64_as_f32(y),
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x: crate::num_cast::f64_as_f32(x),
            y: crate::num_cast::f64_as_f32(y),
            button: PointerButton::Primary,
        },
    ] {
        runtime.push_input_event(event);
    }
}

fn arrow_down(runtime: &mut HeadlessRuntime) {
    for state in [KeyState::Pressed, KeyState::Released] {
        runtime.push_input_event(InputEvent::Key {
            logical_key: KeyCode::Named("ArrowDown".into()).to_w3c_key(),
            physical_code: keyboard_types::Code::Unidentified,
            repeat: false,
            key: KeyCode::Named("ArrowDown".into()),
            state,
            modifiers: Modifiers::default(),
        });
    }
}

/// Clicking the `.on_tap` element nested inside a row's content claims the
/// row's selection press for the gesture; the row's `ListItem` node must
/// still take the focus, so the first `ArrowDown` moves to the next row.
#[test]
fn tap_gesture_inside_row_focuses_the_row_for_arrows() {
    let selection = Binding::container(None::<u64>);
    let taps: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let taps_for_rows = taps.clone();
    let selection_for_list = selection.clone();
    let mut runtime = runtime(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let taps = taps_for_rows.clone();
            AnyView::new(
                List::for_each(SignalCollection::new(rows), move |row| {
                    let index = row.into_inner();
                    let taps = taps.clone();
                    ListItem::new(vstack((
                        text(format!("Row {index}")).size(20.0),
                        text(format!("Open {index}")).size(12.0).on_tap(move || {
                            taps.borrow_mut().push(index);
                        }),
                    )))
                })
                .selection(&selection_for_list),
            )
        }),
    );
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    let (row2, _) = find_node(&updates, Role::ListItem, "Row 2 Open 2")
        .expect("row 2 must publish a ListItem node");
    let (row3, _) = find_node(&updates, Role::ListItem, "Row 3 Open 3")
        .expect("row 3 must publish a ListItem node");
    let (_, tap_bounds) = find_node(&updates, Role::Label, "Open 2")
        .expect("the row's tap label must publish a node");

    click(
        &mut runtime,
        f64::midpoint(tap_bounds.x0, tap_bounds.x1),
        f64::midpoint(tap_bounds.y0, tap_bounds.y1),
    );
    settle(&mut runtime, &mut at, 2);

    assert_eq!(
        taps.borrow().as_slice(),
        &[2],
        "the claimed tap still activates through the gesture engine",
    );
    assert_eq!(
        selection.snapshot(),
        None,
        "the claim takes the selection press, so the click itself selects nothing",
    );
    assert_eq!(
        runtime.renderer().keyboard_focus_node(),
        Some(row2),
        "the press must land the semantic focus on the row's ListItem node",
    );

    arrow_down(&mut runtime);
    settle(&mut runtime, &mut at, 2);
    assert_eq!(
        runtime.renderer().keyboard_focus_node(),
        Some(row3),
        "ArrowDown from the pressed row must focus the next row",
    );
    assert_eq!(
        selection.snapshot(),
        Some(3_u64),
        "ArrowDown must move the list's selection to the next row",
    );
}

/// With an `InteractionStyle` installed the `.on_tap` registers a press
/// target instead of a gesture region: the press commits to the tap's own
/// interaction identity — a key linked to no node — and the row's
/// `ListItem` node must take the focus all the same.
#[test]
fn tap_press_target_inside_row_focuses_the_row_for_arrows() {
    let mut env = test_environment();
    env.install(InteractionStyle::new(
        ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
        Color::srgb(0, 0, 0),
        0.0_f64,
    ));
    let selection = Binding::container(None::<u64>);
    let taps: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let taps_for_rows = taps.clone();
    let selection_for_list = selection.clone();
    let mut runtime = runtime(
        env,
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let taps = taps_for_rows.clone();
            AnyView::new(
                List::for_each(SignalCollection::new(rows), move |row| {
                    let index = row.into_inner();
                    let taps = taps.clone();
                    ListItem::new(text(format!("Row {index}")).size(20.0).on_tap(move || {
                        taps.borrow_mut().push(index);
                    }))
                })
                .selection(&selection_for_list),
            )
        }),
    );
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    let (row2, row2_bounds) =
        find_node(&updates, Role::ListItem, "Row 2").expect("row 2 must publish a ListItem node");
    let (row3, _) =
        find_node(&updates, Role::ListItem, "Row 3").expect("row 3 must publish a ListItem node");

    click(
        &mut runtime,
        f64::midpoint(row2_bounds.x0, row2_bounds.x1),
        f64::midpoint(row2_bounds.y0, row2_bounds.y1),
    );
    settle(&mut runtime, &mut at, 2);

    assert_eq!(
        taps.borrow().as_slice(),
        &[2],
        "the tap press target still activates",
    );
    assert_eq!(
        runtime.renderer().keyboard_focus_node(),
        Some(row2),
        "the press must land the semantic focus on the row's ListItem node",
    );

    arrow_down(&mut runtime);
    settle(&mut runtime, &mut at, 2);
    assert_eq!(
        runtime.renderer().keyboard_focus_node(),
        Some(row3),
        "ArrowDown from the pressed row must focus the next row",
    );
    assert_eq!(
        selection.snapshot(),
        Some(3_u64),
        "ArrowDown must move the list's selection to the next row",
    );
}
