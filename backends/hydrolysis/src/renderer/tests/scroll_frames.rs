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

use waterui::ViewExt as _;
use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{VStack, vstack};

use super::test_environment;
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
    HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT)
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
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT);
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
