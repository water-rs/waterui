//! Rendered-flush twin of `waterui-graphics`'s accessibility coverage: the
//! graphics leaves' nodes carry real bounds on the rendered runtime, and
//! decorative leaves stay out of the published tree until the application
//! names them. The semantic legs — role, label, naming precedence — live in
//! the component crate's `e2e_semantics.rs`.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation).

mod support;

use kurbo::{Rect, Shape as _};
use support::{mount_rendered, node_bounds_with_role, nodes};
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::text;
use waterui::graphics::color::Srgb;
use waterui::graphics::draw::{Draw, Paint, WorkingColor};
use waterui::layout::Size;
use waterui::reactive::constant;
use waterui::shape::{Circle, ShapeExt as _};
use waterui_core::Environment;
use waterui_graphics::{FlowingGradient, Gradient, Picture, ShaderPaintView};

fn linear_gradient_view() -> impl waterui::View {
    Gradient::linear(
        vec![
            (0.0, Srgb::from_hex("#0F172A").resolve()),
            (1.0, Srgb::from_hex("#38BDF8").resolve()),
        ],
        [0.0, 0.0],
        [1.0, 1.0],
    )
    .size(180.0, 120.0)
    .a11y_role(AccessibilityRole::Image)
    .a11y_label("Linear gradient")
}

fn animated_mesh_gradient_view() -> impl waterui::View {
    FlowingGradient::new()
        .size(180.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Animated mesh gradient")
}

fn shader_surface_view() -> impl waterui::View {
    ShaderPaintView::new(include_str!("fixtures/two_tone.wgsl"))
        .size(180.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Shader surface")
}

/// A drawing that names itself, the way an SVG with a `<title>` does.
fn labeled_picture_view() -> impl waterui::View {
    let recording = Picture::record(|recorder| {
        recorder.fill(
            Rect::new(0.0, 0.0, 24.0, 24.0).to_path(0.1),
            Paint::Solid(WorkingColor::new([0.0, 0.0, 0.0, 1.0])),
        );
    });
    Picture::new(Size::new(24.0, 24.0), constant(recording)).labeled("Warning sign")
}

/// The same drawing, named by the application instead.
fn renamed_picture_view() -> impl waterui::View {
    labeled_picture_view().a11y_label("Severe weather")
}

#[track_caller]
fn assert_image_node<V: waterui::View>(
    view: impl Fn() -> V + Clone + 'static,
    case: &str,
    label: &str,
) {
    let mut app = mount_rendered(Environment::new(), view, 320, 240);
    app.settle();
    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, label);
    assert!(bounds.width() > 0.0, "{case}: width must be positive");
    assert!(bounds.height() > 0.0, "{case}: height must be positive");
}

#[test]
fn linear_gradient_exposes_accessibility_image() {
    assert_image_node(
        linear_gradient_view,
        "linear-gradient-exposes-accessibility-image",
        "Linear gradient",
    );
}

#[test]
fn animated_mesh_gradient_exposes_accessibility_image() {
    assert_image_node(
        animated_mesh_gradient_view,
        "animated-mesh-gradient-exposes-accessibility-image",
        "Animated mesh gradient",
    );
}

#[test]
fn shader_surface_exposes_accessibility_image() {
    assert_image_node(
        shader_surface_view,
        "shader-surface-exposes-accessibility-image",
        "Shader surface",
    );
}

#[test]
fn a_picture_offers_its_own_name() {
    assert_image_node(
        labeled_picture_view,
        "a-picture-offers-its-own-name",
        "Warning sign",
    );
}

#[test]
fn the_application_label_wins_over_the_pictures_own() {
    let mut app = mount_rendered(Environment::new(), renamed_picture_view, 320, 240);
    app.settle();
    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Severe weather");
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "the renamed picture's node carries bounds"
    );
    assert!(
        !support::nodes(app.tree()).any(|(_, node)| node.label() == Some("Warning sign")),
        "the picture's own name must not reach the tree once the application named it"
    );
}

/// Decorative graphics leaves must stay out of the accessibility tree until
/// the application names them (water-rs/hydrolysis#148).
fn decorative_background_view() -> impl waterui::View {
    text("Content").background(Gradient::linear(
        vec![
            (0.0, Srgb::from_hex("#0F172A").resolve()),
            (1.0, Srgb::from_hex("#38BDF8").resolve()),
        ],
        [0.0, 0.0],
        [1.0, 1.0],
    ))
}

#[test]
fn an_unlabelled_background_fill_emits_no_accessibility_node() {
    let mut app = mount_rendered(Environment::new(), decorative_background_view, 320, 240);
    app.settle();
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == accesskit::Role::Image),
        "a decorative background fill must not emit an unnamed Image node"
    );
}

/// water-rs/hydrolysis#148.
fn decorative_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
}

#[test]
fn an_unlabelled_shape_fill_emits_no_accessibility_node() {
    let mut app = mount_rendered(Environment::new(), decorative_shape_view, 320, 240);
    app.settle();
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == accesskit::Role::Image),
        "a decorative shape fill must not emit an unnamed Image node"
    );
}

/// water-rs/hydrolysis#148.
fn labeled_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
        .a11y_label("Avatar")
}

#[test]
fn a_labelled_shape_emits_a_named_image_node() {
    assert_image_node(
        labeled_shape_view,
        "a-labelled-shape-emits-a-named-image-node",
        "Avatar",
    );
}

/// water-rs/hydrolysis#148.
fn image_role_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
        .a11y_role(AccessibilityRole::Image)
}

#[test]
fn an_explicit_image_role_keeps_the_shape_in_the_tree() {
    let mut app = mount_rendered(Environment::new(), image_role_shape_view, 320, 240);
    app.settle();
    assert!(
        nodes(app.tree()).any(|(_, node)| node.role() == accesskit::Role::Image),
        "an explicit image role keeps the leaf in the tree"
    );
}
