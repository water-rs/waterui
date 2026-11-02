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
use waterui::widget::condition::when;
use waterui::{AnyView, View, component::text};
use waterui_controls::button::button;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::{Id, SelfId};
use waterui_layout::frame::Frame;
use waterui_layout::scroll::{ScrollController, scroll};
use waterui_layout::stack::{VStack, hstack, vstack};
use waterui_navigation::NavigationView;
use waterui_navigation::tab::{Tab, TabsLayout};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{
    InputEvent, Overscroll, PointerButton, PointerKind, RubberBand, ScrollPlatform,
    TouchScrollConfig,
};

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

fn runtime(view: impl View) -> HeadlessRuntime {
    runtime_with(view, test_config())
}

fn runtime_with(view: impl View, config: TouchScrollConfig) -> HeadlessRuntime {
    let view = std::cell::RefCell::new(Some(AnyView::new(view)));
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
    runtime.set_touch_scroll_config(config);
    runtime
}

fn tall_stack() -> impl View {
    let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
    scroll(VStack::for_each(rows, |row| {
        let index = row.into_inner();
        vstack((text(format!("Row {index}")),)).size(360.0, ROW_HEIGHT)
    }))
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
fn scroll_with_button(tapped: Binding<bool>) -> impl View {
    scroll(vstack((
        ().size(360.0, 260.0),
        Frame::new(button("tap me").action(move || tapped.set(true)))
            .width(360.0)
            .height(44.0),
        ().size(360.0, 1_200.0),
    )))
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
    let mut runtime = runtime(scroll(
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
    ));
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

/// A fling only stops on a new touch, so it can outlive the scroll view it
/// drives: content swapped mid-fling by a non-input change (a data load)
/// drops the scroll view, and the fling ends with it instead of ticking a
/// handle no registered `ScrollTarget` holds.
#[test]
fn a_fling_whose_scroll_view_is_replaced_mid_fling_ends_with_it() {
    let show = Binding::container(true);
    let mut runtime = runtime(when(show.clone(), tall_stack).otherwise(|| text("Loaded")));
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
    let _ = runtime.pump_at(false, start + Duration::from_millis(96));
    assert!(
        runtime.renderer().has_active_touch_fling(),
        "a fast release over the scroll view must fling"
    );

    show.set(false);
    for frame in 1..=20u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(96 + frame * 16));
    }
    assert!(
        runtime.renderer().scroll_metrics_at(200.0, 400.0).is_none(),
        "the scroll view must be gone"
    );
    assert!(
        !runtime.renderer().has_active_touch_fling(),
        "the fling must end with the scroll view it drove"
    );
}

/// A tab switched in code mid-fling stops placing the scroll view it
/// showed: the view stays alive, so its handle still takes the fling's
/// writes, but no registered `ScrollTarget` holds the handle any more —
/// the fling ends with the view's registrations instead of marking a
/// retired owner.
#[test]
fn a_fling_whose_scroll_view_is_hidden_by_a_tab_switch_mid_fling_ends_with_it() {
    let tabs = waterui_core::views::SharedAnyViews::new(vec![
        Tab::new("Rows", || NavigationView::new("Rows", tall_stack())),
        Tab::new("Other", || NavigationView::new("Other", text("Other"))),
    ]);
    let selection = Binding::container(
        waterui_core::views::ViewSnapshot::get_id(&waterui_core::views::Views::snapshot(&tabs), 0)
            .expect("a non-empty tab set reports an id"),
    );
    let mut runtime = runtime(TabsLayout::new(selection.clone(), tabs));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    assert!(
        runtime.renderer().scroll_metrics_at(200.0, 300.0).is_some(),
        "the scroll view must be under the touch"
    );

    runtime.push_input_event(touch_down(200.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    for (frame_ms, y) in [(32_u64, 260.0_f32), (48, 220.0), (64, 180.0)] {
        runtime.push_input_event(touch_move(200.0, y));
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
    runtime.push_input_event(touch_up(200.0, 180.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(80));
    let _ = runtime.pump_at(false, start + Duration::from_millis(96));
    assert!(
        runtime.renderer().has_active_touch_fling(),
        "a fast release over the scroll view must fling"
    );

    selection.set(Id::try_from(2).expect("non-zero tab id"));
    for frame in 1..=20u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(96 + frame * 16));
    }
    assert!(
        runtime.renderer().scroll_metrics_at(200.0, 300.0).is_none(),
        "the hidden tab's scroll view must no longer be hittable"
    );
    assert!(
        !runtime.renderer().has_active_touch_fling(),
        "the fling must end with the scroll view it drove"
    );
}

/// water-rs/waterui#2794 — a host that reports no touch-scroll
/// parameters cannot receive a touch press: the renderer panics and
/// names the requirement instead of silently dropping the gesture.
#[test]
#[should_panic(
    expected = "hydrolysis: a touch press arrived but the platform window reported no touch_scroll_config"
)]
fn a_touch_press_without_a_touch_scroll_config_panics() {
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(|| AnyView::new(tall_stack())),
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    runtime.push_input_event(touch_down(200.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
}

/// A touch press waits out the touch delay before it begins; a target
/// removed inside that window drops the pending press with it, so the
/// release never begins a press on a node that no longer exists.
#[test]
fn a_touch_press_whose_target_is_removed_before_release_is_dropped() {
    let show = Binding::container(true);
    let tapped = Binding::container(false);
    let tapped_for_view = tapped.clone();
    let mut runtime = runtime(
        when(show.clone(), move || {
            let tapped = tapped_for_view.clone();
            Frame::new(button("tap me").action(move || tapped.set(true)))
                .width(400.0)
                .height(640.0)
        })
        .otherwise(|| text("Gone")),
    );
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(touch_down(200.0, 320.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    // Inside the 150ms touch delay, a non-input change removes the button.
    show.set(false);
    let _ = runtime.pump_at(false, start + Duration::from_millis(32));
    runtime.push_input_event(touch_up(200.0, 320.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(48));
    for frame in 4..=12u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
    }
    assert!(
        !runtime.renderer().has_pending_pointer_press(),
        "the delayed press must end with its target"
    );
    assert!(!tapped.snapshot(), "a removed button must not activate");
}

// water-rs/waterui#2795 — the `UIScrollView` physics family: exponential
// decay, and rubber-band edges a drag bands past and a release springs
// back from. The model's own math is unit-tested in `scroll_physics`; these
// drive it through the real input path.

fn uikit_runtime(view: impl View) -> HeadlessRuntime {
    runtime_with(view, TouchScrollConfig::uikit_default())
}

fn uikit_band() -> RubberBand {
    match TouchScrollConfig::uikit_default().physics().overscroll() {
        Overscroll::RubberBand(band) => band,
        Overscroll::Clamped | Overscroll::Stretch(_) => panic!("the UIKit family rubber-bands"),
    }
}

/// `ROWS` rows in a scroll view whose offset reports into `offset`.
fn reporting_stack(
    rows: usize,
    offset: &Binding<waterui_core::layout::Point>,
) -> impl View + use<> {
    let rows = (0..rows).map(SelfId::new).collect::<Vec<_>>();
    scroll(VStack::for_each(rows, |row| {
        let index = row.into_inner();
        vstack((text(format!("Row {index}")),)).size(360.0, ROW_HEIGHT)
    }))
    .report_offset(offset)
}

/// Pumps 16 ms frames from `from_ms` for `frames` frames and returns the
/// offsets of the scroll view under (200, 300) after each.
fn pump_offsets(
    runtime: &mut HeadlessRuntime,
    start: Instant,
    from_ms: u64,
    frames: u64,
) -> Vec<f64> {
    (1..=frames)
        .map(|frame| {
            let _ = runtime.pump_at(false, start + Duration::from_millis(from_ms + frame * 16));
            offset_y_at(runtime, 200.0, 300.0).expect("the scroll view must keep its metrics")
        })
        .collect()
}

/// Drags from (200, 200) down by 15 — past the slop, claimed — then to
/// (200, 310), and holds still long enough that the release carries no
/// velocity: 100 units of finger travel past the top edge.
fn drag_past_the_top(runtime: &mut HeadlessRuntime, start: Instant) {
    runtime.push_input_event(touch_down(200.0, 200.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    runtime.push_input_event(touch_move(200.0, 215.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(32));
    runtime.push_input_event(touch_move(200.0, 310.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(48));
}

#[test]
fn a_uikit_drag_past_the_top_edge_rubber_bands_and_its_release_springs_back() {
    let offset = Binding::container(waterui_core::layout::Point::zero());
    let mut runtime = uikit_runtime(reporting_stack(ROWS, &offset));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    let viewport = runtime
        .renderer()
        .scroll_metrics_at(200.0, 300.0)
        .expect("the scroll view must publish its metrics")
        .viewport_height;

    // At the top edge, a drag toward it is still claimed, and it bands:
    // 100 units past the edge (110 dragged − 10 slop) show as the band's.
    drag_past_the_top(&mut runtime, start);
    let banded = offset_y_at(&runtime, 200.0, 300.0).expect("the scroll view's metrics");
    let expected = -uikit_band().band(100.0, viewport);
    assert!(
        (banded - expected).abs() < 1e-6,
        "the drag bands past the edge: {banded}, expected {expected}"
    );
    assert!(banded > -100.0, "the band resists the finger");
    assert!(
        (f64::from(offset.snapshot().y) - banded).abs() < 1e-3,
        "the reported offset goes past the edge with it, as contentOffset does"
    );

    // A still release springs back onto the edge, monotonically, never
    // into the content.
    runtime.push_input_event(touch_up(200.0, 310.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(400));
    let offsets = pump_offsets(&mut runtime, start, 400, 150);
    assert!(
        offsets[0] < -1.0,
        "the spring takes time to return, not a snap: {offsets:?}"
    );
    assert!(
        offsets.windows(2).all(|pair| pair[1] >= pair[0]),
        "the spring returns monotonically: {offsets:?}"
    );
    assert!(offsets.iter().all(|&offset| offset <= 0.0));
    assert_eq!(
        offsets.last().copied(),
        Some(0.0),
        "it settles exactly on the edge"
    );
    assert_eq!(offset.snapshot().y, 0.0);
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn a_uikit_fling_into_the_bottom_edge_bounces_back_onto_it() {
    let offset = Binding::container(waterui_core::layout::Point::zero());
    // 20 rows of 44 over the 640-high window: a 240-unit extent.
    let mut runtime = uikit_runtime(reporting_stack(20, &offset));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    let max = runtime
        .renderer()
        .scroll_metrics_at(200.0, 300.0)
        .expect("the scroll view must publish its metrics")
        .max_y;
    assert!(max > 0.0, "the content must overflow the viewport");

    runtime.push_input_event(touch_down(200.0, 400.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    for (frame_ms, y) in [(32_u64, 360.0_f32), (48, 320.0), (64, 280.0)] {
        runtime.push_input_event(touch_move(200.0, y));
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
    runtime.push_input_event(touch_up(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(80));
    let offsets = pump_offsets(&mut runtime, start, 80, 300);

    let peak = offsets.iter().copied().fold(f64::MIN, f64::max);
    assert!(
        peak > max + 1.0,
        "the fling bounces past the edge (peak {peak}, max {max})"
    );
    assert_eq!(
        offsets.last().copied(),
        Some(max),
        "it returns exactly onto the edge"
    );
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn a_touch_catches_a_uikit_bounce_and_its_release_springs_back() {
    let mut runtime = uikit_runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    drag_past_the_top(&mut runtime, start);
    runtime.push_input_event(touch_up(200.0, 310.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(400));
    let springing = pump_offsets(&mut runtime, start, 400, 2);
    let caught_at = *springing.last().expect("two frames");
    assert!(caught_at < 0.0, "still past the edge when the touch lands");

    // The touch stops the spring where it is.
    runtime.push_input_event(touch_down(200.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(448));
    let held = pump_offsets(&mut runtime, start, 448, 3);
    assert!(
        held.iter()
            .all(|&offset| (offset - caught_at).abs() < f64::EPSILON)
    );

    // Lifting without a drag lets it go: it springs back onto the edge.
    runtime.push_input_event(touch_up(200.0, 300.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(512));
    let offsets = pump_offsets(&mut runtime, start, 512, 150);
    assert_eq!(offsets.last().copied(), Some(0.0));
    assert!(offsets.iter().all(|&offset| offset <= 0.0));
}

// water-rs/waterui#2796 — the non-Apple family's stretch edges: the offset
// stays in the extents and the content stretches. The effect's numerics
// are unit-tested against AOSP in `edge_stretch`; these drive it through
// the real input path.

/// The stretch intensities of the scroll view under (200, 300).
fn stretch_at(runtime: &HeadlessRuntime) -> kurbo::Vec2 {
    runtime
        .renderer()
        .scroll_stretch_at(200.0, 300.0)
        .expect("the scroll view must keep its target")
}

/// The scroll view's inner layer as committed, and its frame's content
/// space in window points composed from the committed layer props.
fn inner_stretch(
    runtime: &HeadlessRuntime,
) -> (
    crate::renderer::mount::layers::InnerStretchProbe,
    kurbo::Affine,
) {
    struct Probe(
        Option<(
            crate::renderer::mount::layers::InnerStretchProbe,
            kurbo::Affine,
        )>,
    );
    impl<E: crate::text::TextEngine> crate::renderer::mount::layers::LayerVisitor<E> for Probe {
        fn node(
            &mut self,
            layers: &crate::renderer::mount::layers::NodeLayers<E>,
            world: kurbo::Affine,
            _alphas: &[f32],
        ) {
            if let Some(probe) = layers.inner_stretch() {
                assert!(self.0.is_none(), "the test mounts one scroll view");
                self.0 = Some((probe, world));
            }
        }
    }
    let mut probe = Probe(None);
    crate::renderer::mount::layers::visit(&runtime.renderer().mount_roots(), &mut probe);
    probe.0.expect("the scroll view mounts an inner layer")
}

/// While the content stretches, the inner layer draws through HWUI's
/// stretch shader in the viewport's frame: the window's display transform
/// over the committed layers' composition, from the viewport's origin,
/// across its extent, in `stretch`'s direction.
fn assert_draws_the_stretch(runtime: &HeadlessRuntime, stretch: kurbo::Vec2) {
    let (probe, world) = inner_stretch(runtime);
    let (filter, params) = probe
        .stretch
        .expect("a stretch registers the inner layer's filter");
    assert_eq!(
        probe.filter,
        Some(filter),
        "the inner layer draws through it"
    );
    let viewport = probe.clip.expect("the inner layer clips to its viewport");
    let frame = runtime.renderer().window_display_transform
        * world
        * kurbo::Affine::translate(viewport.origin().to_vec2());
    let expected = frame.as_coeffs().into_iter().chain([
        viewport.width(),
        viewport.height(),
        stretch.x,
        stretch.y,
    ]);
    for (index, (actual, expected)) in params.into_iter().zip(expected).enumerate() {
        assert!(
            (f64::from(actual) - expected).abs() <= 1e-4 * expected.abs().max(1.0),
            "stretch parameter {index}: {actual}, expected {expected} ({params:?})"
        );
    }
}

#[test]
fn a_drag_past_the_top_edge_stretches_and_its_release_relaxes() {
    let mut runtime = runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    let viewport = runtime
        .renderer()
        .scroll_metrics_at(200.0, 300.0)
        .expect("the scroll view must publish its metrics")
        .viewport_height;

    // At the top edge a drag toward it is claimed and stretches the
    // content: 100 units past the slop pull the top edge by 100 / height,
    // drawn at AOSP `dampStretchVector`'s `0.016·d + 0.016·(1 − e^(−d·e/0.33))`.
    drag_past_the_top(&mut runtime, start);
    assert_eq!(
        offset_y_at(&runtime, 200.0, 300.0),
        Some(0.0),
        "the offset stays on the edge"
    );
    let pull = 100.0 / viewport;
    let expected = 0.016f64.mul_add(
        pull,
        0.016 * (1.0 - (-pull * core::f64::consts::E / 0.33).exp()),
    );
    let stretch = stretch_at(&runtime);
    assert!(
        (stretch.y - expected).abs() < 1e-9 && stretch.x.abs() < f64::EPSILON,
        "the top edge stretches: {stretch:?}, expected {expected}"
    );
    assert_draws_the_stretch(&runtime, stretch);

    // A still release relaxes it to nothing and ends the gesture.
    runtime.push_input_event(touch_up(200.0, 310.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(400));
    let mut relaxing = Vec::new();
    for frame in 1..=60_u64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(400 + frame * 16));
        relaxing.push(stretch_at(&runtime).y);
    }
    assert!(
        relaxing[0] > 0.0,
        "the stretch relaxes over time, not at once"
    );
    assert!(
        relaxing.windows(2).all(|pair| pair[1] <= pair[0]),
        "it relaxes monotonically: {relaxing:?}"
    );
    assert_eq!(relaxing.last().copied(), Some(0.0));
    assert_eq!(offset_y_at(&runtime, 200.0, 300.0), Some(0.0));
    assert!(!runtime.renderer().has_active_touch_fling());
    // At rest the inner layer draws without the filter, which stays
    // registered for the next pull.
    let (probe, _) = inner_stretch(&runtime);
    assert_eq!(probe.filter, None, "at rest nothing stretches the content");
    assert!(probe.stretch.is_some(), "the filter stays registered");
}

#[test]
fn a_fling_into_the_bottom_edge_is_absorbed_by_the_stretch() {
    let offset = Binding::container(waterui_core::layout::Point::zero());
    // 20 rows of 44 over the 640-high window: a 240-unit extent.
    let mut runtime = runtime(reporting_stack(20, &offset));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    let max = runtime
        .renderer()
        .scroll_metrics_at(200.0, 300.0)
        .expect("the scroll view must publish its metrics")
        .max_y;
    assert!(max > 0.0, "the content must overflow the viewport");

    runtime.push_input_event(touch_down(200.0, 400.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    for (frame_ms, y) in [(32_u64, 360.0_f32), (48, 320.0), (64, 280.0)] {
        runtime.push_input_event(touch_move(200.0, y));
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
    runtime.push_input_event(touch_up(200.0, 280.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(80));
    let mut stretches = Vec::new();
    let offsets = (1..=120_u64)
        .map(|frame| {
            let _ = runtime.pump_at(false, start + Duration::from_millis(80 + frame * 16));
            let stretch = stretch_at(&runtime);
            // HWUI's `NON_ZERO_EPSILON`: past it the content draws stretched.
            if stretch.y.abs() > 0.000_04 {
                assert_draws_the_stretch(&runtime, stretch);
            }
            stretches.push(stretch.y);
            offset_y_at(&runtime, 200.0, 300.0).expect("the scroll view's metrics")
        })
        .collect::<Vec<_>>();

    assert!(
        offsets.iter().all(|&offset| (0.0..=max).contains(&offset)),
        "the offset never leaves the extents: {offsets:?}"
    );
    assert_eq!(offsets.last().copied(), Some(max), "it rests on the edge");
    let deepest = stretches.iter().copied().fold(0.0, f64::min);
    assert!(
        deepest < 0.0,
        "the bottom edge absorbs the fling into a stretch: {stretches:?}"
    );
    assert_eq!(stretches.last().copied(), Some(0.0), "and relaxes");
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn the_physics_family_follows_the_compile_target() {
    let expected = if cfg!(target_vendor = "apple") {
        ScrollPlatform::Apple
    } else {
        ScrollPlatform::Other
    };
    assert_eq!(ScrollPlatform::native(), expected);
    assert_eq!(
        TouchScrollConfig::for_platform(ScrollPlatform::Apple),
        TouchScrollConfig::uikit_default()
    );
    assert_eq!(
        TouchScrollConfig::for_platform(ScrollPlatform::Other),
        TouchScrollConfig::android_default()
    );
}

#[test]
fn the_physics_family_follows_the_browsers_user_agent() {
    let apple = [
        // Safari on iPhone.
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
        // Chrome on iPhone.
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/126.0.6478.54 Mobile/15E148 Safari/604.1",
        // Safari on iPad with "Request Mobile Website".
        "Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
        // Safari on iPadOS by default, and on macOS: both report Macintosh.
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
        // Chrome on macOS.
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    ];
    let other = [
        // Chrome on Android (the reduced user agent).
        "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36",
        // Firefox on Android.
        "Mozilla/5.0 (Android 14; Mobile; rv:127.0) Gecko/127.0 Firefox/127.0",
        // Edge on Windows.
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0",
        // Firefox on Linux.
        "Mozilla/5.0 (X11; Linux x86_64; rv:127.0) Gecko/20100101 Firefox/127.0",
    ];
    for user_agent in apple {
        assert_eq!(
            ScrollPlatform::from_user_agent(user_agent),
            ScrollPlatform::Apple,
            "{user_agent}"
        );
    }
    for user_agent in other {
        assert_eq!(
            ScrollPlatform::from_user_agent(user_agent),
            ScrollPlatform::Other,
            "{user_agent}"
        );
    }
}

// water-rs/waterui#2904 — each scroll view's overscroll effect lives with
// that scroll view, not with whichever fling last took a single fling slot:
// a newer claim on the same view ends its fling but lets its stretch relax
// on its own spring, or — under the rubber band, whose overscroll is the
// offset itself — takes the offset and clamps it into the extents; a touch,
// a release or a fling on another scroll view touches neither.

/// The two side-by-side scroll views these cases drive: the left one under
/// x = `LEFT`, the right one under x = `RIGHT`, each `ROWS` rows tall.
const LEFT: f32 = 100.0;
const RIGHT: f32 = 300.0;

fn side_by_side() -> impl View {
    let column = |name: &'static str| {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        scroll(VStack::for_each(rows, move |row| {
            let index = row.into_inner();
            vstack((text(format!("{name} {index}")),)).size(180.0, ROW_HEIGHT)
        }))
    };
    hstack((column("Left"), column("Right"))).spacing(0.0)
}

/// A scroll view driven by `controller`, `ROWS` rows tall.
fn controlled_stack(
    controller: &ScrollController<waterui_core::layout::Point>,
) -> impl View + use<> {
    let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
    scroll(VStack::for_each(rows, |row| {
        let index = row.into_inner();
        vstack((text(format!("Row {index}")),)).size(360.0, ROW_HEIGHT)
    }))
    .scroll_controller(controller)
}

fn offset_at(runtime: &HeadlessRuntime, x: f32) -> f64 {
    offset_y_at(runtime, x, 300.0).expect("the scroll view must keep its metrics")
}

fn stretch_y_at(runtime: &HeadlessRuntime, x: f32) -> f64 {
    runtime
        .renderer()
        .scroll_stretch_at(x, 300.0)
        .expect("the scroll view must keep its target")
        .y
}

fn assert_side_by_side(runtime: &HeadlessRuntime) {
    let left = runtime
        .renderer()
        .scroll_metrics_at(LEFT, 300.0)
        .expect("the left scroll view is under x = LEFT");
    let right = runtime
        .renderer()
        .scroll_metrics_at(RIGHT, 300.0)
        .expect("the right scroll view is under x = RIGHT");
    assert!(
        left.viewport_width <= 200.0 && right.viewport_width <= 200.0 && left.max_y > 0.0,
        "two scrollable views side by side: {left:?} {right:?}"
    );
}

/// Pulls the scroll view under `x` 100 units past its top edge, gives the
/// drag back 30 of them, and flicks it into the content: the release at
/// 224 ms flings into the content at 1250 units/s while the pulled edge,
/// still stretched, relaxes — the fling and the stretch run together.
fn pull_then_flick(runtime: &mut HeadlessRuntime, start: Instant, x: f32) {
    for (frame_ms, event) in [
        (16_u64, touch_down(x, 200.0)),
        (32, touch_move(x, 215.0)),
        (48, touch_move(x, 310.0)),
        (200, touch_move(x, 300.0)),
        (208, touch_move(x, 290.0)),
        (216, touch_move(x, 280.0)),
        (224, touch_up(x, 280.0)),
    ] {
        runtime.push_input_event(event);
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
}

/// Pulls the scroll view under `x` 100 units past its top edge and lets go
/// of it still at 400 ms.
fn pull_and_let_go(runtime: &mut HeadlessRuntime, start: Instant, x: f32) {
    for (frame_ms, event) in [
        (16_u64, touch_down(x, 200.0)),
        (32, touch_move(x, 215.0)),
        (48, touch_move(x, 310.0)),
        (400, touch_up(x, 310.0)),
    ] {
        runtime.push_input_event(event);
        let _ = runtime.pump_at(false, start + Duration::from_millis(frame_ms));
    }
}

/// A fast upward fling on the scroll view under `x`, from `from_ms`: down,
/// three 40-unit moves 16 ms apart, and the release.
fn fling_up(runtime: &mut HeadlessRuntime, start: Instant, x: f32, from_ms: u64) {
    runtime.push_input_event(touch_down(x, 500.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(from_ms));
    for (step, y) in [(1_u64, 460.0_f32), (2, 420.0), (3, 380.0)] {
        runtime.push_input_event(touch_move(x, y));
        let _ = runtime.pump_at(false, start + Duration::from_millis(from_ms + step * 16));
    }
    runtime.push_input_event(touch_up(x, 380.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(from_ms + 64));
}

/// A 40-unit pixel wheel scroll into the content over the scroll view
/// under `x`.
const fn wheel_into_the_content(x: f32) -> InputEvent {
    InputEvent::Scroll {
        x,
        y: 300.0,
        dx: 0.0,
        dy: -40.0,
        is_line_delta: false,
    }
}

/// Pumps 16 ms frames from `from_ms` for 1.44 s and returns the stretch of
/// the scroll view under `x` after each.
fn relaxing_stretch(
    runtime: &mut HeadlessRuntime,
    start: Instant,
    from_ms: u64,
    x: f32,
) -> Vec<f64> {
    (1..=90_u64)
        .map(|frame| {
            let _ = runtime.pump_at(false, start + Duration::from_millis(from_ms + frame * 16));
            stretch_y_at(runtime, x)
        })
        .collect()
}

/// The stretch keeps relaxing from where the interruption found it: shown
/// on the interrupted frame, never growing, and at rest by the end.
fn assert_relaxes(before: f64, after: f64, rest: &[f64]) {
    assert!(
        before > 0.0,
        "the top edge stretches before the interruption"
    );
    assert!(
        after > 0.0 && after <= before,
        "the interruption must not drop the stretch: {before} → {after}"
    );
    // A request lands on the frame after it is issued, so the frame after
    // the interruption must still show the stretch too.
    assert!(
        rest[0] > 0.0,
        "the stretch must still relax once the interruption landed: {rest:?}"
    );
    let mut relaxing = vec![after];
    relaxing.extend_from_slice(rest);
    assert!(
        relaxing.windows(2).all(|pair| pair[1] <= pair[0]),
        "it relaxes monotonically: {relaxing:?}"
    );
    assert_eq!(rest.last().copied(), Some(0.0), "and comes to rest");
}

#[test]
fn a_wheel_scroll_ends_the_fling_but_lets_the_stretch_relax() {
    let mut runtime = runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    pull_then_flick(&mut runtime, start, 200.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(240));
    let before = stretch_y_at(&runtime, 200.0);
    assert!(
        offset_at(&runtime, 200.0) > 0.0,
        "the flick flings into the content"
    );

    runtime.push_input_event(wheel_into_the_content(200.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(256));
    let after = stretch_y_at(&runtime, 200.0);
    let wheeled = offset_at(&runtime, 200.0);
    let rest = relaxing_stretch(&mut runtime, start, 256, 200.0);

    assert_relaxes(before, after, &rest);
    assert!(
        (offset_at(&runtime, 200.0) - wheeled).abs() < f64::EPSILON,
        "the wheel owns the offset: the fling it superseded writes no more"
    );
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn a_programmatic_scroll_ends_the_fling_but_lets_the_stretch_relax() {
    let controller = ScrollController::new(waterui_core::layout::Point::zero());
    let mut runtime = runtime(controlled_stack(&controller));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    pull_then_flick(&mut runtime, start, 200.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(240));
    let before = stretch_y_at(&runtime, 200.0);

    controller.scroll_to(waterui_core::layout::Point::new(0.0, 500.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(256));
    let after = stretch_y_at(&runtime, 200.0);
    let rest = relaxing_stretch(&mut runtime, start, 256, 200.0);

    assert_relaxes(before, after, &rest);
    assert!(
        (offset_at(&runtime, 200.0) - 500.0).abs() < f64::EPSILON,
        "the request owns the offset: the fling it superseded writes no more"
    );
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn another_scroll_views_fling_leaves_a_running_stretch_and_fling_alone() {
    let mut runtime = runtime(side_by_side());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    assert_side_by_side(&runtime);
    pull_then_flick(&mut runtime, start, LEFT);
    let _ = runtime.pump_at(false, start + Duration::from_millis(240));
    let before = stretch_y_at(&runtime, LEFT);
    let flinging = offset_at(&runtime, LEFT);

    // The right scroll view is touched, dragged and flung while the left
    // one still flings and relaxes.
    fling_up(&mut runtime, start, RIGHT, 256);
    let after = stretch_y_at(&runtime, LEFT);
    assert!(
        offset_at(&runtime, LEFT) > flinging,
        "the left fling keeps running through the right one's touch"
    );
    let rest = relaxing_stretch(&mut runtime, start, 320, LEFT);

    assert_relaxes(before, after, &rest);
    assert!(
        offset_at(&runtime, RIGHT) > 0.0,
        "the right scroll view flung"
    );
    assert!(!runtime.renderer().has_active_touch_fling());
}

/// The offsets of the scroll view under `x` over 16 ms frames from
/// `from_ms` for 2.4 s.
fn settling_offsets(
    runtime: &mut HeadlessRuntime,
    start: Instant,
    from_ms: u64,
    x: f32,
) -> Vec<f64> {
    (1..=150_u64)
        .map(|frame| {
            let _ = runtime.pump_at(false, start + Duration::from_millis(from_ms + frame * 16));
            offset_at(runtime, x)
        })
        .collect()
}

/// Leaves the scroll view under `x` springing back from past its top edge
/// at 432 ms, and returns that offset.
fn springing_back(runtime: &mut HeadlessRuntime, start: Instant, x: f32) -> f64 {
    pull_and_let_go(runtime, start, x);
    let _ = runtime.pump_at(false, start + Duration::from_millis(416));
    let _ = runtime.pump_at(false, start + Duration::from_millis(432));
    let springing = offset_at(runtime, x);
    assert!(springing < 0.0, "still past the edge: {springing}");
    springing
}

#[test]
fn a_wheel_scroll_takes_a_uikit_bounce_back_into_the_extents() {
    let mut runtime = uikit_runtime(tall_stack());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    springing_back(&mut runtime, start, 200.0);

    // The wheel claims the offset the spring held past the edge: it lands
    // in the extents and stays where the wheel put it.
    runtime.push_input_event(wheel_into_the_content(200.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(448));
    let wheeled = offset_at(&runtime, 200.0);
    assert!(wheeled >= 0.0, "back inside the extents: {wheeled}");
    let offsets = settling_offsets(&mut runtime, start, 448, 200.0);
    assert!(
        offsets
            .iter()
            .all(|&offset| (offset - wheeled).abs() < f64::EPSILON),
        "the superseded spring writes no more: {offsets:?}"
    );
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn a_programmatic_scroll_takes_a_uikit_bounce_back_into_the_extents() {
    let controller = ScrollController::new(waterui_core::layout::Point::zero());
    let mut runtime = uikit_runtime(controlled_stack(&controller));
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    springing_back(&mut runtime, start, 200.0);

    controller.scroll_to(waterui_core::layout::Point::new(0.0, 500.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(448));
    let landed = offset_at(&runtime, 200.0);
    let offsets = settling_offsets(&mut runtime, start, 448, 200.0);
    assert!(
        core::iter::once(landed)
            .chain(offsets.iter().copied())
            .all(|offset| (offset - 500.0).abs() < f64::EPSILON),
        "the request owns the offset; the superseded spring writes no more: {landed} {offsets:?}"
    );
    assert!(!runtime.renderer().has_active_touch_fling());
}

#[test]
fn another_scroll_views_fling_leaves_a_uikit_bounce_to_settle() {
    let mut runtime = uikit_runtime(side_by_side());
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    assert_side_by_side(&runtime);
    let springing = springing_back(&mut runtime, start, LEFT);

    fling_up(&mut runtime, start, RIGHT, 448);
    let offsets = settling_offsets(&mut runtime, start, 512, LEFT);
    assert!(
        offsets[0] > springing && offsets[0] < 0.0,
        "the left spring keeps returning through the right one's touch: {springing} → {offsets:?}"
    );
    assert!(
        offsets.windows(2).all(|pair| pair[1] >= pair[0]),
        "it returns monotonically: {offsets:?}"
    );
    assert_eq!(
        offsets.last().copied(),
        Some(0.0),
        "it settles exactly on the edge, never left past it"
    );
    // The right fling's decay is still running — UIKit's projection of a
    // 2500 units/s release takes about 4.7 s to come to rest.
    assert!(
        offset_at(&runtime, RIGHT) > 0.0,
        "the right scroll view flung"
    );
}
