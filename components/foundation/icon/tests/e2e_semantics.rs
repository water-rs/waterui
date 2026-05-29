use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_icon::{IconGlyph, system_icon};
use waterui_testing::{Role, SemanticApp};

fn icon_glyph_view() -> impl waterui::View {
    IconGlyph::new('\u{2605}', "Helvetica")
        .with_size(24.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Glyph icon")
}

fn system_icon_view() -> impl waterui::View {
    system_icon::star()
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("System icon")
}

#[waterui::test(icon_glyph_view)]
fn icon_glyph_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Glyph icon")
        .assert_exists();
}

#[waterui::test(system_icon_view)]
fn system_icon_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("System icon")
        .assert_exists();
}
