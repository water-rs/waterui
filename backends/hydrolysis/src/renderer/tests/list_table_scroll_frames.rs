//! `List` and `Table` scroll frames ride the layer scroll offset.
//!
//! A `List` or `Table` scroll moves the widget's inner-layer scroll offset —
//! the same write a `scroll` node's content takes (water-rs/waterui#1910) — so
//! a scroll frame materializes only the rows a pan newly reveals and leaves
//! the rest of the visible window untouched: the frame-work counters show no
//! view-body dispatches and no layer mounts for the rows that stayed, the
//! layout pass is pure cache replay, and the only presentation delta is the
//! one scroll-offset property the accessibility tree reports back.

use core::time::Duration;
use std::time::Instant;

use accesskit::Role;
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::table::{Table, col};
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_core::views::ForEach;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, TouchPhase};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const ROWS: usize = 200;
const COLUMNS: usize = 2;

fn list_runtime() -> HeadlessRuntime {
    let rows = Binding::container((0..ROWS).map(SelfId::new).collect::<Vec<_>>());
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(List::for_each(SignalCollection::new(rows.clone()), |id| {
            let index = id.into_inner();
            ListItem::new(text(format!("Row {index}")))
        }))
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

fn table_runtime() -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(Table::new(nami::Computed::constant(vec![
            col(
                "A",
                ForEach::new(SignalCollection::new(rows.clone()), |id| {
                    text(format!("A{}", id.into_inner()))
                }),
            ),
            col(
                "B",
                ForEach::new(SignalCollection::new(rows), |id| {
                    text(format!("B{}", id.into_inner()))
                }),
            ),
        ])))
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// The row pitch of the emitted `ListItem` nodes, measured on a frame's
/// accessibility update from the first two rows' window-space origins.
fn row_pitch(result: &crate::HeadlessPumpResult) -> f64 {
    let mut items = result
        .tree_update
        .as_ref()
        .expect("a list frame must publish an accessibility update")
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            (node.role() == Role::ListItem).then(|| node.bounds().map(|rect| rect.y0))?
        })
        .collect::<Vec<_>>();
    items.sort_by(f64::total_cmp);
    items
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .find(|pitch| *pitch > 1.0)
        .expect("a resting list frame must emit at least two rows")
}

/// The `(index, window-space bounds)` of every `Row {i}` item a frame's
/// accessibility update emits, in index order.
fn row_items(result: &crate::HeadlessPumpResult) -> Vec<(usize, accesskit::Rect)> {
    let mut items = result
        .tree_update
        .as_ref()
        .expect("a list frame must publish an accessibility update")
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            if node.role() != Role::ListItem {
                return None;
            }
            let index = node
                .label()
                .and_then(|label| label.strip_prefix("Row "))
                .and_then(|index| index.parse::<usize>().ok())?;
            node.bounds().map(|rect| (index, rect))
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|(index, _)| *index);
    items
}

fn pan(runtime: &mut HeadlessRuntime, dy: f32) {
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: crate::num_cast::u32_as_f32(WINDOW_WIDTH) / 2.0,
        y: crate::num_cast::u32_as_f32(WINDOW_HEIGHT) / 2.0,
        dx: 0.0,
        dy,
        phase: TouchPhase::Moved,
    });
}

fn scroll_y(result: &crate::HeadlessPumpResult) -> f64 {
    result
        .tree_update
        .as_ref()
        .expect("a scroll frame must publish an accessibility update")
        .nodes
        .iter()
        .find_map(|(_, node)| node.scroll_y())
        .expect("a scrolling widget must publish its vertical offset")
}

const HEADER_HEIGHT: f64 = 100.0;

/// A `List` below a fixed-height sibling: the widget's frame sits away from
/// the window origin, so the scroll path's record-space math — rows at
/// `bounds.origin + content`, the visible window in content space — is
/// exercised through a real placement chain instead of the degenerate
/// full-window case (water-rs/waterui#1910 review).
fn headered_list_runtime() -> HeadlessRuntime {
    let rows = Binding::container((0..ROWS).map(SelfId::new).collect::<Vec<_>>());
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((
            ().size(0.0, crate::num_cast::f64_as_f32(HEADER_HEIGHT)),
            List::for_each(SignalCollection::new(rows.clone()), |id| {
                let index = id.into_inner();
                ListItem::new(text(format!("Row {index}")))
            }),
        )))
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// The window-bounded work any scroll frame may do: nothing reveals more than
/// one viewport of fresh rows, however far the pan travelled.
const MAX_REVEALED_PER_FRAME: u64 = 64;
/// Extent tracking re-measures the visible rows once each per frame — a
/// scroll frame's misses stay inside one viewport of cells, not one per row
/// of the dataset.
const MAX_VISIBLE_WINDOW: u32 = 96;

#[test]
fn a_list_scroll_frame_records_only_newly_revealed_rows() {
    let mut runtime = list_runtime();
    let start = Instant::now();
    let first = runtime.pump_at(false, start);
    let resting = scroll_y(&first);

    // A sub-row pan reveals nothing new: the frame is the one scroll-offset
    // write, with zero rows recorded or mounted.
    pan(&mut runtime, -10.0);
    let nudged = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        scroll_y(&nudged) > resting,
        "the pan must present the new scroll offset"
    );
    let work = nudged.profile.counters.frame_work;
    assert_eq!(
        work.semantic_builds, 0,
        "a pan revealing no rows must record no row"
    );
    assert_eq!(
        work.layer_creations, 0,
        "a pan revealing no rows must mount no layer"
    );
    assert_eq!(work.structural_patches, 0);
    assert_eq!(work.layer_removals, 0);
    // Row extents are re-resolved for the visible window every frame (that is
    // the list's extent bookkeeping, not scroll work), so the misses bound to
    // the window — they must not scale with how far the pan travelled.
    assert!(
        nudged.profile.counters.measurement_cache_misses <= MAX_VISIBLE_WINDOW,
        "a scroll frame missed the measurement cache {} times — more than one viewport",
        nudged.profile.counters.measurement_cache_misses
    );
    assert_eq!(nudged.profile.counters.rebuild_iterations, 0);

    // A deeper pan records only the rows it reveals — never the whole visible
    // window and never a count that scales with the row total.
    let mut offset = scroll_y(&nudged);
    for frame in 2..=4 {
        pan(&mut runtime, -160.0);
        let panned = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        let presented = scroll_y(&panned);
        assert!(presented > offset, "every pan must present its new offset");
        offset = presented;
        let work = panned.profile.counters.frame_work;
        assert!(
            work.semantic_builds <= MAX_REVEALED_PER_FRAME,
            "a scroll frame recorded {} rows — more than one viewport of new rows",
            work.semantic_builds
        );
        assert!(
            work.layer_creations <= MAX_REVEALED_PER_FRAME,
            "a scroll frame mounted {} layers — more than one viewport of new rows",
            work.layer_creations
        );
        assert!(
            work.recorded_view_contents <= 96,
            "a scroll frame encoded {} view contents — bounded by the visible window, not {ROWS} rows",
            work.recorded_view_contents
        );
        assert_eq!(work.structural_patches, 0);
        assert_eq!(panned.profile.counters.rebuild_iterations, 0);
        assert!(
            panned.profile.counters.measurement_cache_misses <= MAX_VISIBLE_WINDOW,
            "a scroll frame missed the measurement cache {} times — more than one viewport",
            panned.profile.counters.measurement_cache_misses
        );
    }
}

#[test]
fn a_table_scroll_frame_records_only_newly_revealed_cells() {
    let mut runtime = table_runtime();
    let start = Instant::now();
    let first = runtime.pump_at(false, start);
    let resting = scroll_y(&first);

    // A sub-row pan can expose one straddling row — at most `COLUMNS` cells
    // materialize, not the visible window again.
    pan(&mut runtime, -10.0);
    let nudged = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        scroll_y(&nudged) > resting,
        "the pan must present the new scroll offset"
    );
    let work = nudged.profile.counters.frame_work;
    assert!(
        work.semantic_builds <= COLUMNS as u64,
        "a pan revealing at most one row recorded {} cells",
        work.semantic_builds
    );
    assert!(
        work.layer_creations <= COLUMNS as u64,
        "a pan revealing at most one row mounted {} layers",
        work.layer_creations
    );
    assert_eq!(work.structural_patches, 0);
    assert!(
        nudged.profile.counters.measurement_cache_misses <= MAX_VISIBLE_WINDOW,
        "a scroll frame missed the measurement cache {} times — more than one viewport",
        nudged.profile.counters.measurement_cache_misses
    );
    assert_eq!(nudged.profile.counters.rebuild_iterations, 0);

    let mut offset = scroll_y(&nudged);
    for frame in 2..=4 {
        pan(&mut runtime, -160.0);
        let panned = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        let presented = scroll_y(&panned);
        assert!(presented > offset, "every pan must present its new offset");
        offset = presented;
        let work = panned.profile.counters.frame_work;
        assert!(
            work.semantic_builds <= MAX_REVEALED_PER_FRAME,
            "a scroll frame recorded {} cells — more than one viewport of new rows",
            work.semantic_builds
        );
        assert!(
            work.layer_creations <= MAX_REVEALED_PER_FRAME,
            "a scroll frame mounted {} layers — more than one viewport of new cells",
            work.layer_creations
        );
        assert!(
            work.recorded_view_contents <= 96,
            "a scroll frame encoded {} view contents — bounded by the visible window, not {ROWS} rows",
            work.recorded_view_contents
        );
        assert_eq!(work.structural_patches, 0);
        assert_eq!(panned.profile.counters.rebuild_iterations, 0);
        assert!(
            panned.profile.counters.measurement_cache_misses <= MAX_VISIBLE_WINDOW,
            "a scroll frame missed the measurement cache {} times — more than one viewport",
            panned.profile.counters.measurement_cache_misses
        );
    }
}

#[test]
fn a_list_below_a_header_scrolls_against_the_intersecting_window() {
    let mut runtime = headered_list_runtime();
    let start = Instant::now();
    let first = runtime.pump_at(false, start);
    let resting = scroll_y(&first);

    // The list's viewport is the window band below the header; the resting
    // frame emits exactly the rows inside it.
    let pitch = row_pitch(&first);
    let list_height = f64::from(WINDOW_HEIGHT) - HEADER_HEIGHT;
    let resting_items = row_items(&first);
    assert_eq!(
        resting_items.first().map(|(index, _)| *index),
        Some(0),
        "the resting window must start at Row 0"
    );
    assert_eq!(
        resting_items.last().map(|(index, _)| *index),
        Some(crate::num_cast::f64_as_usize((list_height / pitch).ceil()) - 1),
        "the resting window must end at the last row the viewport intersects"
    );

    // A sub-row pan reveals nothing new: the same one-write scroll frame as
    // the unshifted list's.
    pan(&mut runtime, -10.0);
    let nudged = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        scroll_y(&nudged) > resting,
        "the pan must present the new scroll offset"
    );
    let work = nudged.profile.counters.frame_work;
    assert_eq!(
        work.semantic_builds, 0,
        "a pan revealing no rows must record no row"
    );
    assert_eq!(
        work.layer_creations, 0,
        "a pan revealing no rows must mount no layer"
    );
    assert_eq!(work.structural_patches, 0);
    assert_eq!(work.layer_removals, 0);
    assert_eq!(nudged.profile.counters.rebuild_iterations, 0);

    // A deep pan through the shifted viewport: the emitted rows are exactly
    // the ones whose content-space band intersects it — `offset` in content
    // space, the window `[offset, offset + viewport]` — and their published
    // bounds land inside the list's frame, not the header's.
    pan(&mut runtime, -160.0);
    let panned = runtime.pump_at(false, start + Duration::from_millis(32));
    let offset = scroll_y(&panned);
    assert!(
        offset > resting + 160.0,
        "the pan must accumulate: {offset}"
    );
    let expected_first = crate::num_cast::f64_as_usize((offset / pitch).floor());
    let expected_last = crate::num_cast::f64_as_usize(((offset + list_height) / pitch).ceil()) - 1;
    let items = row_items(&panned);
    assert_eq!(
        items.first().map(|(index, _)| *index),
        Some(expected_first),
        "the first emitted row must be the one straddling the viewport top"
    );
    assert_eq!(
        items.last().map(|(index, _)| *index),
        Some(expected_last),
        "the last emitted row must be the one straddling the viewport bottom"
    );
    for (index, rect) in &items {
        assert!(
            rect.y1 > HEADER_HEIGHT && rect.y0 < f64::from(WINDOW_HEIGHT),
            "Row {index} emitted at {rect:?} — outside the list's viewport band"
        );
    }
}
