//! End-to-end visual-rendering tests for the `canvas` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui::layout::{Point, Rect, Size};
use waterui_canvas::Canvas;
use waterui_testing::{Role, SemanticApp};

fn canvas_fill_rect_view() -> impl waterui::View {
    Canvas::new(|ctx| {
        ctx.set_fill_style(Srgb::new(1.0, 0.25, 0.1));
        ctx.fill_rect(Rect::new(Point::new(20.0, 16.0), Size::new(100.0, 60.0)));
    })
    .size(160.0, 100.0)
    .background(Srgb::BLACK)
    .a11y_role(AccessibilityRole::Image)
    .a11y_label("Filled rect canvas")
}

#[waterui::test(canvas_fill_rect_view)]
fn canvas_fill_rect_exposes_accessibility_node(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Filled rect canvas")
        .assert_exists();
}
