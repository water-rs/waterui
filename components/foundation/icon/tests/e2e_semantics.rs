//! End-to-end accessibility-semantics tests for the `icon` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_icon::IconGlyph;
use waterui_testing::{Role, SemanticApp};

fn icon_glyph_view() -> impl waterui::View {
    // Roboto is the deterministic family every test host bundles; a system
    // family here would resolve on one OS and not the next.
    IconGlyph::new('\u{2605}', "Roboto")
        .with_size(24.0)
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

// `system_icon` is deliberately not covered here. This suite runs on
// Hydrolysis, and a self-drawn backend has no OS-supplied icon catalog, so
// rendering a `SystemIcon` panics by design — the asymmetry is documented
// rather than papered over (see the framework principle on asymmetric
// primitives). `IconGlyph` above covers the accessibility path with an icon
// source every backend can render; `SystemIcon` belongs in an Apple-backend
// test.
