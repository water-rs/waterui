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
use cherenkov_record::{Draw as _, Paint, WorkingColor};
use nami::collection::SignalCollection;
use nami::{Binding, Computed, SignalExt as _};
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::component::list::List;
use waterui::component::table::{Table, col};
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, TouchPhase};
use crate::testing::plain_chrome;
use crate::theme::{Custom, ListControls, ListGestures, ListLook, ListMetrics};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const ROWS: usize = 200;
const COLUMNS: usize = 2;

fn list_runtime() -> HeadlessRuntime {
    let rows = Binding::container((0..ROWS).map(SelfId::new).collect::<Vec<_>>());
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(List::for_each(SignalCollection::new(rows.clone()), |id| {
            let index = id.into_inner();
            text(format!("Row {index}"))
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
        AnyView::new(Table::new(
            SignalCollection::new(rows),
            (
                col("A", |id: &SelfId<usize>| {
                    text(format!("A{}", (*id).into_inner()))
                }),
                col("B", |id: &SelfId<usize>| {
                    text(format!("B{}", (*id).into_inner()))
                }),
            ),
        ))
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
        .tree_update()
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
        .tree_update()
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
        .tree_update()
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
                text(format!("Row {index}"))
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
    let work = nudged.profile().counters().frame_work();
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
    assert_eq!(
        nudged.profile().counters().measurement_cache_misses(),
        0,
        "a pan revealing no rows must reuse every visible row's cached measurement"
    );
    assert_eq!(nudged.profile().counters().rebuild_iterations(), 0);

    // A deeper pan records only the rows it reveals — never the whole visible
    // window and never a count that scales with the row total.
    let mut offset = scroll_y(&nudged);
    for frame in 2..=4 {
        pan(&mut runtime, -160.0);
        let panned = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        let presented = scroll_y(&panned);
        assert!(presented > offset, "every pan must present its new offset");
        offset = presented;
        let work = panned.profile().counters().frame_work();
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
        assert_eq!(panned.profile().counters().rebuild_iterations(), 0);
        assert!(
            panned.profile().counters().measurement_cache_misses() <= MAX_VISIBLE_WINDOW,
            "a scroll frame missed the measurement cache {} times — more than one viewport",
            panned.profile().counters().measurement_cache_misses()
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
    let work = nudged.profile().counters().frame_work();
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
        nudged.profile().counters().measurement_cache_misses() <= MAX_VISIBLE_WINDOW,
        "a scroll frame missed the measurement cache {} times — more than one viewport",
        nudged.profile().counters().measurement_cache_misses()
    );
    assert_eq!(nudged.profile().counters().rebuild_iterations(), 0);

    let mut offset = scroll_y(&nudged);
    for frame in 2..=4 {
        pan(&mut runtime, -160.0);
        let panned = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        let presented = scroll_y(&panned);
        assert!(presented > offset, "every pan must present its new offset");
        offset = presented;
        let work = panned.profile().counters().frame_work();
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
        assert_eq!(panned.profile().counters().rebuild_iterations(), 0);
        assert!(
            panned.profile().counters().measurement_cache_misses() <= MAX_VISIBLE_WINDOW,
            "a scroll frame missed the measurement cache {} times — more than one viewport",
            panned.profile().counters().measurement_cache_misses()
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
    let work = nudged.profile().counters().frame_work();
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
    assert_eq!(nudged.profile().counters().rebuild_iterations(), 0);

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

fn decorated_list_look() -> ListLook {
    let row_bg = WorkingColor::new([0.95, 0.95, 0.97, 1.0]);
    let divider = WorkingColor::new([0.82, 0.82, 0.85, 1.0]);
    ListLook::new(
        ListMetrics {
            one_line_row_height: 56.0,
            horizontal_inset: 16.0,
            vertical_inset: 10.0,
            divider_leading_inset: 16.0,
            divider_trailing_inset: 16.0,
            move_control_width: 20.0,
            delete_control_width: 26.0,
            trailing_control_spacing: 6.0,
            trailing_control_vertical_inset: 6.0,
            section_header_height: 48.0,
            section_footer_height: 40.0,
        },
        ListGestures {
            swipe_threshold: 0.5,
            reorder_threshold: 0.5,
            slop: 8.0,
            settle: Animation::linear(Duration::from_millis(15)),
        },
        Custom::new((), move |recorder, frame, ()| {
            recorder.fill(frame, Paint::Solid(row_bg));
        }),
        Custom::new((), move |recorder, frame, ()| {
            recorder.fill(frame, Paint::Solid(divider));
        }),
        ListControls {
            move_up: plain_chrome(),
            move_down: plain_chrome(),
            delete: plain_chrome(),
        },
    )
}

#[test]
fn a_decorated_list_scroll_frame_reuses_row_measurements_and_run_layers() {
    let rows = Binding::container(
        (0..ROWS)
            .map(|i| SelfId::new(u64::try_from(i).expect("row index fits in u64")))
            .collect::<Vec<_>>(),
    );
    let selection = Binding::container(None::<u64>);
    let grow_row_zero = Binding::bool(false);
    let selection_for_list = selection.clone();
    let grow_for_rows = grow_row_zero.clone();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let grow_for_rows = grow_for_rows.clone();
        AnyView::new(
            List::for_each(SignalCollection::new(rows.clone()), move |id| {
                let index = id.into_inner();
                let height = if index == 0 {
                    grow_for_rows
                        .clone()
                        .map(|tall| if tall { 140.0 } else { 20.0 })
                        .computed()
                } else {
                    Computed::constant(20.0)
                };
                text(format!("Row {index}")).size(height)
            })
            .selection(&selection_for_list),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default().with_list_look(decorated_list_look()),
    );
    let start = Instant::now();
    let first = runtime.pump_at(false, start);
    let resting = scroll_y(&first);
    let initial_mount = runtime.mount_stats();
    assert!(
        initial_mount.installs >= 24,
        "initial decorated list frame must install row backgrounds and separators (got {})",
        initial_mount.installs
    );

    // Steady vertical scroll revealing no new rows: every visible row reuses
    // its cached measurement and its compact static background/separator runs.
    pan(&mut runtime, -10.0);
    let nudged = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        scroll_y(&nudged) > resting,
        "the pan must advance the scroll offset"
    );
    let work = nudged.profile().counters().frame_work();
    let scroll_mount = runtime.mount_stats();
    assert_eq!(
        nudged.profile().counters().measurement_cache_misses(),
        0,
        "steady vertical scroll must not re-measure unchanged visible rows"
    );
    assert_eq!(
        scroll_mount.installs, 0,
        "steady vertical scroll must not re-record unchanged compact static list-row RunLayers"
    );
    assert_eq!(
        scroll_mount.live_records, 0,
        "steady vertical scroll must not re-record live text layers for unchanged rows"
    );
    assert_eq!(work.semantic_builds, 0);
    assert_eq!(work.layer_creations, 0);
    assert_eq!(work.layer_removals, 0);

    // Changing selection on a visible row must invalidate only that row's
    // background RunLayer while the remaining visible rows stay cached.
    selection.set(Some(0));
    let selected = runtime.pump_at(false, start + Duration::from_millis(32));
    assert_eq!(
        runtime.mount_stats().installs,
        1,
        "selecting Row 0 must re-record only Row 0's background RunLayer"
    );

    // Changing a visible row's reactive content size must invalidate its cached
    // measurement and grow its published bounds (the first frame measures the
    // new extent and marks layout so the follow-up frame publishes it in a11y).
    let before_height = row_items(&selected)
        .into_iter()
        .find(|(index, _)| *index == 0)
        .expect("Row 0 must be visible")
        .1
        .height();
    grow_row_zero.set(true);
    let _ = runtime.pump_at(false, start + Duration::from_millis(48));
    let grown = runtime.pump_at(false, start + Duration::from_millis(64));
    let after_height = row_items(&grown)
        .into_iter()
        .find(|(index, _)| *index == 0)
        .expect("Row 0 must stay visible after growing")
        .1
        .height();
    assert!(
        after_height > before_height + 40.0,
        "reactive content growth must re-measure Row 0 (before={before_height:.1}, after={after_height:.1})"
    );
}
