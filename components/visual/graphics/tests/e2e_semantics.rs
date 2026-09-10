//! Accessibility semantics coverage for graphics views.

use kurbo::{Affine, Rect, Shape as _};
use peniko::{Brush, Color, Fill};
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui::layout::Size;
use waterui::reactive::constant;
use waterui_graphics::{
    AnimatedMeshGradient, AnimatedMeshGradientConfig, Gradient, Picture, ShaderSurface,
};
use waterui_testing::{Role, SemanticApp};

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
    AnimatedMeshGradient::new(AnimatedMeshGradientConfig::soft_blush())
        .size(180.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Animated mesh gradient")
}

fn shader_surface_view() -> impl waterui::View {
    ShaderSurface::new(include_str!("fixtures/two_tone.wgsl"))
        .size(180.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Shader surface")
}

/// A drawing that names itself, the way an SVG with a `<title>` does.
fn labeled_picture_view() -> impl waterui::View {
    let recording = Picture::record(|scene| {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(Color::BLACK),
            None,
            &Rect::new(0.0, 0.0, 24.0, 24.0).to_path(0.1),
        );
    });
    Picture::new(Size::new(24.0, 24.0), constant(recording)).labeled("Warning sign")
}

/// The same drawing, named by the application instead.
fn renamed_picture_view() -> impl waterui::View {
    labeled_picture_view().a11y_label("Severe weather")
}

fn assert_image_node(app: &mut SemanticApp, case: &str, label: &str) {
    let node = app.query().role(Role::IMAGE).label(label).single();
    let bounds = node.bounds();
    assert!(bounds.width() > 0.0, "{case}: width must be positive");
    assert!(bounds.height() > 0.0, "{case}: height must be positive");
}

#[waterui::test(linear_gradient_view)]
fn linear_gradient_exposes_accessibility_image(app: &mut SemanticApp) {
    assert_image_node(
        app,
        "linear-gradient-exposes-accessibility-image",
        "Linear gradient",
    );
}

#[waterui::test(animated_mesh_gradient_view)]
fn animated_mesh_gradient_exposes_accessibility_image(app: &mut SemanticApp) {
    assert_image_node(
        app,
        "animated-mesh-gradient-exposes-accessibility-image",
        "Animated mesh gradient",
    );
}

#[waterui::test(shader_surface_view)]
fn shader_surface_exposes_accessibility_image(app: &mut SemanticApp) {
    assert_image_node(
        app,
        "shader-surface-exposes-accessibility-image",
        "Shader surface",
    );
}

#[waterui::test(labeled_picture_view)]
fn a_picture_offers_its_own_name(app: &mut SemanticApp) {
    assert_image_node(app, "a-picture-offers-its-own-name", "Warning sign");
}

#[waterui::test(renamed_picture_view)]
fn the_application_label_wins_over_the_pictures_own(app: &mut SemanticApp) {
    assert_image_node(
        app,
        "the-application-label-wins-over-the-pictures-own",
        "Severe weather",
    );
    assert!(
        !app.query().label("Warning sign").exists(),
        "the picture's own name must not reach the tree once the application named it"
    );
}
