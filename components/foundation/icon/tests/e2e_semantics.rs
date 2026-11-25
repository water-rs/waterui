//! End-to-end accessibility-semantics tests for the `icon` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_icon::IconGlyph;
use waterui_testing::{Role, SemanticApp};

fn icon_glyph_view() -> impl waterui::View {
    // Roboto is the deterministic family every test host bundles; a system
    // family here would resolve on one OS and not the next.
    IconGlyph::new('\u{2605}', "Roboto")
        .with_font_size(24.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Glyph icon")
}

#[waterui::test(icon_glyph_view)]
fn icon_glyph_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Glyph icon")
        .assert_exists();
}

#[test]
fn icon_glyph_font_size_does_not_shadow_view_ext_size() {
    let glyph = IconGlyph::new('\u{2605}', "Roboto").with_font_size(20.0);
    assert!((glyph.font_size() - 20.0).abs() < f32::EPSILON);
    let _framed = glyph.size(32.0, 32.0);
}
