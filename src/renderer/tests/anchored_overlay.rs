//! `.anchored_overlay(...)` (water-rs/waterui#1275), end to end through the
//! retained renderer: the shared placement contract
//! (`waterui_backend_core::overlay::place_anchored_overlay`) for an anchor in
//! the middle of the window, the flip when the preferred edge overflows, the
//! window clamp, outside-interaction dismissal versus `Manual`, and
//! re-placement when the window resizes.
//!
//! Placement assertions read `HeadlessRuntime::anchored_overlay_frames()` —
//! the hit-space rects the post-flush pass drew the overlays into.

use std::time::Instant;

use accesskit::Role;
use nami::Binding;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui::metadata::anchored_overlay::{
    AnchorEdge, AnchoredOverlay, Clamp, Dismissal, EdgeAlignment,
};
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
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
    let mut found = None;
    for _ in 0..64 {
        if let Some(update) = runtime.pump_at(false, Instant::now()).tree_update
            && let Some((_, node)) = find_by_label(&update, Role::Button, label)
        {
            found = node.bounds();
        }
        if runtime.is_settled() {
            break;
        }
    }
    found.unwrap_or_else(|| panic!("{label} must emit a button with bounds"))
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
fn presented_frame(runtime: &HeadlessRuntime) -> vello::kurbo::Rect {
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
            (anchor.x0 + anchor.x1) / 2.0 - f64::from(OVERLAY.0) / 2.0
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
    let open_for_view = open.clone();
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
    let open_for_view = open.clone();
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
        ((far.x0 + far.x1) / 2.0) as f32,
        ((far.y0 + far.y1) / 2.0) as f32,
    );
    assert!(
        !frame.contains(vello::kurbo::Point::new(f64::from(x), f64::from(y))),
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
    let open_for_view = open.clone();
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
    let open_for_view = open.clone();
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
