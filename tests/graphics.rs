//! Renderer presentation tests for graphics views: the image nodes' bounds on
//! the rendered runtime.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); every case names its origin file and asserts what
//! it asserted there, mounted under `Material3::defaults()` on the rendered
//! runtime.

use kurbo::{Rect, Shape as _};
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::text;
use waterui::graphics::cherenkov::{Draw, Paint, WorkingColor};
use waterui::graphics::color::Srgb;
use waterui::layout::Size;
use waterui::reactive::constant;
use waterui::shape::{Circle, ShapeExt as _};
use waterui_graphics::{Gradient, Picture, ShaderPaintView};
use waterui_testing::{OffscreenApp, Role};

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
    ShaderPaintView::flowing_gradient()
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

fn assert_image_node(app: &mut OffscreenApp, case: &str, label: &str) {
    let node = app.query().role(Role::IMAGE).label(label).single();
    let bounds = node.bounds();
    assert!(bounds.width() > 0.0, "{case}: width must be positive");
    assert!(bounds.height() > 0.0, "{case}: height must be positive");
}

// Origin: waterui `components/visual/graphics/tests/e2e_semantics.rs`.
#[waterui::test(linear_gradient_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn linear_gradient_exposes_accessibility_image(app: &mut OffscreenApp) {
    assert_image_node(
        app,
        "linear-gradient-exposes-accessibility-image",
        "Linear gradient",
    );
}

// Origin: waterui `components/visual/graphics/tests/e2e_semantics.rs`.
#[waterui::test(animated_mesh_gradient_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn animated_mesh_gradient_exposes_accessibility_image(app: &mut OffscreenApp) {
    assert_image_node(
        app,
        "animated-mesh-gradient-exposes-accessibility-image",
        "Animated mesh gradient",
    );
}

// Origin: waterui `components/visual/graphics/tests/e2e_semantics.rs`.
#[waterui::test(shader_surface_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn shader_surface_exposes_accessibility_image(app: &mut OffscreenApp) {
    assert_image_node(
        app,
        "shader-surface-exposes-accessibility-image",
        "Shader surface",
    );
}

// Origin: waterui `components/visual/graphics/tests/e2e_semantics.rs`.
#[waterui::test(labeled_picture_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn a_picture_offers_its_own_name(app: &mut OffscreenApp) {
    assert_image_node(app, "a-picture-offers-its-own-name", "Warning sign");
}

// Origin: waterui `components/visual/graphics/tests/e2e_semantics.rs`.
#[waterui::test(renamed_picture_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn the_application_label_wins_over_the_pictures_own(app: &mut OffscreenApp) {
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

// Origin: water-rs/hydrolysis#148 — decorative graphics leaves must stay out
// of the accessibility tree until the application names them.
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

#[waterui::test(decorative_background_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn an_unlabelled_background_fill_emits_no_accessibility_node(app: &mut OffscreenApp) {
    assert!(
        !app.query().role(Role::IMAGE).exists(),
        "a decorative background fill must not emit an unnamed Image node"
    );
}

// Origin: water-rs/hydrolysis#148.
fn decorative_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
}

#[waterui::test(decorative_shape_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn an_unlabelled_shape_fill_emits_no_accessibility_node(app: &mut OffscreenApp) {
    assert!(
        !app.query().role(Role::IMAGE).exists(),
        "a decorative shape fill must not emit an unnamed Image node"
    );
}

// Origin: water-rs/hydrolysis#148.
fn labeled_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
        .a11y_label("Avatar")
}

#[waterui::test(labeled_shape_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn a_labelled_shape_emits_a_named_image_node(app: &mut OffscreenApp) {
    assert_image_node(app, "a-labelled-shape-emits-a-named-image-node", "Avatar");
}

// Origin: water-rs/hydrolysis#148.
fn image_role_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
        .a11y_role(AccessibilityRole::Image)
}

#[waterui::test(image_role_shape_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
fn an_explicit_image_role_keeps_the_shape_in_the_tree(app: &mut OffscreenApp) {
    assert!(
        app.query().role(Role::IMAGE).exists(),
        "an explicit image role keeps the leaf in the tree"
    );
}
