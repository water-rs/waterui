//! water-rs/waterui#1684 — touch-drag scrolling and fling.
//!
//! A `PointerKind::Touch` press inside a scroll view still lands on the
//! content under it; a drag past the platform's touch slop lets the
//! innermost enclosing scroll view that can scroll along the dominant axis
//! claim the gesture — cancelling the content's press, never completing it
//! — and track one to one; a fast release flings and decelerates to rest.
//! These tests drive the real input path (`push_input_event` →
//! `pump_at`) with a `ViewConfiguration`-shaped
//! [`crate::platform::TouchScrollConfig`] on the headless host, and read
//! the scroll handle's metrics as the observable offset. Time advances
//! only through `pump_at`'s frame instant.

use core::time::Duration;
use std::time::Instant;

use nami::{Binding, Signal as _};
use waterui::ViewExt as _;
use waterui::{AnyView, component::text};
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::frame::Frame;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{VStack, vstack};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind, TouchScrollConfig};

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const ROW_HEIGHT: f32 = 44.0;
const ROWS: usize = 80;

/// `ViewConfiguration`-shaped values at density 1.0: a 10-unit slop,
/// Android's fling bounds, and the `OverScroller` physical coefficient a
/// density-1 display produces (`GRAVITY_EARTH * 39.37 * 160 * 0.84` px/s²,
/// in logical units at `ppi = 160`).
fn test_config() -> TouchScrollConfig {
    TouchScrollConfig::android_default()
}

fn runtime(view: AnyView) -> HeadlessRuntime {
    let view = std::cell::RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    runtime.set_touch_scroll_config(test_config());
    runtime
}

fn tall_stack() -> AnyView {
    let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
    AnyView::new(scroll(VStack::for_each(rows, |row| {
        let index = row.into_inner();
        vstack((text(format!("Row {index}")),)).size(360.0, ROW_HEIGHT)
    })))
}

fn touch_down(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerDown {
        id: 1,
        kind: PointerKind::Touch,
        x,
        y,
        button: PointerButton::Primary,
    }
}

fn touch_move(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerMove {
        id: 1,
        kind: PointerKind::Touch,
        x,
        y,
    }
}

fn touch_up(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerUp {
        id: 1,
        kind: PointerKind::Touch,
        x,
        y,
        button: PointerButton::Primary,
    }
}

/// The vertical offset of the scroll view under `point` — `None` while no
/// scroll target covers it.
fn offset_y_at(runtime: &HeadlessRuntime, x: f32, y: f32) -> Option<f64> {
    runtime
        .renderer()
        .scroll_metrics_at(x, y)
        .map(|m| m.offset_y)
}

/// A tall scroll whose content puts a full-width `button` under the touch
/// point the tests drive — sized so the press target's bounds cover it.
fn scroll_with_button(tapped: Binding<bool>) -> AnyView {
    AnyView::new(scroll(vstack((
        ().size(360.0, 260.0),
        Frame::new(button("tap me").action(move || tapped.set(true)))
            .width(360.0)
            .height(44.0),
        ().size(360.0, 1_200.0),
    ))))
}

#[test]
fn a_tap_inside_a_scroll_view_still_activates_the_content() {
    let tapped = Binding::container(false);
    let mut runtime = runtime(scroll_with_button(tapped.clone()));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    runtime.push_input_event(touch_up(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(50));

    assert!(
        tapped.snapshot(),
        "a tap inside a scroll view must still activate the content under it"
    );
    assert_eq!(
        offset_y_at(&runtime, 200.0, 280.0),
        Some(0.0),
        "a tap must not move the scroll view"
    );
}

#[test]
fn a_drag_past_the_slop_scrolls_and_never_activates_the_press() {
    let tapped = Binding::container(false);
    let mut runtime = runtime(scroll_with_button(tapped.clone()));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    // Past the 10-unit slop: the scroll view claims the gesture and the
    // crossing move applies its excess — the content follows the finger
    // one to one from where the slop was crossed.
    runtime.push_input_event(touch_move(200.0, 260.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(32));
    assert_eq!(
        offset_y_at(&runtime, 200.0, 280.0),
        Some(10.0),
        "the crossing move applies its excess over the slop (20 dragged - 10 slop)"
    );

    runtime.push_input_event(touch_move(200.0, 240.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(48));
    assert_eq!(
        offset_y_at(&runtime, 200.0, 280.0),
        Some(30.0),
        "the drag tracks one to one once claimed"
    );

    // A held, slow release settles without a fling: the earlier samples
    // age out of the velocity window.
    runtime.push_input_event(touch_up(200.0, 240.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(300));
    assert_eq!(
        offset_y_at(&runtime, 200.0, 280.0),
        Some(30.0),
        "a slow release must not fling"
    );

    assert!(
        !tapped.snapshot(),
        "a scroll-claimed press is cancelled, never completed"
    );
}

#[test]
fn a_horizontal_drag_does_not_scroll_a_vertical_only_scroll_view() {
    let mut runtime = runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    runtime.push_input_event(touch_move(160.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(32));
    runtime.push_input_event(touch_move(120.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(48));
    runtime.push_input_event(touch_up(120.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(64));

    assert_eq!(
        offset_y_at(&runtime, 200.0, 300.0),
        Some(0.0),
        "a vertical-only scroll view must not claim a horizontal drag"
    );
}

#[test]
fn a_nested_scroll_view_claims_the_drag_over_its_enclosing_parent() {
    let inner_rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
    let mut runtime = runtime(AnyView::new(scroll(
        vstack((
            scroll(VStack::for_each(inner_rows, |row| {
                let index = row.into_inner();
                vstack((text(format!("Inner {index}")),)).size(360.0, ROW_HEIGHT)
            }))
            .height(200.0),
            // A tall filler gives the outer scroll an extent of its own.
            ().size(360.0, 1_200.0),
        ))
        .spacing(0.0),
    )));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 100.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    runtime.push_input_event(touch_move(200.0, 80.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(32));
    runtime.push_input_event(touch_move(200.0, 60.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(48));
    runtime.push_input_event(touch_up(200.0, 60.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(300));

    assert_eq!(
        offset_y_at(&runtime, 200.0, 100.0),
        Some(30.0),
        "the inner scroll view claims a vertical drag over its parent"
    );
    assert_eq!(
        offset_y_at(&runtime, 200.0, 400.0),
        Some(0.0),
        "the outer scroll view must not move while the inner claims"
    );
}

#[test]
fn a_fast_release_flings_and_decelerates_to_rest_inside_the_bounds() {
    let mut runtime = runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 400.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    // -40 units per 16ms ≈ 2500 units/s of upward finger velocity —
    // comfortably over the minimum fling speed.
    for (frame_ms, y) in [(32_u64, 360.0_f32), (48, 320.0), (64, 280.0)] {
        runtime.push_input_event(touch_move(200.0, y));
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
    runtime.push_input_event(touch_up(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(80));

    let dragged =
        offset_y_at(&runtime, 200.0, 400.0).expect("the scroll view must publish its metrics");
    let max = runtime
        .renderer()
        .scroll_metrics_at(200.0, 400.0)
        .map(|m| m.max_y)
        .expect("the scroll view must publish its extent");
    assert!(max > 0.0, "the content must overflow the viewport");

    // The fling advances the offset over pumped frames and decelerates:
    // per-frame deltas shrink until the run settles inside the bounds.
    let mut offsets = vec![dragged];
    for frame in 6..=200u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(80 + frame * 16));
        offsets.push(
            offset_y_at(&runtime, 200.0, 400.0).expect("the scroll view must keep its metrics"),
        );
    }

    let first_delta = offsets[1] - offsets[0];
    assert!(
        first_delta > 0.0,
        "a fast release must fling (offset {dragged} must advance, got {:?})",
        &offsets[..6]
    );
    assert!(
        offsets.windows(2).all(|pair| pair[1] >= pair[0]),
        "a fling is monotonic toward its rest offset: {offsets:?}"
    );

    let final_offset = *offsets.last().expect("the fling ran frames");
    assert!(
        final_offset < max,
        "the fling must come to rest inside the content bounds (got {final_offset}, max {max})"
    );
    assert!(
        (final_offset - dragged) > 50.0,
        "the fling must carry the offset well past the drag ({dragged} → {final_offset})"
    );

    // Deceleration: the run's early frames cover more distance per frame
    // than its late frames.
    let early = offsets[2] - offsets[1];
    let late = final_offset - offsets[offsets.len() - 2];
    assert!(
        early > late && late.abs() < f64::EPSILON,
        "the fling must decelerate to rest (early {early}, late {late})"
    );
}

#[test]
fn a_touch_down_during_a_fling_stops_it_where_it_is() {
    let mut runtime = runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 400.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    for (frame_ms, y) in [(32_u64, 360.0_f32), (48, 320.0), (64, 280.0)] {
        runtime.push_input_event(touch_move(200.0, y));
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
    runtime.push_input_event(touch_up(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(80));

    // Let the fling advance.
    for frame in 1..=4u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(80 + frame * 16));
    }
    let flying =
        offset_y_at(&runtime, 200.0, 400.0).expect("the scroll view must publish its metrics");

    // A new touch down grabs the content: the fling stops where it is.
    runtime.push_input_event(touch_down(200.0, 500.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(144));
    let stopped =
        offset_y_at(&runtime, 200.0, 400.0).expect("the scroll view must publish its metrics");
    assert!(
        stopped >= flying,
        "the grab lands on the running fling's offset ({flying} → {stopped})"
    );

    runtime.push_input_event(touch_up(200.0, 500.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(160));
    for frame in 11..=30u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(64 + frame * 16));
    }
    let settled =
        offset_y_at(&runtime, 200.0, 400.0).expect("the scroll view must publish its metrics");
    assert!(
        (settled - stopped).abs() < f64::EPSILON,
        "the fling must stay stopped after the grab (stopped {stopped}, settled {settled})"
    );
}
