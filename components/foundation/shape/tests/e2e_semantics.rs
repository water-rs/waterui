//! End-to-end accessibility-semantics tests for the `shape` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui_shape::{Circle, RoundedRectangle, ShapeExt};
use waterui_testing::{Role, SemanticApp};

fn filled_circle_view() -> impl waterui::View {
    Circle
        .fill(Srgb::new(1.0, 0.2, 0.2))
        .width(72.0)
        .height(72.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Filled circle")
}

fn morph_shape_view() -> impl waterui::View {
    RoundedRectangle::new(0.3)
        .fill(Srgb::new(0.2, 0.8, 1.0))
        .morph_to(Circle)
        .width(84.0)
        .height(84.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Morph shape")
}

#[waterui::test(filled_circle_view)]
fn filled_shape_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Filled circle")
        .assert_exists();
}

#[waterui::test(morph_shape_view)]
fn morph_shape_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Morph shape")
        .assert_exists();
}
