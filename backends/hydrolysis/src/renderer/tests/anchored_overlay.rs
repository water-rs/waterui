//! `.anchored_overlay(...)` (water-rs/waterui#1275), end to end through the
//! retained renderer: the shared placement contract
//! (`waterui_backend_core::overlay::place_anchored_overlay`) for an anchor in
//! the middle of the window, the flip when the preferred edge overflows, the
//! window clamp, outside-interaction dismissal versus `Manual`, and
//! re-placement when the window resizes.
//!
//! Placement assertions read `HeadlessRuntime::anchored_overlay_frames()` —
//! the hit-space rects the post-flush pass drew the overlays into.

use std::time::{Duration, Instant};

use accesskit::Role;
use nami::Binding;
use nami::Signal as _;
use nami::SignalExt as _;
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::component::text;
use waterui::metadata::anchored_overlay::{
    AnchorEdge, AnchoredOverlay, Clamp, Dismissal, EdgeAlignment,
};
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::Color;
use waterui_layout::frame::Frame;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::vstack;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const WINDOW: (u32, u32) = (320, 240);
/// The overlay content every test presents: a fixed `60x20` frame, so the
/// placed frame's size is exact.
const OVERLAY: (f32, f32) = (60.0, 20.0);
const GAP: f32 = 4.0;
const MARGIN: f32 = 8.0;

fn overlay_content() -> AnyView {
    AnyView::new(Frame::new(text("tip")).width(OVERLAY.0).height(OVERLAY.1))
}

fn runtime(view: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        view,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    )
}

fn pump_until_settled(runtime: &mut HeadlessRuntime) {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
}

/// The a11y bounds `label`'s button was last seen with — the anchor's frame,
/// in hit space.
fn bounds_of(runtime: &mut HeadlessRuntime, label: &str) -> accesskit::Rect {
    pump_until_settled(runtime);
    runtime
        .accessibility_tree()
        .as_ref()
        .and_then(|update| find_by_label(update, Role::Button, label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{label} must emit a button with bounds"))
}

fn primary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ]
}

/// The one presented overlay's frame — every test here mounts exactly one.
fn presented_frame(runtime: &HeadlessRuntime) -> kurbo::Rect {
    let frames = runtime.anchored_overlay_frames();
    assert_eq!(frames.len(), 1, "exactly one overlay is presented");
    frames[0]
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.5
}

/// An anchor in the middle of the window, `edge: Bottom, alignment: Center`,
/// gap 4: the overlay hugs the anchor's bottom edge, centered on it.
#[test]
fn a_mid_window_anchor_places_the_overlay_below_it_centered() {
    let open = Binding::container(false);
    let open_for_view = open.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 100.0),
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, overlay_content)
                    .edge(AnchorEdge::Bottom)
                    .alignment(EdgeAlignment::Center)
                    .gap(GAP)
                    .clamp(Clamp::Window { margin: MARGIN }),
            ),
            ().size(0.0, 100.0),
        )))
    });
    let mut runtime = runtime(view);
    let anchor = bounds_of(&mut runtime, "host");

    open.set(true);
    pump_until_settled(&mut runtime);

    let frame = presented_frame(&runtime);
    assert!(
        near(
            frame.x0,
            f64::midpoint(anchor.x0, anchor.x1) - f64::from(OVERLAY.0) / 2.0
        ) && near(frame.y0, anchor.y1 + f64::from(GAP))
            && near(frame.width(), f64::from(OVERLAY.0))
            && near(frame.height(), f64::from(OVERLAY.1)),
        "the overlay sits centered under the anchor, got {frame:?} for anchor {anchor:?}"
    );
}

/// With `edge: Top` on an anchor at the top of the window the overlay does
/// not fit above it; `flip` moves it to the bottom edge. The trailing spacer
/// fills the column, so the anchor starts at the window's top edge.
#[test]
fn an_overlay_too_tall_for_the_top_edge_flips_below() {
    let open = Binding::container(true);
    let open_for_view = open;
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, overlay_content)
                    .edge(AnchorEdge::Top)
                    .alignment(EdgeAlignment::Center)
                    .gap(GAP)
                    .flip(true)
                    .clamp(Clamp::Window { margin: MARGIN }),
            ),
            spacer(),
        )))
    });
    let mut runtime = runtime(view);
    let anchor = bounds_of(&mut runtime, "host");

    let frame = presented_frame(&runtime);
    assert!(
        near(frame.y0, anchor.y1 + f64::from(GAP)),
        "the flipped overlay hugs the anchor's bottom edge, got {frame:?} for anchor {anchor:?}"
    );
}

/// `edge: Trailing` on an anchor reaching the trailing window edge puts the
/// overlay past it; `Clamp::Window` pulls it back inside the margin.
#[test]
fn a_trailing_edge_overlay_clamps_inside_the_window_margin() {
    let open = Binding::container(true);
    let open_for_view = open;
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 100.0),
            Frame::new(text("anchor"))
                .width(crate::num_cast::u32_as_f32(WINDOW.0))
                .height(20.0)
                .anchored_overlay(
                    AnchoredOverlay::new(&open, overlay_content)
                        .edge(AnchorEdge::Trailing)
                        .alignment(EdgeAlignment::Center)
                        .gap(GAP)
                        .clamp(Clamp::Window { margin: MARGIN }),
                ),
        )))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);

    let frame = presented_frame(&runtime);
    assert!(
        near(frame.x1, f64::from(WINDOW.0) - f64::from(MARGIN))
            && near(frame.width(), f64::from(OVERLAY.0)),
        "the overlay's trailing edge sits at the window margin, got {frame:?}"
    );
}

/// `OutsideInteraction` (the default): a press outside the overlay writes
/// `false` to `is_presented` — and the press still reaches its target, the
/// same delivery the context menu's outside dismissal uses.
#[test]
fn an_outside_press_writes_false_and_still_reaches_its_target() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let pressed = Binding::container(false);
    let pressed_for_view = pressed.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let pressed = pressed_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 100.0),
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, overlay_content)
                    .edge(AnchorEdge::Bottom)
                    .gap(GAP),
            ),
            ().size(0.0, 40.0),
            button("far").action(move || {
                pressed.set(true);
            }),
        )))
    });
    let mut runtime = runtime(view);
    // The far button — below the overlay, clear of its frame — takes the
    // press, which also dismisses the overlay.
    let far = bounds_of(&mut runtime, "far");
    let frame = presented_frame(&runtime);
    let (x, y) = (
        crate::num_cast::f64_as_f32(f64::midpoint(far.x0, far.x1)),
        crate::num_cast::f64_as_f32(f64::midpoint(far.y0, far.y1)),
    );
    assert!(
        !frame.contains(kurbo::Point::new(f64::from(x), f64::from(y))),
        "the press target must be outside the overlay frame"
    );
    for event in primary_click(x, y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);

    assert!(
        pressed.snapshot(),
        "the outside press still reaches its target"
    );
    assert!(
        !open.snapshot(),
        "an outside press writes false to is_presented"
    );
    assert!(
        runtime.anchored_overlay_frames().is_empty(),
        "the dismissed overlay leaves the window"
    );
}

/// `Dismissal::Manual`: the same outside press leaves the overlay open —
/// only the binding closes it.
#[test]
fn a_manual_overlay_ignores_outside_presses() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 100.0),
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, overlay_content)
                    .edge(AnchorEdge::Bottom)
                    .gap(GAP)
                    .dismissal(Dismissal::Manual),
            ),
        )))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    let frame = presented_frame(&runtime);

    // A press in the window's top-leading corner — clear of the overlay,
    // which hangs below the mid-window anchor.
    for event in primary_click(4.0, 4.0) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);

    assert!(
        open.snapshot(),
        "manual dismissal ignores the outside press"
    );
    assert_eq!(
        presented_frame(&runtime),
        frame,
        "the overlay still draws at the same frame"
    );

    open.set(false);
    pump_until_settled(&mut runtime);
    assert!(
        runtime.anchored_overlay_frames().is_empty(),
        "the binding still closes a manual overlay"
    );
}

/// The overlay follows the window: shrinking the window so the natural
/// placement overflows reclamps the frame inside the new margin.
#[test]
fn a_window_resize_replaces_the_overlay() {
    let open = Binding::container(true);
    let open_for_view = open;
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 100.0),
            Frame::new(text("anchor"))
                .width(crate::num_cast::u32_as_f32(WINDOW.0))
                .height(20.0)
                .anchored_overlay(
                    AnchoredOverlay::new(&open, overlay_content)
                        .edge(AnchorEdge::Trailing)
                        .alignment(EdgeAlignment::Center)
                        .gap(GAP)
                        .clamp(Clamp::Window { margin: MARGIN }),
                ),
        )))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    let before = presented_frame(&runtime);
    assert!(
        near(before.x1, f64::from(WINDOW.0) - f64::from(MARGIN)),
        "before the resize the trailing edge sits at the margin, got {before:?}"
    );

    // Shrink the window horizontally: the clamp pins the trailing edge to the
    // new window's margin.
    runtime.push_input_event(InputEvent::Resize {
        width: 200,
        height: WINDOW.1,
    });
    pump_until_settled(&mut runtime);

    let after = presented_frame(&runtime);
    assert!(
        near(after.x1, 200.0 - f64::from(MARGIN)),
        "the resize re-placed the overlay inside the new window, got {after:?}"
    );
    assert_ne!(before, after, "the overlay moved with the window");
}

/// The measurement offer is the window minus the clamp margins, so a long
/// text wraps into a taller frame instead of overflowing horizontally.
#[test]
fn a_long_overlay_wraps_inside_the_window() {
    let open = Binding::container(true);
    let open_for_view = open;
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 60.0),
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, move || {
                    AnyView::new(text(
                        "A long line of overlay text that cannot fit on a \
                         single line inside a narrow window.",
                    ))
                })
                .edge(AnchorEdge::Bottom)
                .alignment(EdgeAlignment::Center)
                .gap(GAP)
                .clamp(Clamp::Window { margin: MARGIN }),
            ),
        )))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);

    let frame = presented_frame(&runtime);
    assert!(
        frame.width() <= f64::from(WINDOW.0) + 0.5 && frame.width() > 100.0,
        "the text laid out bounded by the window, got {frame:?}"
    );
    assert!(
        frame.height() > f64::from(OVERLAY.1),
        "the content wrapped into a taller frame, got {frame:?}"
    );
    assert!(
        frame.x0 >= f64::from(MARGIN) - 0.5
            && frame.y1 <= f64::from(WINDOW.1) - f64::from(MARGIN) + 0.5,
        "the wrapped overlay stays inside the window margins, got {frame:?}"
    );
}

/// A scale animation the content starts when `is_presented` turns false
/// plays to completion on screen — the exit waits on the content's own
/// animation slots, never on a timeout.
#[test]
fn a_closing_overlays_exit_animation_plays_to_completion() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let scale = Binding::f32(1.0);
    let scale_for_view = scale.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let scale = scale_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, move || {
                let scale = scale.clone();
                AnyView::new(
                    Frame::new(
                        text("tip").scale(
                            scale
                                .clone()
                                .with(Animation::linear(Duration::from_millis(1_000))),
                            scale.with(Animation::linear(Duration::from_millis(1_000))),
                        ),
                    )
                    .width(OVERLAY.0)
                    .height(OVERLAY.1),
                )
            })
            .edge(AnchorEdge::Bottom)
            .gap(GAP),
        ),)))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    assert_eq!(
        runtime.anchored_overlay_frames().len(),
        1,
        "the overlay is presented"
    );

    // Anchor the frame clock at `start` before dismissing: signal updates
    // apply with the last pump's instant, so an unanchored `set` would let
    // wall time since that pump count toward the animation's elapsed.
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    // Close, then shrink the content: the exit must keep the overlay drawn
    // until that animation finishes.
    open.set(false);
    scale.set(0.0);

    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    assert_eq!(
        runtime.anchored_overlay_frames().len(),
        1,
        "mid-exit the overlay is still on screen"
    );

    let _ = runtime.pump_at(false, start + Duration::from_millis(600));
    assert_eq!(
        runtime.anchored_overlay_frames().len(),
        1,
        "the exit animation is still running"
    );

    let _ = runtime.pump_at(false, start + Duration::from_millis(1_500));
    pump_until_settled(&mut runtime);
    assert!(
        runtime.anchored_overlay_frames().is_empty(),
        "once the exit animation finishes the overlay is gone"
    );
}

/// A dismissed overlay whose content starts no animation in response is gone
/// on the dismissal frame itself — the exit draws nothing once the scope has
/// no slot left to animate.
#[test]
fn an_overlay_with_no_exit_animation_is_gone_on_the_dismissal_frame() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, overlay_content)
                .edge(AnchorEdge::Bottom)
                .gap(GAP),
        ),)))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    assert_eq!(
        runtime.anchored_overlay_frames().len(),
        1,
        "the overlay is presented"
    );

    open.set(false);
    let _ = runtime.pump_at(false, Instant::now());
    assert!(
        runtime.anchored_overlay_frames().is_empty(),
        "no exit animation started — the overlay is gone on the dismissal frame"
    );
}

/// Visual evidence of the exit contract: halfway through a content scale
/// animation the dismissed overlay is still drawn, mid-flight at scale 0.5.
/// The frame lands under
/// `<artifacts>/hydrolysis/anchored_overlay_exit/mid-flight.png`.
#[test]
fn the_exiting_overlay_draws_its_animation_mid_flight() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let scale = Binding::f32(1.0);
    let scale_for_view = scale.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let scale = scale_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, move || {
                let scale = scale.clone();
                AnyView::new(
                    Frame::new(
                        Color::srgb(30, 120, 220).scale(
                            scale
                                .clone()
                                .with(Animation::linear(Duration::from_millis(1_000))),
                            scale.with(Animation::linear(Duration::from_millis(1_000))),
                        ),
                    )
                    .width(OVERLAY.0)
                    .height(OVERLAY.1),
                )
            })
            .edge(AnchorEdge::Bottom)
            .gap(GAP),
        ),)))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    let presented = presented_frame(&runtime);

    // Anchor the frame clock the same way, so the capture lands at scale 0.5
    // exactly instead of wherever wall time since the last pump left it.
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    open.set(false);
    scale.set(0.0);
    let result = runtime.pump_at(true, start + Duration::from_millis(500));
    let (width, height, rgba8, _) = result
        .into_snapshot()
        .expect("a mid-exit frame must be capturable")
        .into_parts();
    let path =
        crate::testing::artifact_snapshot_path("hydrolysis", "anchored_overlay_exit", "mid-flight");
    std::fs::create_dir_all(path.parent().expect("the case directory"))
        .expect("the capture directory must be creatable");
    image::RgbaImage::from_raw(width, height, rgba8)
        .expect("snapshot dimensions must match the rgba buffer")
        .save(&path)
        .expect("the mid-exit frame must be writable");

    assert_eq!(
        runtime.anchored_overlay_frames().as_slice(),
        &[presented],
        "mid-exit the overlay still draws at its placed frame"
    );
}

/// An overlay mid-exit is inert: a press landing inside its frame — on the
/// button the content carries — reaches the content underneath instead.
#[test]
fn input_during_the_exit_reaches_the_content_underneath() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let scale = Binding::f32(1.0);
    let scale_for_view = scale.clone();
    let inner = Binding::container(false);
    let inner_for_view = inner.clone();
    let under_pressed = Binding::container(false);
    let under_for_view = under_pressed.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let scale = scale_for_view.clone();
        let inner = inner_for_view.clone();
        let under = under_for_view.clone();
        AnyView::new(vstack((
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, move || {
                    let inner = inner.clone();
                    let scale = scale.clone();
                    AnyView::new(
                        button("in")
                            .action(move || {
                                inner.set(true);
                            })
                            .scale(
                                scale
                                    .clone()
                                    .with(Animation::linear(Duration::from_millis(1_000))),
                                scale.with(Animation::linear(Duration::from_millis(1_000))),
                            ),
                    )
                })
                .edge(AnchorEdge::Bottom)
                .gap(GAP),
            ),
            button("under").action(move || {
                under.set(true);
            }),
        )))
    });
    let mut runtime = runtime(view);
    let under = bounds_of(&mut runtime, "under");
    pump_until_settled(&mut runtime);
    let frame = presented_frame(&runtime);

    // Presented, a press inside the frame lands on the overlay's own button.
    let point = kurbo::Point::new(
        f64::midpoint(frame.x0, frame.x1),
        f64::midpoint(frame.y0, frame.y1),
    );
    assert!(
        under.contains(accesskit::Point::new(point.x, point.y)),
        "the 'under' button must reach into the overlay's frame for this test"
    );
    for event in primary_click(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
    ) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    assert!(
        inner.snapshot(),
        "a presented overlay's content takes the press inside its frame"
    );
    assert!(
        !under_pressed.snapshot(),
        "the press did not reach the content underneath"
    );

    // Close with an exit animation running: the same press now lands on the
    // content below the still-drawn overlay. The clock anchor keeps the
    // animation's epoch synthetic — see the exit-playout test above.
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    open.set(false);
    scale.set(0.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    assert_eq!(
        runtime.anchored_overlay_frames().len(),
        1,
        "the exiting overlay is still drawn"
    );

    for event in primary_click(
        crate::num_cast::f64_as_f32(point.x),
        crate::num_cast::f64_as_f32(point.y),
    ) {
        runtime.push_input_event(event);
    }
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        under_pressed.snapshot(),
        "the exiting overlay no longer intercepts input"
    );
}

/// `is_presented` back to `true` mid-exit presents a new, independent
/// instance (layout-spec.md §4.5): the dismissed presentation keeps drawing
/// its exit beside it, inert, until its own animations finish, then leaves —
/// the new one is presented at the same placed frame throughout.
#[test]
fn re_presenting_mid_exit_enters_a_new_instance_beside_the_exit() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let scale = Binding::f32(1.0);
    let scale_for_view = scale.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let scale = scale_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, move || {
                let scale = scale.clone();
                AnyView::new(
                    Frame::new(
                        text("tip").scale(
                            scale
                                .clone()
                                .with(Animation::linear(Duration::from_millis(1_000))),
                            scale.with(Animation::linear(Duration::from_millis(1_000))),
                        ),
                    )
                    .width(OVERLAY.0)
                    .height(OVERLAY.1),
                )
            })
            .edge(AnchorEdge::Bottom)
            .gap(GAP),
        ),)))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    let before = presented_frame(&runtime);

    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    open.set(false);
    scale.set(0.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));

    open.set(true);
    scale.set(1.0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(300));
    assert_eq!(
        runtime.anchored_overlay_frames().as_slice(),
        &[before, before],
        "mid-exit the dismissed presentation still draws beside the new one"
    );

    // The exiting instance's scale animation finishes ~1300ms in; it leaves
    // and the new presentation stays.
    let _ = runtime.pump_at(false, start + Duration::from_millis(1_400));
    let _ = runtime.pump_at(false, start + Duration::from_millis(1_416));
    assert_eq!(
        runtime.anchored_overlay_frames().as_slice(),
        &[before],
        "once the exit finishes only the new presentation remains, at the same frame"
    );
}

/// An overlay body declaring `.transition(opacity)` (layout-spec.md §4.5):
/// presenting runs the insertion, the dismissal removes the content from the
/// accessibility tree on that frame while the presentation keeps drawing in
/// its placed frame under the removal clock, and the clock's completion — not
/// a content animation — ends the exit. A re-present while it runs enters a
/// new instance beside the leaving one.
#[test]
fn an_overlay_body_transition_runs_on_present_and_dismiss() {
    let open = Binding::container(false);
    let open_for_view = open.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, || {
                AnyView::new(
                    button("inside")
                        .action(|| {})
                        .transition(waterui_structural_transition::PropertyTransition::opacity())
                        .animation(Animation::linear(Duration::from_millis(1_000))),
                )
            })
            .edge(AnchorEdge::Bottom)
            .gap(GAP),
        ),)))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    let has_inside = |runtime: &mut HeadlessRuntime| {
        runtime
            .accessibility_tree()
            .as_ref()
            .and_then(|update| find_by_label(update, Role::Button, "inside"))
            .is_some()
    };

    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    open.set(true);
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    let presented = presented_frame(&runtime);
    let mid_insertion = runtime
        .pump_at(true, start + Duration::from_millis(516))
        .into_snapshot()
        .expect("a mid-insertion frame must be capturable");
    assert!(
        has_inside(&mut runtime),
        "the inserted presentation is in the accessibility tree from its first frame"
    );
    let settled = runtime
        .pump_at(true, start + Duration::from_millis(1_100))
        .into_snapshot()
        .expect("a settled frame must be capturable");
    assert!(
        mid_insertion.rgba8() != settled.rgba8(),
        "halfway through the insertion the overlay is still fading in"
    );

    open.set(false);
    let _ = runtime.pump_at(false, start + Duration::from_millis(1_116));
    assert!(
        !has_inside(&mut runtime),
        "the dismissed presentation leaves the accessibility tree at once"
    );
    assert_eq!(
        runtime.anchored_overlay_frames().as_slice(),
        &[presented],
        "the leaving presentation keeps drawing in its placed frame"
    );
    let mid_removal = runtime
        .pump_at(true, start + Duration::from_millis(1_616))
        .into_snapshot()
        .expect("a mid-removal frame must be capturable");
    assert!(
        mid_removal.rgba8() != settled.rgba8(),
        "halfway through the removal the overlay is fading out"
    );

    // Re-present mid-removal: a new instance enters beside the leaving one.
    open.set(true);
    let _ = runtime.pump_at(false, start + Duration::from_millis(1_632));
    assert_eq!(
        runtime.anchored_overlay_frames().as_slice(),
        &[presented, presented],
        "the leaving presentation and the new one both draw"
    );
    assert!(
        has_inside(&mut runtime),
        "the new presentation is in the accessibility tree"
    );

    // The removal clock completes ~2116ms in and releases the old instance.
    let _ = runtime.pump_at(false, start + Duration::from_millis(2_200));
    assert_eq!(
        runtime.anchored_overlay_frames().as_slice(),
        &[presented],
        "once its removal completes the old presentation is gone"
    );
}

/// `placed_edge` reports the logical edge after any flip: a `Top` overlay that
/// flipped reads `Bottom`, and `Leading` under RTL reads `Leading` — the
/// physical right side converted back.
#[test]
fn placed_edge_reports_the_logical_edge_after_a_flip() {
    let open = Binding::container(true);
    let placed = Binding::container(AnchorEdge::Top);
    let (open_for_view, placed_for_view) = (open, placed.clone());
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let placed = placed_for_view.clone();
        // The trailing spacer fills the column, so the anchor starts at the
        // window's top edge and has no room above it.
        AnyView::new(vstack((
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, overlay_content)
                    .edge(AnchorEdge::Top)
                    .alignment(EdgeAlignment::Center)
                    .gap(GAP)
                    .flip(true)
                    .placed_edge(&placed),
            ),
            spacer(),
        )))
    });
    let mut runtime = runtime(view);
    pump_until_settled(&mut runtime);
    assert_eq!(
        placed.snapshot(),
        AnchorEdge::Bottom,
        "edge Top with no room above flipped to Bottom"
    );

    // RTL: `Leading` resolves to the anchor's physical right side, then reads
    // back as `Leading`.
    let mut env = test_environment();
    env.insert(waterui_core::layout::LayoutDirection::RightToLeft);
    let open = Binding::container(true);
    let placed = Binding::container(AnchorEdge::Top);
    let (open_for_view, placed_for_view) = (open, placed.clone());
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let placed = placed_for_view.clone();
        AnyView::new(vstack((
            ().size(0.0, 100.0),
            button("host").action(|| {}).anchored_overlay(
                AnchoredOverlay::new(&open, overlay_content)
                    .edge(AnchorEdge::Leading)
                    .alignment(EdgeAlignment::Center)
                    .gap(GAP)
                    .placed_edge(&placed),
            ),
        )))
    });
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, view, WINDOW.0, WINDOW.1, MinimalTestTheme::default());
    let anchor = bounds_of(&mut runtime, "host");
    pump_until_settled(&mut runtime);
    let frame = presented_frame(&runtime);
    assert_eq!(
        placed.snapshot(),
        AnchorEdge::Leading,
        "the written-back edge is logical: Leading, not the physical Right"
    );
    assert!(
        frame.x0 >= anchor.x1 - 0.5,
        "under RTL, Leading sits to the anchor's right, got {frame:?} for anchor {anchor:?}"
    );
}

#[cfg(feature = "menu")]
mod menu_presentation_motion_tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use accesskit::{Rect as AccessRect, Role};
    #[cfg(hydrolysis_gpu_painter)]
    use cherenkov_record::MaterialEffect;
    use cherenkov_record::{Draw as _, WorkingColor};
    use kurbo::{Rect, RoundedRect};
    use nami::Binding;
    use nami::Signal as _;
    use waterui::ViewExt as _;
    use waterui::animation::Animation;
    use waterui::graphics::Color;
    use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay};
    use waterui::theme::color::Foreground;
    use waterui_controls::button::button;
    use waterui_controls::menu::{Command, Menu, MenuItem, Shortcut};
    use waterui_core::AnyView;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::metadata::Metadata;
    use waterui_layout::frame::Frame;
    use waterui_layout::spacer::spacer;
    use waterui_layout::stack::vstack;
    use waterui_structural_transition::ReduceMotion;

    use super::{
        GAP, MinimalTestTheme, WINDOW, bounds_of, find_by_label, near, presented_frame,
        primary_click, pump_until_settled, test_environment,
    };
    use crate::num_cast::{f64_as_f32, f64_as_u32};
    use crate::platform::{InputEvent, PointerKind};
    use crate::renderer::input::PopupMenuSurface;
    use crate::theme::{
        Chrome, CornerShape, Custom, FocusIndicator, MenuLayout, MenuLook, MenuPresentationMotion,
        MenuPresentationSample, Motion, PressFeedback, StateLayer, Surface,
    };
    use crate::{HeadlessRuntime, HeadlessSnapshot};

    fn test_motion() -> MenuPresentationMotion {
        MenuPresentationMotion::new(
            Animation::spring(304.6, 33.1),
            Animation::spring(503.5, 42.6),
        )
        .content_opacity_window(0.25, 0.75)
        .content_scale_from(0.92)
        .content_scale_window(0.15, 0.85)
        .trigger_opacity_window(0.0, 0.25)
    }

    fn recording_theme(
        motion: MenuPresentationMotion,
        samples: &Rc<RefCell<Vec<MenuPresentationSample>>>,
    ) -> MinimalTestTheme {
        let recorded = Rc::clone(samples);
        let panel_fill = WorkingColor::new([0.16, 0.42, 0.86, 1.0]);
        let look = MinimalTestTheme::menu_look()
            .presentation_motion(motion)
            .presentation_panel(move |recorder, sample| {
                recorded.borrow_mut().push(*sample);
                recorder.fill(RoundedRect::from_rect(sample.surface, 12.0), panel_fill);
            });
        MinimalTestTheme::default().with_menu_look(look)
    }

    fn menu_overlay_view(
        open: Binding<bool>,
        under_pressed: Option<Binding<bool>>,
    ) -> AnyViewBuilder<AnyView> {
        AnyViewBuilder::new(move || {
            let open = open.clone();
            let under_pressed = under_pressed.clone();
            AnyView::new(vstack((
                ().size(0.0, 24.0),
                button("host").action(|| {}).anchored_overlay(
                    AnchoredOverlay::new(&open, || {
                        AnyView::new(Metadata::new(
                            Frame::new(button("Item").action(|| {}))
                                .width(120.0)
                                .height(48.0),
                            PopupMenuSurface { selected_row: None },
                        ))
                    })
                    .edge(AnchorEdge::Bottom)
                    .gap(GAP),
                ),
                ().size(0.0, 8.0),
                button("under").action(move || {
                    if let Some(pressed) = &under_pressed {
                        pressed.set(true);
                    }
                }),
            )))
        })
    }

    fn has_menu_item(runtime: &mut HeadlessRuntime) -> bool {
        runtime
            .accessibility_tree()
            .as_ref()
            .and_then(|update| find_by_label(update, Role::Button, "Item"))
            .is_some()
    }

    fn assert_present_morph(
        runtime: &mut HeadlessRuntime,
        open: &Binding<bool>,
        samples: &Rc<RefCell<Vec<MenuPresentationSample>>>,
        anchor: AccessRect,
        start: Instant,
    ) -> (Rect, Vec<u8>) {
        samples.borrow_mut().clear();
        open.set(true);
        let _ = runtime.pump_at(false, start + Duration::from_millis(16));
        let panel_frame = presented_frame(runtime);
        let first = *samples
            .borrow()
            .last()
            .expect("presentation_panel must be invoked on present");
        assert!(
            near(first.progress, 0.0),
            "first present frame starts at progress 0, got {}",
            first.progress
        );
        assert!(
            near(first.origin.width(), anchor.width())
                && near(first.origin.height(), anchor.height())
                && near(first.origin.x0, anchor.x0 - panel_frame.x0)
                && near(first.origin.y0, anchor.y0 - panel_frame.y0),
            "sample.origin must carry the trigger anchor bounds in panel space: \
             origin={:?}, anchor={anchor:?}, panel_frame={panel_frame:?}",
            first.origin
        );
        assert!(
            near(first.panel.width(), 120.0) && near(first.panel.height(), 48.0),
            "sample.panel must match the placed panel bounds, got {:?}",
            first.panel
        );
        assert!(
            !runtime.is_settled(),
            "a running present spring must keep the runtime unsettled"
        );
        assert!(
            has_menu_item(runtime),
            "presented menu items enter the accessibility tree immediately"
        );

        let mid_present = runtime
            .pump_at(true, start + Duration::from_millis(96))
            .into_snapshot()
            .expect("mid-present frame must capture");
        let mid_sample = *samples
            .borrow()
            .last()
            .expect("mid-present frame records panel");
        assert!(
            mid_sample.progress > 0.1 && mid_sample.progress < 0.99,
            "mid-present sample must be in flight, got {}",
            mid_sample.progress
        );

        let settled = runtime
            .pump_at(true, start + Duration::from_millis(1_600))
            .into_snapshot()
            .expect("settled frame must capture");
        assert!(
            runtime.is_settled(),
            "once the present spring settles the runtime must report settled"
        );
        let settled_sample = *samples
            .borrow()
            .last()
            .expect("settled frame records panel");
        assert!(
            near(settled_sample.progress, 1.0)
                && near(settled_sample.content_opacity, 1.0)
                && near(settled_sample.content_scale, 1.0)
                && near(settled_sample.trigger_opacity, 0.0),
            "settled sample must reach rest with faded trigger: {settled_sample:?}"
        );
        assert_ne!(
            mid_present.rgba8(),
            settled.rgba8(),
            "mid-present pixels must differ from settled pixels"
        );
        (panel_frame, settled.rgba8().to_vec())
    }

    /// A theme with `MenuPresentationMotion` and `presentation_panel` receives
    /// the trigger anchor's `origin`, the target `panel`, and spring-driven
    /// `progress` from `0.0 -> 1.0` on present, then retains an inert,
    /// accessibility-suppressed dismissal ghost while animating back toward
    /// `0.0` on dismiss.
    #[test]
    fn menu_presentation_morphs_from_anchor_and_retains_dismiss_ghost() {
        let open = Binding::container(false);
        let under_pressed = Binding::container(false);
        let samples = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = HeadlessRuntime::new_for_tests(
            test_environment(),
            menu_overlay_view(open.clone(), Some(under_pressed.clone())),
            WINDOW.0,
            WINDOW.1,
            recording_theme(test_motion(), &samples),
        );
        let anchor = bounds_of(&mut runtime, "host");
        let under = bounds_of(&mut runtime, "under");

        let start = Instant::now();
        let closed_snapshot = runtime
            .pump_at(true, start)
            .into_snapshot()
            .expect("closed frame must capture");
        let (panel_frame, settled_rgba) =
            assert_present_morph(&mut runtime, &open, &samples, anchor, start);

        // Dismiss: accessibility drops at once, ghost stays painted but inert,
        // and progress contracts back toward 0.0 until settled.
        open.set(false);
        let _ = runtime.pump_at(false, start + Duration::from_millis(1_616));
        assert!(
            !has_menu_item(&mut runtime),
            "dismissed menu leaves the accessibility tree immediately"
        );
        assert_eq!(
            runtime.anchored_overlay_frames().as_slice(),
            &[panel_frame],
            "dismissal ghost stays drawn while the dismiss spring runs"
        );
        let mid_dismiss = runtime
            .pump_at(true, start + Duration::from_millis(1_696))
            .into_snapshot()
            .expect("mid-dismiss frame must capture");
        let dismiss_sample = *samples
            .borrow()
            .last()
            .expect("mid-dismiss ghost records panel");
        assert!(
            dismiss_sample.progress > 0.01 && dismiss_sample.progress < 0.95,
            "mid-dismiss ghost must be contracting back toward 0, got {}",
            dismiss_sample.progress
        );
        assert_ne!(
            mid_dismiss.rgba8(),
            settled_rgba.as_slice(),
            "mid-dismiss ghost pixels must differ from settled pixels"
        );

        // A click aimed at the button underneath passes through the inert ghost.
        let click_x = f64_as_f32(f64::midpoint(under.x0, under.x1));
        let click_y = f64_as_f32(f64::midpoint(under.y0, under.y1));
        for event in primary_click(click_x, click_y) {
            runtime.push_input_event(event);
        }
        let _ = runtime.pump_at(false, start + Duration::from_millis(1_712));
        assert!(
            under_pressed.snapshot(),
            "the exiting dismissal ghost must not intercept input"
        );

        // Once the dismiss spring settles, the ghost unmounts cleanly and the
        // trigger returns to its original closed appearance.
        let _ = runtime.pump_at(false, start + Duration::from_millis(3_000));
        let final_snapshot = runtime
            .pump_at(true, start + Duration::from_millis(3_016))
            .into_snapshot()
            .expect("post-dismiss frame must capture");
        assert!(
            runtime.anchored_overlay_frames().is_empty(),
            "settled dismissal ghost must unmount"
        );
        assert!(
            runtime.is_settled(),
            "runtime must settle once the dismissal ghost unmounts"
        );
        assert_eq!(
            final_snapshot.rgba8(),
            closed_snapshot.rgba8(),
            "trigger opacity and canvas must restore cleanly after dismiss settles"
        );
    }

    /// Dismissing mid-presentation preserves the in-flight progress rather than
    /// jumping to `1.0` or snapping to `0.0`.
    #[test]
    fn interrupting_menu_presentation_mid_flight_dismisses_continuously() {
        let open = Binding::container(false);
        let samples = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = HeadlessRuntime::new_for_tests(
            test_environment(),
            menu_overlay_view(open.clone(), None),
            WINDOW.0,
            WINDOW.1,
            recording_theme(test_motion(), &samples),
        );
        pump_until_settled(&mut runtime);

        let start = Instant::now();
        let _ = runtime.pump_at(false, start);
        open.set(true);
        let _ = runtime.pump_at(false, start + Duration::from_millis(16));
        let _ = runtime.pump_at(false, start + Duration::from_millis(76));
        let before_interrupt = *samples.borrow().last().expect("mid-flight sample");
        assert!(
            before_interrupt.progress > 0.05 && before_interrupt.progress < 0.9,
            "must interrupt while mid-flight, got {}",
            before_interrupt.progress
        );

        open.set(false);
        let _ = runtime.pump_at(false, start + Duration::from_millis(92));
        let at_interrupt = *samples.borrow().last().expect("interrupt sample");
        assert!(
            (at_interrupt.progress - before_interrupt.progress).abs() < 0.25,
            "mid-flight dismissal must continue from current progress: before={}, after={}",
            before_interrupt.progress,
            at_interrupt.progress
        );
        assert_eq!(
            runtime.anchored_overlay_frames().len(),
            1,
            "interrupted presentation stays drawn as an exiting ghost"
        );

        let _ = runtime.pump_at(false, start + Duration::from_millis(2_000));
        assert!(
            runtime.anchored_overlay_frames().is_empty(),
            "interrupted dismissal ghost must unmount once settled"
        );
    }

    /// Under `ReduceMotion(true)`, `MenuPresentationMotion` snaps immediately to
    /// settled on present and removes immediately with no dismissal ghost.
    #[test]
    fn menu_presentation_motion_snaps_without_ghost_under_reduce_motion() {
        let open = Binding::container(false);
        let samples = Rc::new(RefCell::new(Vec::new()));
        let mut env = test_environment();
        env.insert(ReduceMotion(true));
        let mut runtime = HeadlessRuntime::new_for_tests(
            env,
            menu_overlay_view(open.clone(), None),
            WINDOW.0,
            WINDOW.1,
            recording_theme(test_motion(), &samples),
        );
        pump_until_settled(&mut runtime);

        let start = Instant::now();
        let _ = runtime.pump_at(false, start);
        open.set(true);
        let _ = runtime.pump_at(false, start + Duration::from_millis(16));
        let sample = *samples.borrow().last().expect("present sample");
        assert!(
            near(sample.progress, 1.0),
            "ReduceMotion must snap present progress to 1.0, got {}",
            sample.progress
        );
        assert!(
            runtime.is_settled(),
            "ReduceMotion present must settle immediately"
        );

        open.set(false);
        let _ = runtime.pump_at(false, start + Duration::from_millis(32));
        assert!(
            runtime.anchored_overlay_frames().is_empty(),
            "ReduceMotion dismiss must remove immediately without a ghost"
        );
    }

    /// `PopupMenuSurface` records through `draw_context` (layered recording),
    /// so a theme's `presentation_panel` can emit `backdrop_material` commands
    /// for glass or refractive surfaces without violating the layered contract.
    #[cfg(hydrolysis_gpu_painter)]
    #[test]
    fn menu_presentation_panel_supports_backdrop_material_recording() {
        use super::super::material_chrome::{GLASS, GLASS_SHADER, glass_plan};

        let open = Binding::container(true);
        let look = MinimalTestTheme::menu_look()
            .presentation_motion(test_motion())
            .presentation_panel(|recorder, sample| {
                recorder.backdrop_material(
                    RoundedRect::from_rect(sample.surface, 14.0),
                    GLASS_SHADER,
                    GLASS,
                    MaterialEffect::default(),
                );
            });
        let theme = MinimalTestTheme {
            chrome: glass_plan(Vec::new()),
            ..MinimalTestTheme::default()
        }
        .with_menu_look(look);
        let mut runtime = HeadlessRuntime::new_for_tests(
            test_environment(),
            menu_overlay_view(open, None),
            WINDOW.0,
            WINDOW.1,
            theme,
        );
        let start = Instant::now();
        let snapshot = runtime
            .pump_at(true, start)
            .into_snapshot()
            .expect("backdrop_material presentation panel must render without panicking");
        assert_eq!(snapshot.width(), WINDOW.0);
        assert_eq!(snapshot.height(), WINDOW.1);
        assert_eq!(
            presented_frame(&runtime).size(),
            Rect::new(0.0, 0.0, 120.0, 48.0).size()
        );
    }

    fn anisotropic_menu_motion() -> MenuPresentationMotion {
        MenuPresentationMotion::new(
            Animation::spring(484.0, 33.44),
            Animation::spring(101.4, 12.46),
        )
        .present_vertical(Animation::spring(408.0, 13.45))
        .dismiss_vertical(Animation::spring(60.7, 11.77))
        .content_opacity_window(0.25, 0.80)
        .dismiss_content_opacity_window(0.50, 0.95)
        .content_scale_from(0.92)
        .content_scale_window(0.15, 0.85)
        .content_blur_from(8.0)
        .content_blur_window(0.20, 0.80)
        .dismiss_content_blur_window(0.55, 0.95)
        .trigger_opacity_window(0.45, 0.90)
        .trigger_chrome_opacity_window(0.0, 0.0)
        .surface_corner_radius(26.0)
        .cover_trigger(true)
    }

    /// A `Menu` trigger under a theme with `MenuPresentationMotion` presents
    /// in-window over the trigger (`cover_trigger`), drives anisotropic X/Y
    /// springs, interpolates `surface_corner_radius` and `content_blur`, hides
    /// trigger chrome immediately, and contracts back on command activation.
    #[test]
    fn menu_trigger_morphs_in_window_with_anisotropic_springs_blur_and_command_dismiss() {
        let samples = Rc::new(RefCell::new(Vec::new()));
        let fired = Rc::new(Cell::new(false));
        let motion = anisotropic_menu_motion();

        let fired_for_cmd = Rc::clone(&fired);
        let view = AnyViewBuilder::new(move || {
            let fired_inner = Rc::clone(&fired_for_cmd);
            AnyView::new(vstack((
                ().size(0.0, 12.0),
                Menu::new(
                    "Actions",
                    (
                        Command::builder("Rename").action(|| {}),
                        MenuItem::Divider,
                        Command::builder("Duplicate").action(move || {
                            fired_inner.set(true);
                        }),
                    ),
                ),
                spacer(),
            )))
        });

        let mut runtime = HeadlessRuntime::new_for_tests(
            test_environment(),
            view,
            WINDOW.0,
            WINDOW.1,
            recording_theme(motion, &samples),
        );
        let start = Instant::now();
        let _ = runtime.pump_at(false, start);
        let trigger = bounds_of(&mut runtime, "Actions");

        // Tap the trigger button to open the menu in-window.
        let click_x = f64_as_f32(f64::midpoint(trigger.x0, trigger.x1));
        let click_y = f64_as_f32(f64::midpoint(trigger.y0, trigger.y1));
        for event in primary_click(click_x, click_y) {
            runtime.push_input_event(event);
        }
        let _ = runtime.pump_at(false, start + Duration::from_millis(16));
        let panel_frame = presented_frame(&runtime);
        assert!(
            near(panel_frame.y0, trigger.y0),
            "cover_trigger must align panel top to trigger top: panel={panel_frame:?}, trigger={trigger:?}"
        );
        let first = *samples.borrow().last().expect("open sample");
        assert!(
            near(first.trigger_chrome_opacity, 0.0),
            "trigger_chrome_opacity_window(0, 0) must hide trigger chrome immediately, got {}",
            first.trigger_chrome_opacity
        );
        assert!(
            first.content_blur > 7.5,
            "initial content_blur must start near 8.0, got {}",
            first.content_blur
        );

        // Mid-open: anisotropic vertical spring overshoots ahead of horizontal.
        let _ = runtime.pump_at(true, start + Duration::from_millis(116));
        let mid = *samples.borrow().last().expect("mid-open sample");
        assert!(
            (mid.vertical_progress - mid.progress).abs() > 0.01,
            "anisotropic open springs must diverge between x ({}) and y ({})",
            mid.progress,
            mid.vertical_progress
        );
        assert!(
            mid.surface_corner_radius > 0.0 && mid.surface_corner_radius <= 26.0,
            "interpolated surface_corner_radius must be in (0, 26], got {}",
            mid.surface_corner_radius
        );

        // Settled open: Duplicate item is in the accessibility tree and clickable.
        let _ = runtime.pump_at(true, start + Duration::from_millis(1_600));
        assert!(runtime.is_settled());
        let settled = *samples.borrow().last().expect("settled sample");
        assert!(near(settled.progress, 1.0) && near(settled.vertical_progress, 1.0));
        assert!(near(settled.content_blur, 0.0));
        assert!(near(settled.surface_corner_radius, 26.0));
        let dup_bounds = bounds_of(&mut runtime, "Duplicate");
        let dup_x = f64_as_f32(f64::midpoint(dup_bounds.x0, dup_bounds.x1));
        let dup_y = f64_as_f32(f64::midpoint(dup_bounds.y0, dup_bounds.y1));
        for event in primary_click(dup_x, dup_y) {
            runtime.push_input_event(event);
        }
        let _ = runtime.pump_at(false, start + Duration::from_millis(1_616));
        assert!(
            fired.get(),
            "clicking Duplicate must fire its command action"
        );

        // Dismiss ghost stays active and contracts with anisotropic springs.
        let _ = runtime.pump_at(true, start + Duration::from_millis(1_766));
        let dismissing = *samples.borrow().last().expect("dismissing sample");
        assert!(dismissing.dismissing);
        assert!(
            dismissing.progress < 0.95 && dismissing.progress > 0.0,
            "dismiss ghost must be contracting, got {}",
            dismissing.progress
        );

        // Once dismiss spring settles, overlay is removed completely.
        let _ = runtime.pump_at(false, start + Duration::from_millis(3_500));
        assert!(
            runtime.anchored_overlay_frames().is_empty(),
            "dismissed menu ghost must clear after settling"
        );
    }

    fn pixel_at(snapshot: &HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * snapshot.width() + x) * 4) as usize;
        let bytes = snapshot.rgba8();
        [
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]
    }

    fn wide_menu_runtime(
        dup_fired: &Rc<Cell<bool>>,
        archive_fired: &Rc<Cell<bool>>,
    ) -> HeadlessRuntime {
        let dup_for_cmd = Rc::clone(dup_fired);
        let archive_for_cmd = Rc::clone(archive_fired);
        let view = AnyViewBuilder::new(move || {
            let dup = Rc::clone(&dup_for_cmd);
            let archive = Rc::clone(&archive_for_cmd);
            AnyView::new(vstack((
                ().size(0.0, 12.0),
                Menu::new(
                    "Actions",
                    (
                        Command::builder("Rename").action(|| {}),
                        MenuItem::Divider,
                        Command::builder("Duplicate").action(move || {
                            dup.set(true);
                        }),
                        Command::builder("Archive")
                            .action(move || {
                                archive.set(true);
                            })
                            .shortcut(Shortcut::new('a').control()),
                    ),
                ),
                spacer(),
            )))
        });
        let layout = MenuLayout {
            min_width: 250.0,
            max_width: 280.0,
            row_height: 45.0,
            vertical_padding: 8.0,
            horizontal_padding: 20.0,
            ..MinimalTestTheme::menu_layout()
        };
        let panel_fill = WorkingColor::new([0.92, 0.92, 0.94, 1.0]);
        let look = MenuLook::new(
            layout,
            MinimalTestTheme::menu_look().panel().clone(),
            Custom::new((), |_, _, ()| {}),
        )
        .presentation_motion(anisotropic_menu_motion())
        .presentation_panel(move |recorder, sample| {
            recorder.fill(RoundedRect::from_rect(sample.surface, 12.0), panel_fill);
        });
        let hover_chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Color::new(Foreground)).opacities(0.25, 0.25, 0.35, 0.25),
            PressFeedback::new(),
            FocusIndicator::Tint,
            Motion::uniform(Animation::linear(Duration::from_millis(15))),
        );
        HeadlessRuntime::new_for_tests(
            test_environment(),
            view,
            WINDOW.0,
            WINDOW.1,
            MinimalTestTheme::default()
                .with_menu_look(look)
                .with_button_chrome(hover_chrome),
        )
    }

    /// Popup menu command rows — both plain and shortcut-bearing — must span
    /// the full panel width for accessibility bounds, hover state layers, and
    /// pointer hit-testing (water-rs/waterui#4968).
    #[test]
    fn menu_command_rows_hit_test_and_hover_across_full_panel_width() {
        let dup_fired = Rc::new(Cell::new(false));
        let archive_fired = Rc::new(Cell::new(false));
        let mut runtime = wide_menu_runtime(&dup_fired, &archive_fired);

        let mut now = Instant::now();
        let _ = runtime.pump_at(false, now);
        let trigger = bounds_of(&mut runtime, "Actions");
        let trigger_x = f64_as_f32(f64::midpoint(trigger.x0, trigger.x1));
        let trigger_y = f64_as_f32(f64::midpoint(trigger.y0, trigger.y1));

        // Open the menu and wait for the present spring to settle.
        for event in primary_click(trigger_x, trigger_y) {
            runtime.push_input_event(event);
        }
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        now += Duration::from_millis(1_600);
        let baseline = runtime
            .pump_at(true, now)
            .into_snapshot()
            .expect("settled open menu must render");
        let panel_frame = presented_frame(&runtime);
        assert!(
            panel_frame.width() >= 249.0,
            "menu panel must honor min_width 250.0, got {panel_frame:?}"
        );

        let dup_bounds = bounds_of(&mut runtime, "Duplicate");
        let archive_bounds = bounds_of(&mut runtime, "Archive");
        for (name, bounds) in [("Duplicate", dup_bounds), ("Archive", archive_bounds)] {
            assert!(
                (bounds.x0 - panel_frame.x0).abs() <= 1.0
                    && (bounds.x1 - panel_frame.x1).abs() <= 1.0,
                "{name} row bounds must span full panel width: row={bounds:?}, panel={panel_frame:?}"
            );
        }

        // Hover over the trailing empty region of the "Duplicate" row (well to
        // the right of its label glyphs) and confirm the hover state layer
        // changes pixels there.
        let trailing_x = panel_frame.x1 - 24.0;
        let dup_y = f64::midpoint(dup_bounds.y0, dup_bounds.y1);
        runtime.push_input_event(InputEvent::PointerMove {
            id: 1,
            kind: PointerKind::Mouse,
            x: f64_as_f32(trailing_x),
            y: f64_as_f32(dup_y),
        });
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        now += Duration::from_millis(100);
        let hovered = runtime
            .pump_at(true, now)
            .into_snapshot()
            .expect("hovered open menu must render");
        let px = f64_as_u32(trailing_x);
        let py = f64_as_u32(dup_y);
        assert_ne!(
            pixel_at(&baseline, px, py),
            pixel_at(&hovered, px, py),
            "hovering the trailing empty region of a menu row must paint the hover state layer at ({px}, {py})"
        );

        // Click the trailing empty region of "Duplicate" — must fire and close.
        for event in primary_click(f64_as_f32(trailing_x), f64_as_f32(dup_y)) {
            runtime.push_input_event(event);
        }
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        assert!(
            dup_fired.get(),
            "clicking the trailing empty region of Duplicate must fire its command"
        );

        // Wait for dismiss to settle, re-open, and click over Archive's trailing
        // shortcut region.
        now += Duration::from_millis(2_000);
        let _ = runtime.pump_at(false, now);
        assert_eq!(
            runtime.anchored_overlay_frames(),
            Vec::<Rect>::new(),
            "dismissed menu ghost must clear before reopening"
        );

        for event in primary_click(trigger_x, trigger_y) {
            runtime.push_input_event(event);
        }
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        now += Duration::from_millis(1_600);
        let _ = runtime.pump_at(false, now);
        let archive_bounds = bounds_of(&mut runtime, "Archive");
        let archive_y = f64::midpoint(archive_bounds.y0, archive_bounds.y1);
        for event in primary_click(f64_as_f32(trailing_x), f64_as_f32(archive_y)) {
            runtime.push_input_event(event);
        }
        now += Duration::from_millis(16);
        let _ = runtime.pump_at(false, now);
        assert!(
            archive_fired.get(),
            "clicking the trailing shortcut region of Archive must fire its command"
        );
    }
}
