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

const PUA_CODEPOINT: char = '\u{e5c4}';

fn unnamed_icon_glyph_view() -> impl waterui::View {
    IconGlyph::new(PUA_CODEPOINT, "Roboto")
}

#[waterui::test(unnamed_icon_glyph_view)]
fn unnamed_icon_glyph_emits_no_pua_or_image_node(app: &mut SemanticApp) {
    let pua = PUA_CODEPOINT.to_string();
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().role(Role::LABEL).assert_not_exists();
    app.query().label(pua).assert_not_exists();
}

fn unnamed_wrapped_icon_glyph_view() -> impl waterui::View {
    Icon::from_glyph(IconGlyph::new(PUA_CODEPOINT, "Roboto"))
}

#[waterui::test(unnamed_wrapped_icon_glyph_view)]
fn unnamed_wrapped_icon_glyph_emits_no_pua_or_image_node(app: &mut SemanticApp) {
    let pua = PUA_CODEPOINT.to_string();
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().role(Role::LABEL).assert_not_exists();
    app.query().label(pua).assert_not_exists();
}

fn named_icon_glyph_without_explicit_role_view() -> impl waterui::View {
    Icon::from_glyph(IconGlyph::new(PUA_CODEPOINT, "Roboto")).a11y_label("Remove draft")
}

#[waterui::test(named_icon_glyph_without_explicit_role_view)]
fn named_icon_glyph_exposes_image_role_without_pua(app: &mut SemanticApp) {
    let pua = PUA_CODEPOINT.to_string();
    assert_eq!(
        app.query()
            .role(Role::IMAGE)
            .label("Remove draft")
            .all()
            .len(),
        1
    );
    app.query().role(Role::LABEL).assert_not_exists();
    app.query().label(pua).assert_not_exists();
}

fn hidden_named_icon_glyph_view() -> impl waterui::View {
    Icon::from_glyph(IconGlyph::new(PUA_CODEPOINT, "Roboto"))
        .a11y_label("Remove draft")
        .a11y_hidden(true)
}

#[waterui::test(hidden_named_icon_glyph_view)]
fn hidden_icon_glyph_is_absent_from_accessibility_tree(app: &mut SemanticApp) {
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("Remove draft").assert_not_exists();
    app.query()
        .label(PUA_CODEPOINT.to_string())
        .assert_not_exists();
}

fn button_with_icon_glyph_view() -> impl waterui::View {
    waterui::component::button(
        waterui::component::label("Delete")
            .icon(Icon::from_glyph(IconGlyph::new(PUA_CODEPOINT, "Roboto"))),
    )
    .action(|| {})
}

#[waterui::test(button_with_icon_glyph_view)]
fn button_with_icon_glyph_announces_only_control_label(app: &mut SemanticApp) {
    let pua = PUA_CODEPOINT.to_string();
    assert_eq!(
        app.query().role(Role::BUTTON).label("Delete").all().len(),
        1
    );
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label(pua).assert_not_exists();
}

fn label_with_icon_glyph_view() -> impl waterui::View {
    waterui::component::label("Delete").icon(IconGlyph::new(PUA_CODEPOINT, "Roboto"))
}

#[waterui::test(label_with_icon_glyph_view)]
fn label_with_icon_glyph_suppresses_pua_glyph(app: &mut SemanticApp) {
    let pua = PUA_CODEPOINT.to_string();
    app.query()
        .role(Role::LABEL)
        .label("Delete")
        .assert_exists();
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label(pua).assert_not_exists();
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
    use waterui_testing::{OffscreenApp, Role, SemanticApp, TestStyle};

    fn unnamed_svg_icon_view() -> impl waterui::View {
        Icon::from_path("M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z", 24.0, 24.0)
    }

    #[waterui::test(unnamed_svg_icon_view)]
    fn unnamed_svg_icon_is_decorative(app: &mut SemanticApp) {
        app.query().role(Role::IMAGE).assert_not_exists();
    }

    fn named_svg_icon_without_explicit_role_view() -> impl waterui::View {
        Icon::from_path("M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z", 24.0, 24.0)
            .a11y_label("Remove draft")
    }

    #[waterui::test(named_svg_icon_without_explicit_role_view)]
    fn named_svg_icon_exposes_single_image_node(app: &mut SemanticApp) {
        assert_eq!(
            app.query()
                .role(Role::IMAGE)
                .label("Remove draft")
                .all()
                .len(),
            1
        );
    }

    fn hidden_svg_icon_view() -> impl waterui::View {
        Icon::from_path("M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z", 24.0, 24.0)
            .a11y_label("Remove draft")
            .a11y_hidden(true)
    }

    #[waterui::test(hidden_svg_icon_view)]
    fn hidden_svg_icon_is_absent(app: &mut SemanticApp) {
        app.query().role(Role::IMAGE).assert_not_exists();
        app.query().label("Remove draft").assert_not_exists();
    }

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
