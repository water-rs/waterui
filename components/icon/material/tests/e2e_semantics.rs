//! First accessibility-semantics coverage for the Material icon set.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_testing::{Role, SemanticApp};

fn search_icon_view() -> impl waterui::View {
    waterui_icons_material_icon::magnify()
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Search")
}

#[waterui::test(search_icon_view)]
fn material_icon_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Search")
        .assert_exists();
}
