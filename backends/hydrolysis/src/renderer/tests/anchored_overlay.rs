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
                AnchoredOverlay::new(&open, overlay_content())
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
/// not fit above it; `flip` moves it to the bottom edge.
#[test]
fn an_overlay_too_tall_for_the_top_edge_flips_below() {
    let open = Binding::container(true);
    let open_for_view = open;
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, overlay_content())
                .edge(AnchorEdge::Top)
                .alignment(EdgeAlignment::Center)
                .gap(GAP)
                .flip(true)
                .clamp(Clamp::Window { margin: MARGIN }),
        ),)))
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
                .width(WINDOW.0 as f32)
                .height(20.0)
                .anchored_overlay(
                    AnchoredOverlay::new(&open, overlay_content())
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
                AnchoredOverlay::new(&open, overlay_content())
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
        f64::midpoint(far.x0, far.x1) as f32,
        f64::midpoint(far.y0, far.y1) as f32,
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
                AnchoredOverlay::new(&open, overlay_content())
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
                .width(WINDOW.0 as f32)
                .height(20.0)
                .anchored_overlay(
                    AnchoredOverlay::new(&open, overlay_content())
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
                AnchoredOverlay::new(
                    &open,
                    AnyView::new(text(
                        "A long line of overlay text that cannot fit on a \
                         single line inside a narrow window.",
                    )),
                )
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
            AnchoredOverlay::new(
                &open,
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
                ),
            )
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
            AnchoredOverlay::new(&open, overlay_content())
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
            AnchoredOverlay::new(
                &open,
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
                ),
            )
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
    let snapshot = result
        .snapshot
        .expect("a mid-exit frame must be capturable");
    let path = waterui_testing::TestArtifacts::new("hydrolysis")
        .snapshot_path("anchored_overlay_exit", "mid-flight");
    std::fs::create_dir_all(path.parent().expect("the case directory"))
        .expect("the capture directory must be creatable");
    image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.rgba8)
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
                AnchoredOverlay::new(
                    &open,
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
                    ),
                )
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
    for event in primary_click(point.x as f32, point.y as f32) {
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

    for event in primary_click(point.x as f32, point.y as f32) {
        runtime.push_input_event(event);
    }
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    assert!(
        under_pressed.snapshot(),
        "the exiting overlay no longer intercepts input"
    );
}

/// `is_presented` back to `true` mid-exit keeps the same overlay presented —
/// the exit is cancelled, not restarted.
#[test]
fn re_presenting_mid_exit_keeps_the_overlay_presented() {
    let open = Binding::container(true);
    let open_for_view = open.clone();
    let scale = Binding::f32(1.0);
    let scale_for_view = scale.clone();
    let view = AnyViewBuilder::<AnyView>::new(move || {
        let open = open_for_view.clone();
        let scale = scale_for_view.clone();
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(
                &open,
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
                ),
            )
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
    pump_until_settled(&mut runtime);

    let after = presented_frame(&runtime);
    assert_eq!(
        before, after,
        "the re-presented overlay keeps the same subtree at the same frame"
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
        AnyView::new(vstack((button("host").action(|| {}).anchored_overlay(
            AnchoredOverlay::new(&open, overlay_content())
                .edge(AnchorEdge::Top)
                .alignment(EdgeAlignment::Center)
                .gap(GAP)
                .flip(true)
                .placed_edge(&placed),
        ),)))
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
                AnchoredOverlay::new(&open, overlay_content())
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
