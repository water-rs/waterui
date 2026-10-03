//! Scroll-input frame economy.
//!
//! A scroll offset change schedules the one per-frame pass — patch, layout,
//! re-encode — and must present the new offset (scene, hit-test geometry,
//! accessibility tree) on the very frame that consumed it, never the previous
//! frame's scene unchanged. Layout runs every frame, but over an unchanged
//! tree it must be pure cache replay: no re-measurement and no window
//! rebuild. These tests drive the real runner input path
//! (`handle_input_events` → frame pump) headlessly and read the flushed
//! accessibility tree as the observable output.

use core::time::Duration;
use std::time::Instant;

use nami::Binding;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{VStack, vstack};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind, TouchPhase};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const ROW_HEIGHT: f32 = 44.0;
const ROWS: usize = 40;

/// A vertically-scrolling stack of labeled fixed-height rows, taller than the
/// viewport — the shape of a plain (non-virtualized) form screen.
fn labeled_rows() -> AnyView {
    let data = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
    AnyView::new(scroll(VStack::for_each(data, |row| {
        let index = row.into_inner();
        vstack((text(format!("Row {index}")),)).size(360.0, ROW_HEIGHT)
    })))
}

fn runtime() -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(labeled_rows);
    let env = test_environment();
    HeadlessRuntime::new_for_tests(
        env,
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

fn scroll_y(result: &crate::HeadlessPumpResult) -> f64 {
    result
        .tree_update
        .as_ref()
        .expect("scroll frame must publish an accessibility update")
        .nodes
        .iter()
        .find_map(|(_, node)| node.scroll_y())
        .expect("scroll view must publish its vertical offset")
}

#[test]
fn trackpad_pan_presents_the_new_offset_without_re_measuring() {
    let mut runtime = runtime();
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -120.0,
        phase: TouchPhase::Moved,
    });
    let panned = runtime.pump_at(false, start + Duration::from_millis(16));

    assert!(
        (scroll_y(&panned) - 120.0).abs() < 0.5,
        "a trackpad pan must re-encode the retained tree at the new offset \
         (got {}, want 120)",
        scroll_y(&panned)
    );
    assert_eq!(
        (
            panned.profile.counters.measurement_cache_hits,
            panned.profile.counters.measurement_cache_misses,
        ),
        (0, 0),
        "a pan over an unchanged tree must be pure cache replay and not re-measure"
    );
    assert_eq!(
        panned.profile.counters.rebuild_iterations, 0,
        "a pan must never rebuild the window"
    );
}

#[test]
fn wheel_ticks_glide_without_re_measuring() {
    let mut runtime = runtime();
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(InputEvent::Scroll {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -2.0,
        is_line_delta: true,
    });
    let mut last_offset = 0.0;
    for frame in 1..=30u64 {
        let result = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        assert_eq!(
            (
                result.profile.counters.measurement_cache_hits,
                result.profile.counters.measurement_cache_misses,
            ),
            (0, 0),
            "wheel glide frame {frame} over an unchanged tree must not re-measure"
        );
        if let Some(update) = &result.tree_update
            && let Some(offset) = update.nodes.iter().find_map(|(_, node)| node.scroll_y())
        {
            last_offset = offset;
        }
    }
    assert!(
        (last_offset - 80.0).abs() < 0.5,
        "two wheel ticks must glide to their 80px target (got {last_offset})"
    );
}

#[test]
fn pan_over_lazy_content_materializes_entering_rows() {
    const LAZY_ROWS: usize = 500;
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        let data = (0..LAZY_ROWS).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(scroll(VStack::for_each(data, |row| {
            let index = row.into_inner();
            vstack((text(format!("Row {index}")),)).size(360.0, ROW_HEIGHT)
        })))
    });
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    // Pan 50 rows deep in one gesture; the reencode flush must resolve the new
    // visible window and materialize the entering rows.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -(ROW_HEIGHT * 50.0),
        phase: TouchPhase::Moved,
    });
    let panned = runtime.pump_at(false, start + Duration::from_millis(16));
    let update = panned
        .tree_update
        .as_ref()
        .expect("a lazy pan frame must publish an accessibility update");
    assert!(
        (scroll_y(&panned) - f64::from(ROW_HEIGHT * 50.0)).abs() < 0.5,
        "a pan over a lazy stack must present the new offset (got {})",
        scroll_y(&panned)
    );
    // The stack was built with only the first viewport of rows materialized;
    // a row 50 pitches deep can only appear if the scroll frame itself resolved
    // and materialized the entering window (row pitch = height + spacing, so
    // the exact window start depends on the stack's default spacing — row 50
    // is inside the panned viewport for any plausible spacing).
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Row 50")),
        "rows entering the panned viewport must be materialized by the scroll frame"
    );
}

fn scroll_extents(result: &crate::HeadlessPumpResult) -> (f64, f64) {
    result
        .tree_update
        .as_ref()
        .expect("scroll frame must publish an accessibility update")
        .nodes
        .iter()
        .find_map(|(_, node)| Some((node.scroll_y()?, node.scroll_y_max()?)))
        .expect("scroll view must publish its vertical offset and extent")
}

#[test]
fn scrollbar_gutter_drag_maps_track_position_to_offset() {
    let mut runtime = runtime();
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    // Press deep in the scrollbar track: the thumb jumps to the pointer, which
    // clamps to the end of the scrollable range.
    let gutter_x = WINDOW_WIDTH as f32 - 4.0;
    runtime.push_input_event(InputEvent::PointerDown {
        id: 1,
        kind: PointerKind::Mouse,
        x: gutter_x,
        y: WINDOW_HEIGHT as f32 - 2.0,
        button: PointerButton::Primary,
    });
    let pressed = runtime.pump_at(false, start + Duration::from_millis(16));
    let (offset, max) = scroll_extents(&pressed);
    assert!(max > 0.0, "the stack must overflow the viewport");
    assert!(
        (offset - max).abs() < 0.5,
        "pressing the end of the scrollbar track must jump to the end of the \
         range (got {offset}, max {max})"
    );

    // Drag the thumb back to the top of the track.
    runtime.push_input_event(InputEvent::PointerMove {
        id: 1,
        kind: PointerKind::Mouse,
        x: gutter_x,
        y: 0.0,
    });
    let dragged = runtime.pump_at(false, start + Duration::from_millis(32));
    let (offset, _) = scroll_extents(&dragged);
    assert!(
        offset < 0.5,
        "dragging the thumb to the top of the track must scroll home (got {offset})"
    );
    assert_eq!(
        (
            dragged.profile.counters.measurement_cache_hits,
            dragged.profile.counters.measurement_cache_misses,
        ),
        (0, 0),
        "a scrollbar drag over an unchanged tree must not re-measure"
    );

    runtime.push_input_event(InputEvent::PointerUp {
        id: 1,
        kind: PointerKind::Mouse,
        x: gutter_x,
        y: 0.0,
        button: PointerButton::Primary,
    });
    let released = runtime.pump_at(false, start + Duration::from_millis(48));
    let (offset, _) = scroll_extents(&released);
    assert!(
        offset < 0.5,
        "releasing the thumb must keep the dragged offset (got {offset})"
    );
}

#[test]
fn momentum_tail_presents_every_consumed_delta() {
    // A trackpad momentum tail is a stream of pixel deltas that decay toward
    // zero while staying well above the scroll epsilon. Every one of them must
    // reach the screen on the frame that consumed it: a tail whose small
    // deltas accumulate silently and surface later as a jump is exactly the
    // "momentum end feels janky" defect class.
    let mut runtime = runtime();
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    let mut velocity = 30.0_f32;
    let mut expected = 0.0_f64;
    for frame in 1..=120_u64 {
        velocity *= 0.95;
        let dy = velocity.max(0.05);
        runtime.push_input_event(InputEvent::TrackpadPan {
            x: WINDOW_WIDTH as f32 / 2.0,
            y: WINDOW_HEIGHT as f32 / 2.0,
            dx: 0.0,
            dy: -dy,
            phase: TouchPhase::Moved,
        });
        expected += f64::from(dy);
        let result = runtime.pump_at(false, start + Duration::from_millis(frame * 8));
        let presented = scroll_y(&result);
        assert!(
            (presented - expected).abs() < 0.1,
            "momentum frame {frame} must present the accumulated offset \
             (got {presented}, want {expected}, delta this frame {dy})"
        );
    }
}

#[test]
fn inset_lazy_stack_materializes_the_visible_rows_after_pan() {
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        let data = (0..100).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(scroll(
            vstack((
                text("Header").height(300.0),
                VStack::for_each(data, |row| {
                    text(format!("Inset row {}", row.into_inner())).height(44.0)
                })
                .spacing(0.0),
            ))
            .spacing(0.0),
        ))
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: 200.0,
        y: 320.0,
        dx: 0.0,
        dy: -700.0,
        phase: TouchPhase::Moved,
    });
    let frame = runtime.pump_at(false, start + Duration::from_millis(16));
    let update = frame
        .tree_update
        .as_ref()
        .expect("pan publishes visible rows");
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Inset row 10")),
        "a row below the header and inside the viewport must be materialized"
    );
}

/// Like [`scroll_y`], but `None` when the pump published no update at all — a
/// delta that changed nothing schedules no frame, and the quiet frame is the
/// expected outcome for a clamped edge.
fn scroll_y_opt(result: &crate::HeadlessPumpResult) -> Option<f64> {
    result
        .tree_update
        .as_ref()?
        .nodes
        .iter()
        .find_map(|(_, node)| node.scroll_y())
}

/// The offset a scroll container published in a pump's accessibility update,
/// found by the label it was given — `None` when no node carries both.
fn labeled_scroll_offset(result: &crate::HeadlessPumpResult, label: &str) -> Option<f64> {
    result
        .tree_update
        .as_ref()?
        .nodes
        .iter()
        .find_map(|(_, node)| {
            if node.label() == Some(label) {
                node.scroll_y()
            } else {
                None
            }
        })
}

/// The scrollable extent (`scroll_y_max`) published alongside
/// [`labeled_scroll_offset`].
fn labeled_scroll_max(result: &crate::HeadlessPumpResult, label: &str) -> Option<f64> {
    result
        .tree_update
        .as_ref()?
        .nodes
        .iter()
        .find_map(|(_, node)| {
            if node.label() == Some(label) {
                node.scroll_y_max()
            } else {
                None
            }
        })
}

#[test]
fn wheel_and_pan_deltas_clamp_the_offset_at_both_ends() {
    let mut runtime = runtime();
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    // A pixel delta far past the bottom clamps at the scrollable extent.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -10_000.0,
        phase: TouchPhase::Moved,
    });
    let panned = runtime.pump_at(false, start + Duration::from_millis(16));
    let (offset, max) = scroll_extents(&panned);
    assert!(max > 0.0, "the content must overflow the viewport");
    assert!(
        (offset - max).abs() < 0.5,
        "a pixel delta past the bottom must clamp at the extent \
         (got {offset}, max {max})"
    );

    // Pushing further changes nothing: the offset is already the end.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -10.0,
        phase: TouchPhase::Moved,
    });
    let pinned = runtime.pump_at(false, start + Duration::from_millis(32));
    let pinned_offset = scroll_y_opt(&pinned).unwrap_or(max);
    assert!(
        (pinned_offset - max).abs() < 0.5,
        "a delta at the bottom edge must not overscroll (got {pinned_offset})"
    );

    // Back past the top clamps at zero the same way.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: 10_000.0,
        phase: TouchPhase::Moved,
    });
    let home = runtime.pump_at(false, start + Duration::from_millis(48));
    assert!(
        scroll_y(&home).abs() < 0.5,
        "a delta past the top must clamp at zero (got {})",
        scroll_y(&home)
    );

    // Line deltas take the same clamps through the smooth-scroll target.
    runtime.push_input_event(InputEvent::Scroll {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -1_000.0,
        is_line_delta: true,
    });
    let mut glided = 0.0;
    for frame in 4..=60u64 {
        let result = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        if let Some(update) = &result.tree_update
            && let Some(offset) = update.nodes.iter().find_map(|(_, node)| node.scroll_y())
        {
            glided = offset;
        }
    }
    assert!(
        (glided - max).abs() < 0.5,
        "repeated wheel ticks must glide to and clamp at the extent \
         (got {glided}, max {max})"
    );
}

/// An inner `scroll` (200pt tall) inside a taller outer `scroll` — a scrolling
/// card inside a scrolling page. Both overflow their viewports, so either one
/// could consume a wheel delta.
fn nested_scrolls(inner_label: &'static str, outer_label: &'static str) -> AnyViewBuilder<AnyView> {
    let inner_rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
    AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(
            scroll(
                vstack((
                    scroll(VStack::for_each(inner_rows.clone(), |row| {
                        let index = row.into_inner();
                        vstack((text(format!("Inner {index}")),)).size(360.0, ROW_HEIGHT)
                    }))
                    .height(200.0)
                    .a11y_label(inner_label),
                    // A tall filler below the inner scroll gives the outer one
                    // a scrollable extent of its own.
                    ().size(360.0, 1_200.0),
                ))
                .spacing(0.0),
            )
            .a11y_label(outer_label),
        )
    })
}

#[test]
fn a_nested_scroll_consumes_the_delta_until_it_hits_its_edge() {
    let builder = nested_scrolls("inner scroll", "outer scroll");
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    // A pan over the inner viewport moves the inner scroll only.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: -80.0,
        phase: TouchPhase::Moved,
    });
    let panned = runtime.pump_at(false, start + Duration::from_millis(16));
    let inner = labeled_scroll_offset(&panned, "inner scroll")
        .expect("the inner scroll must publish an offset");
    let outer = labeled_scroll_offset(&panned, "outer scroll")
        .expect("the outer scroll must publish an offset");
    assert!(
        (inner - 80.0).abs() < 0.5,
        "a pan over the inner scroll must scroll the inner (got inner {inner}, outer {outer})"
    );
    assert!(
        outer.abs() < 0.5,
        "the outer scroll must not move while the inner can consume (got {outer})"
    );

    // Driving the inner far past its end clamps it; the delta that reached the
    // edge is still consumed whole — nothing spills into the outer mid-gesture.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: -4_000.0,
        phase: TouchPhase::Moved,
    });
    let at_edge = runtime.pump_at(false, start + Duration::from_millis(32));
    let inner = labeled_scroll_offset(&at_edge, "inner scroll").unwrap();
    let inner_max = labeled_scroll_max(&at_edge, "inner scroll").unwrap();
    let outer = labeled_scroll_offset(&at_edge, "outer scroll").unwrap();
    assert!(
        (inner - inner_max).abs() < 0.5,
        "the inner scroll must clamp at its own edge (got {inner}, max {inner_max})"
    );
    assert!(
        outer.abs() < 0.5,
        "the edge-reaching delta must not spill into the outer scroll (got {outer})"
    );

    // At its edge the inner cannot consume: the next delta falls through to
    // the enclosing scroll — nested scrolling, not a dead zone.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: -60.0,
        phase: TouchPhase::Moved,
    });
    let fell_through = runtime.pump_at(false, start + Duration::from_millis(48));
    let inner = labeled_scroll_offset(&fell_through, "inner scroll").unwrap();
    let outer = labeled_scroll_offset(&fell_through, "outer scroll").unwrap();
    assert!(
        (inner - inner_max).abs() < 0.5,
        "the inner scroll must stay pinned at its edge (got {inner})"
    );
    assert!(
        (outer - 60.0).abs() < 0.5,
        "a delta the inner cannot consume must fall through to the outer (got {outer})"
    );

    // Scrolling back up resumes with the inner: the outer keeps its offset.
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: 50.0,
        phase: TouchPhase::Moved,
    });
    let rebound = runtime.pump_at(false, start + Duration::from_millis(64));
    let inner = labeled_scroll_offset(&rebound, "inner scroll").unwrap();
    let outer = labeled_scroll_offset(&rebound, "outer scroll").unwrap();
    assert!(
        (inner - (inner_max - 50.0)).abs() < 0.5,
        "scrolling back up must resume on the inner (got inner {inner}, max {inner_max})"
    );
    assert!(
        (outer - 60.0).abs() < 0.5,
        "the outer must keep its offset while the inner scrolls back (got {outer})"
    );
}

/// A `List` pinned inside a `scroll` is the same nested scroller: the list —
/// which registers its scroll target the same way — consumes the delta until
/// its own edge, then the outer scroll takes over.
#[test]
fn a_list_inside_a_scroll_consumes_the_delta_until_its_edge() {
    let rows = Binding::container((0..ROWS as u64).map(SelfId::new).collect::<Vec<_>>());
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(
            scroll(
                vstack((
                    List::for_each(SignalCollection::new(rows.clone()), |id| {
                        let index = id.into_inner();
                        ListItem::new(text(format!("Row {index}")))
                    })
                    .height(240.0)
                    .a11y_label("inner list"),
                    ().size(360.0, 1_200.0),
                ))
                .spacing(0.0),
            )
            .a11y_label("outer scroll"),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: -80.0,
        phase: TouchPhase::Moved,
    });
    let panned = runtime.pump_at(false, start + Duration::from_millis(16));
    let list = labeled_scroll_offset(&panned, "inner list")
        .expect("the inner list must publish an offset");
    let outer = labeled_scroll_offset(&panned, "outer scroll")
        .expect("the outer scroll must publish an offset");
    assert!(
        (list - 80.0).abs() < 0.5,
        "a pan over the list must scroll the list (got list {list}, outer {outer})"
    );
    assert!(
        outer.abs() < 0.5,
        "the outer scroll must not move while the list can consume (got {outer})"
    );

    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: -4_000.0,
        phase: TouchPhase::Moved,
    });
    let _ = runtime.pump_at(false, start + Duration::from_millis(32));
    runtime.push_input_event(InputEvent::TrackpadPan {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: 100.0,
        dx: 0.0,
        dy: -60.0,
        phase: TouchPhase::Moved,
    });
    let fell_through = runtime.pump_at(false, start + Duration::from_millis(48));
    let list = labeled_scroll_offset(&fell_through, "inner list").unwrap();
    let list_max = labeled_scroll_max(&fell_through, "inner list").unwrap();
    let outer = labeled_scroll_offset(&fell_through, "outer scroll").unwrap();
    assert!(
        (list - list_max).abs() < 0.5,
        "the inner list must stay pinned at its edge (got {list}, max {list_max})"
    );
    assert!(
        (outer - 60.0).abs() < 0.5,
        "a delta the list cannot consume must fall through to the outer (got {outer})"
    );
}
