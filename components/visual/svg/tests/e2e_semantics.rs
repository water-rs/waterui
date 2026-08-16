//! End-to-end accessibility-semantics tests for the `svg` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui_svg::Svg;
use waterui_testing::{Role, SemanticApp};

fn filled_svg_view() -> impl waterui::View {
    Svg::from_path("M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26Z", 24.0, 24.0)
        .tint(Srgb::new(1.0, 0.8, 0.1))
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Filled svg")
}

fn stroke_svg_view() -> impl waterui::View {
    Svg::from_stroke_path("M3 12h18M3 6h18M3 18h18", 24.0, 24.0)
        .tint(Srgb::WHITE)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Stroke svg")
}

/// An icon whose artwork is drawn in a 24pt box, asked to occupy 8pt.
///
/// `.size` here must be `ViewExt`'s layout size. `Svg` used to carry an
/// inherent `size` that reframed the artwork instead, and being inherent it
/// won name resolution — so this is also a guard against that returning.
fn resized_svg_view() -> impl waterui::View {
    Svg::from_path("M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z", 24.0, 24.0)
        .tint(Srgb::WHITE)
        .size(8.0, 8.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Resized svg")
}

#[waterui::test(filled_svg_view)]
fn filled_svg_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Filled svg")
        .assert_exists();
}

#[waterui::test(resized_svg_view)]
fn svg_intrinsic_size_yields_to_an_explicit_size(app: &mut SemanticApp) {
    let bounds = app
        .query()
        .role(Role::IMAGE)
        .label("Resized svg")
        .single()
        .bounds();

    // An intrinsic size is an ideal, not a floor: `.size(8, 8)` wins, and the
    // drawing stays inside the box instead of spilling over its neighbours.
    assert!(
        (bounds.width() - 8.0).abs() < 0.5 && (bounds.height() - 8.0).abs() < 0.5,
        "sized svg should occupy its requested 8x8 box, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

#[waterui::test(stroke_svg_view)]
fn stroke_svg_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Stroke svg")
        .assert_exists();
}
