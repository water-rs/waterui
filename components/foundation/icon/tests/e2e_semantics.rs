//! End-to-end accessibility-semantics tests for the `icon` component.

use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Color;
use waterui::{Signal as _, ViewExt as _};
use waterui_icon::{Icon, IconGlyph, IconSize};
use waterui_testing::{Role, SemanticApp};

fn icon_glyph_view() -> impl waterui::View {
    // Roboto is the deterministic family every test host bundles; a system
    // family here would resolve on one OS and not the next.
    Icon::from_glyph(IconGlyph::new('\u{2605}', "Roboto"))
        .icon_size(24.0)
        .foreground(Color::red())
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

#[test]
fn icon_size_channel_defaults_and_stores_signal() {
    let default = IconSize::default();
    assert!((default.size().snapshot() - 24.0).abs() < f32::EPSILON);
    let custom = IconSize::new(18.0);
    assert!((custom.size().snapshot() - 18.0).abs() < f32::EPSILON);
}
