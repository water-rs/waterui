//! First accessibility-semantics coverage for the Lucide icon set.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_testing::{Role, SemanticApp};

fn home_icon_view() -> impl waterui::View {
    waterui_icons_lucide::house()
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Home")
}

#[waterui::test(home_icon_view)]
fn lucide_icon_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query().role(Role::IMAGE).label("Home").assert_exists();
}

fn labeled_icon_pair_view() -> impl waterui::View {
    waterui::component::vstack((
        waterui_icons_lucide::settings()
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Settings"),
        waterui_icons_lucide::user()
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Profile")
            .a11y_id("nav.profile"),
    ))
}

#[waterui::test(labeled_icon_pair_view)]
fn lucide_icons_keep_distinct_labels_and_identifiers(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Settings")
        .assert_exists();
    let profile = app.query().identifier("nav.profile").single();
    assert_eq!(profile.node().label(), Some("Profile"));
}
