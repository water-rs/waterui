//! Accessibility bounds follow a transform binding: a `.scale` or `.offset`
//! whose binding changes moves the view's placement as a live operand without
//! re-recording, so the published bounds are re-resolved on that placement
//! clock instead of staying at the last flushed geometry.

mod support;

use accesskit::Rect;
use support::{RenderedApp, mount_rendered, node_bounds};
use waterui::component::text;
use waterui::{Binding, ViewExt as _};
use waterui_core::Environment;
use waterui_layout::frame::Frame;

const WIDTH: u32 = 400;
const HEIGHT: u32 = 600;

/// The bounds the runtime currently publishes for the node labelled `label`.
fn published(app: &mut RenderedApp, label: &str) -> Rect {
    let tree = app
        .runtime_mut()
        .accessibility_tree()
        .expect("a mounted window emits a tree");
    node_bounds(&tree, label)
}

#[test]
fn accessible_bounds_follow_a_scale_binding() {
    let factor = Binding::container(1.0_f32);
    let view_factor = factor.clone();
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            Frame::new(text("Hello"))
                .width(100.0)
                .height(40.0)
                .a11y_label("Target")
                .scale(view_factor.clone(), view_factor.clone())
        },
        WIDTH,
        HEIGHT,
    );
    app.settle();
    let before = published(&mut app, "Target");

    factor.set(2.0);
    app.settle();
    let after = published(&mut app, "Target");

    assert!(
        after.width() > before.width() * 1.5 && after.height() > before.height() * 1.5,
        "scaling to 2x grows the published bounds: before={before:?} after={after:?}"
    );
}

#[test]
fn accessible_bounds_follow_an_offset_binding() {
    let shift = Binding::container(0.0_f32);
    let view_shift = shift.clone();
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            Frame::new(text("Hello"))
                .width(100.0)
                .height(40.0)
                .a11y_label("Target")
                .offset(view_shift.clone(), 0.0)
        },
        WIDTH,
        HEIGHT,
    );
    app.settle();
    let before = published(&mut app, "Target");

    shift.set(50.0);
    app.settle();
    let after = published(&mut app, "Target");

    assert!(
        (after.x0 - before.x0 - 50.0).abs() < 0.5,
        "offsetting by 50 moves the published bounds: before={before:?} after={after:?}"
    );
}
