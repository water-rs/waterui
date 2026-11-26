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

#[cfg(feature = "svg")]
mod svg_sizing {
    use waterui::ViewExt as _;
    use waterui::accessibility::AccessibilityRole;
    use waterui_icon::Icon;
    use waterui_testing::{OffscreenApp, Role, TestStyle};

    fn large_viewbox_icon_sized() -> impl waterui::View {
        Icon::from_path("M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z", 512.0, 512.0)
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Large viewbox sized")
            .size(48.0, 48.0)
    }

    #[waterui::test(large_viewbox_icon_sized, theme = TestStyle::new(), offscreen, viewport = (120, 120))]
    fn large_viewbox_icon_honors_explicit_size_above_default(app: &mut OffscreenApp) {
        let bounds = app
            .query()
            .role(Role::IMAGE)
            .label("Large viewbox sized")
            .single()
            .bounds();
        assert!(
            (bounds.width() - 48.0).abs() < 0.5 && (bounds.height() - 48.0).abs() < 0.5,
            "512x512 viewBox icon sized to 48x48 laid out at {}x{}",
            bounds.width(),
            bounds.height()
        );
    }

    fn large_viewbox_icon_unsized() -> impl waterui::View {
        waterui::component::scroll::scroll_both(
            Icon::from_path("M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z", 512.0, 512.0)
                .a11y_role(AccessibilityRole::Image)
                .a11y_label("Large viewbox unsized"),
        )
    }

    #[waterui::test(large_viewbox_icon_unsized, theme = TestStyle::new(), offscreen, viewport = (120, 120))]
    fn large_viewbox_icon_unsized_defaults_to_24pt(app: &mut OffscreenApp) {
        let bounds = app
            .query()
            .role(Role::IMAGE)
            .label("Large viewbox unsized")
            .single()
            .bounds();
        assert!(
            (bounds.width() - 24.0).abs() < 0.5 && (bounds.height() - 24.0).abs() < 0.5,
            "unsized 512x512 viewBox icon should lay out at 24x24, got {}x{}",
            bounds.width(),
            bounds.height()
        );
    }
}
