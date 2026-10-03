//! Toggling `.visible(bool)` on a mounted `List` panics the accessibility
//! flush without a live check on resolved focus links.
//!
//! `.visible` hides through `AccessibilityStateSignal` — no role and no
//! label, so the list gets no `accessibility_container_env` scope and no
//! suppression. `list_accessibility` then registers no `ListItem` node for a
//! hidden row (`row_hidden`) and stamps no focus link, but the link the
//! visible flush wrote into `interaction_nodes` is only pruned in
//! `finalize_tree_update`. `render_list_parts` resolved that stale id through
//! `focus_node_for_key`, pushed it on the accessibility parent stack, and the
//! first child leaf registration panicked on "parent stack contains an
//! unknown node" — the watergram repro gates list sections this way.

use std::time::Instant;

use accesskit::{NodeId, Role, TreeUpdate};
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem, ListSection};
use waterui::component::text;
use waterui::{AnyView, Environment, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 7;
const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
const ROWS: u64 = 4;

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
            x: x as f32,
            y: y as f32,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x: x as f32,
            y: y as f32,
            button: PointerButton::Primary,
        },
    ] {
        runtime.push_input_event(event);
    }
}

/// The gated section: a `List` with a section marker, `.visible(shown)` on
/// the whole subtree — the same env-level `AccessibilityStateSignal` the
/// dogfood app gates each section with — plus a selection so the rows carry
/// the press slots `focus_node_for_key` resolves.
fn sectioned_list(
    shown: Binding<bool>,
    selection: Binding<Option<u64>>,
) -> AnyViewBuilder<AnyView> {
    AnyViewBuilder::<AnyView>::new(move || {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        let shown = shown.clone();
        let selection = selection.clone();
        AnyView::new(
            List::for_each(SignalCollection::new(rows), move |row| {
                let index = row.into_inner();
                let item = ListItem::new(text(format!("Row {index}")));
                if index == 0 {
                    item.section(ListSection::new("Section"))
                } else {
                    item
                }
            })
            .selection(&selection)
            .visible(shown),
        )
    })
}

/// A `.visible(false)` section of a mounted list must emit no nodes and —
/// critically — must not panic the flush on the stale focus link its last
/// visible emission left behind.
#[test]
fn hiding_a_mounted_list_section_emits_no_stale_parent() {
    let shown = Binding::bool(true);
    let mut runtime = runtime(
        test_environment(),
        sectioned_list(shown.clone(), Binding::container(None::<u64>)),
    );
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    find_node(&updates, Role::ListItem, "Row 1")
        .expect("a visible row must publish a ListItem node");

    // The hide flush is the repro: the hidden row emits no `ListItem`, yet
    // `interaction_nodes` still resolves its anchor key to the previous
    // flush's node id — pre-fix the draw pass pushed it and the first child
    // attach panicked.
    shown.set(false);
    let updates = settle(&mut runtime, &mut at, 2);
    let latest = updates.last().expect("the hide flush must publish a tree");
    assert!(
        latest
            .nodes
            .iter()
            .all(|(_, node)| node.role() != Role::ListItem),
        "hidden rows emit no nodes",
    );

    // Showing again re-registers the link cleanly instead of reusing the
    // stale id.
    shown.set(true);
    let updates = settle(&mut runtime, &mut at, 2);
    find_node(&updates, Role::ListItem, "Row 1")
        .expect("a re-shown row must publish its ListItem node again");
}

/// Focus resting on a row when its section hides must not dangle: the hide
/// drops the dead link and relocates focus, and once the section shows again
/// the re-stamped link resolves so a click lands semantic focus back on the
/// row's `ListItem`.
#[test]
fn a_focused_row_hidden_and_shown_again_refocuses() {
    let shown = Binding::bool(true);
    let selection = Binding::container(None::<u64>);
    let mut runtime = runtime(test_environment(), sectioned_list(shown.clone(), selection));
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    let (row1, row1_bounds) = find_node(&updates, Role::ListItem, "Row 1")
        .expect("a visible row must publish a ListItem node");

    click(
        &mut runtime,
        (row1_bounds.x0 + row1_bounds.x1) / 2.0,
        (row1_bounds.y0 + row1_bounds.y1) / 2.0,
    );
    settle(&mut runtime, &mut at, 2);
    assert_eq!(
        runtime.renderer().keyboard_focus_node(),
        Some(row1),
        "the click must land semantic focus on the row's ListItem",
    );

    shown.set(false);
    settle(&mut runtime, &mut at, 2);
    assert_ne!(
        runtime.renderer().keyboard_focus_node(),
        Some(row1),
        "focus must not stay on the hidden row's unregistered node",
    );

    shown.set(true);
    let updates = settle(&mut runtime, &mut at, 2);
    let (row1, row1_bounds) = find_node(&updates, Role::ListItem, "Row 1")
        .expect("a re-shown row must publish its ListItem node again");

    click(
        &mut runtime,
        (row1_bounds.x0 + row1_bounds.x1) / 2.0,
        (row1_bounds.y0 + row1_bounds.y1) / 2.0,
    );
    settle(&mut runtime, &mut at, 2);
    assert_eq!(
        runtime.renderer().keyboard_focus_node(),
        Some(row1),
        "a click after re-show must resolve the row through its re-stamped link",
    );
}
