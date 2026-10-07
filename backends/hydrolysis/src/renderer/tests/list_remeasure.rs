//! Row re-measurement regression tests for water-rs/hydrolysis#199.
//!
//! A `List` row's extent must track its content's measured size: a signal that
//! changes what a row's content measures re-measures exactly that row — other
//! rows keep their cached extents — and a change to a row above the viewport
//! must not shift the row the viewport is anchored to.

use core::time::Duration;
use std::time::Instant;

use accesskit::{Rect, Role, TreeUpdate};
use nami::Binding;
use nami::SignalExt as _;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll::ScrollController;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

/// The reproduction's viewport: the issue mounts offscreen at 400x700.
const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
/// `MinimalTestTheme`'s one-line row height — where a short label's slot rests.
const ROW_HEIGHT: f64 = 56.0;
/// The reproduction's text sizes: 20pt content fits inside the row floor,
/// 160pt content does not.
const SHORT_SIZE: f64 = 20.0;
const TALL_SIZE: f64 = 160.0;
/// Anything past this is provably grown — the floor is 56, tall content lands
/// near 200.
const GROWN_THRESHOLD: f64 = 100.0;

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Pumps until the runtime reports quiet — never fewer than `min_frames`, so a
/// glide or animation still in flight cannot be mistaken for settled — then
/// returns the settled merged tree (as the vec `node_bounds` reads).
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant, min_frames: u32) -> Vec<TreeUpdate> {
    let mut frame = 0;
    loop {
        frame += 1;
        *at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, *at);
        if frame >= min_frames
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
    runtime.accessibility_tree().into_iter().collect()
}

/// The bounds the settled tree carries for the node with `role`
/// derived-labelled `label` — `None` when the node is absent.
fn node_bounds(updates: &[TreeUpdate], role: Role, label: &str) -> Option<Rect> {
    updates.iter().rev().find_map(|update| {
        update.nodes.iter().find_map(|(_, node)| {
            if node.role() == role && node.label() == Some(label) {
                node.bounds()
            } else {
                None
            }
        })
    })
}

/// The issue's reproduction: a one-row list whose label size flips between 20
/// and 160 after mount.
fn size_driven_list(tall: &Binding<bool>, label: &'static str) -> AnyViewBuilder<AnyView> {
    let tall = tall.clone();
    let rows = Binding::container(vec![SelfId::new(1_u64)]);
    AnyViewBuilder::<AnyView>::new(move || {
        let size = tall
            .clone()
            .map(move |tall| if tall { TALL_SIZE } else { SHORT_SIZE });
        AnyView::new(List::for_each(
            SignalCollection::new(rows.clone()),
            move |_| ListItem::new(text(label).size(size.clone())),
        ))
    })
}

/// The issue's grow half: after `tall` flips, the row's frame must grow to
/// hold the content and the label must not be clipped.
#[test]
fn list_row_remeasures_when_content_size_grows() {
    let tall = Binding::bool(false);
    let mut runtime = runtime(size_driven_list(&tall, "growing row"));
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    let initial =
        node_bounds(&updates, Role::ListItem, "growing row").expect("the row must publish bounds");
    assert!(
        (initial.height() - ROW_HEIGHT).abs() < 2.0,
        "a 20pt label sits at the one-line row minimum (got {:.1})",
        initial.height()
    );

    tall.set(true);
    let updates = settle(&mut runtime, &mut at, 4);
    let grown = node_bounds(&updates, Role::ListItem, "growing row")
        .expect("the grown row must publish bounds");
    let label = node_bounds(&updates, Role::Label, "growing row")
        .expect("the grown row's label must publish bounds");
    assert!(
        grown.height() > GROWN_THRESHOLD,
        "the row's frame must grow to hold its 160pt content (got {:.1})",
        grown.height()
    );
    assert!(
        label.height() > GROWN_THRESHOLD && label.y1 <= grown.y1 + 0.5,
        "the label must not be clipped into the stale slot \
         (label {:.1}pt tall, bottom {:.1}, row bottom {:.1})",
        label.height(),
        label.y1,
        grown.y1
    );
}

/// The shrink half: a row whose content re-measures smaller must give the
/// extent back instead of pinning the list at the old height.
#[test]
fn list_row_remeasures_when_content_size_shrinks() {
    let tall = Binding::bool(true);
    let mut runtime = runtime(size_driven_list(&tall, "shrinking row"));
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    let initial = node_bounds(&updates, Role::ListItem, "shrinking row")
        .expect("the row must publish bounds");
    assert!(
        initial.height() > GROWN_THRESHOLD,
        "a 160pt label's row must start grown (got {:.1})",
        initial.height()
    );

    tall.set(false);
    let updates = settle(&mut runtime, &mut at, 4);
    let shrunk = node_bounds(&updates, Role::ListItem, "shrinking row")
        .expect("the shrunk row must publish bounds");
    let label = node_bounds(&updates, Role::Label, "shrinking row")
        .expect("the shrunk row's label must publish bounds");
    assert!(
        (shrunk.height() - ROW_HEIGHT).abs() < 2.0,
        "the row's frame must shrink back to the one-line minimum (got {:.1})",
        shrunk.height()
    );
    assert!(
        label.y1 <= shrunk.y1 + 0.5,
        "the label must stay inside the shrunk slot (label bottom {:.1}, row bottom {:.1})",
        label.y1,
        shrunk.y1
    );
}

/// The anchor half: scrolled so `ANCHOR` is the top row, growing a row above
/// the viewport must leave `ANCHOR`'s on-screen position untouched.
#[test]
fn growing_row_above_viewport_keeps_anchor_row_position() {
    // Enough rows below the anchor that `offset_of(ANCHOR)` is a reachable
    // scroll offset: the viewport can only top-align a row while at least a
    // viewport's worth of content remains below it.
    const ROWS: usize = 45;
    const ANCHOR: usize = 30;
    const GROWN: usize = ANCHOR - 1;

    let grow = Binding::bool(false);
    let controller = ScrollController::new(0usize);
    let builder = {
        let grow = grow.clone();
        let controller = controller.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let grow = grow.clone();
            AnyView::new(
                List::for_each(SignalCollection::new(rows), move |row| {
                    let index = row.into_inner();
                    let size = grow.clone().map(move |grow| {
                        if grow && index == GROWN {
                            TALL_SIZE
                        } else {
                            SHORT_SIZE
                        }
                    });
                    ListItem::new(text(format!("Row {index}")).size(size))
                })
                .scroll_controller(&controller),
            )
        })
    };
    let mut runtime = runtime(builder);
    let mut at = Instant::now();
    let _ = settle(&mut runtime, &mut at, 4);

    controller.scroll_to(ANCHOR);
    // The jump lands within a couple of frames, but the target window still
    // has to materialize and measure; give it a generous floor rather than
    // trusting the settled probe.
    let updates = settle(&mut runtime, &mut at, 150);
    let before = node_bounds(&updates, Role::ListItem, "Row 30")
        .expect("the anchor row must publish bounds once scrolled to");
    assert!(
        before.y0.abs() < 1.0,
        "row {ANCHOR} must rest at the viewport top (got {:.1})",
        before.y0
    );

    grow.set(true);
    let updates = settle(&mut runtime, &mut at, 4);
    let after = node_bounds(&updates, Role::ListItem, "Row 30")
        .expect("the anchor row must keep publishing bounds");
    assert!(
        (after.y0 - before.y0).abs() < 0.5,
        "a row growing above the viewport must not move the anchor row \
         (y0 {:.1} -> {:.1})",
        before.y0,
        after.y0
    );
}
