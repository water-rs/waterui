//! First accessibility-semantics coverage for the Lucide icon set.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui_testing::{OffscreenApp, Role, SemanticApp, TestStyle};

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

fn large_icon_view() -> impl waterui::View {
    waterui_icons_lucide::house()
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Large home")
        .size(48.0, 48.0)
}

/// A pack icon asked to be 48pt must lay out at 48pt, not stay at its 24pt viewBox.
#[waterui::test(large_icon_view, theme = TestStyle::new(), offscreen, viewport = (120, 120))]
fn repro_19_icon_larger_than_viewbox(app: &mut OffscreenApp) {
    let icon = app.query().role(Role::IMAGE).label("Large home").single();
    let bounds = icon.bounds();
    assert!(
        bounds.width() >= 48.0 && bounds.height() >= 48.0,
        "icon sized to 48x48 laid out at {}x{}",
        bounds.width(),
        bounds.height()
    );
}

fn unnamed_trash_icon_view() -> impl waterui::View {
    waterui_icons_lucide::trash()
}

#[waterui::test(unnamed_trash_icon_view)]
fn unnamed_lucide_icon_emits_no_slug_or_image_node(app: &mut SemanticApp) {
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("trash").assert_not_exists();
}

fn labeled_trash_icon_view() -> impl waterui::View {
    waterui_icons_lucide::trash().a11y_label("Remove draft")
}

#[waterui::test(labeled_trash_icon_view)]
fn labeled_lucide_icon_exposes_single_image_node(app: &mut SemanticApp) {
    assert_eq!(
        app.query()
            .role(Role::IMAGE)
            .label("Remove draft")
            .all()
            .len(),
        1
    );
    app.query().label("trash").assert_not_exists();
}

fn hidden_trash_icon_view() -> impl waterui::View {
    waterui_icons_lucide::trash()
        .a11y_label("Remove draft")
        .a11y_hidden(true)
}

#[waterui::test(hidden_trash_icon_view)]
fn hidden_lucide_icon_is_absent(app: &mut SemanticApp) {
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("Remove draft").assert_not_exists();
}

fn delete_button_with_lucide_icon() -> impl waterui::View {
    waterui::component::button(
        waterui::component::label("Delete").icon(waterui_icons_lucide::trash()),
    )
    .action(|| {})
}

#[waterui::test(delete_button_with_lucide_icon)]
fn button_with_lucide_icon_announces_only_control_label(app: &mut SemanticApp) {
    assert_eq!(
        app.query().role(Role::BUTTON).label("Delete").all().len(),
        1
    );
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("trash").assert_not_exists();
}

fn unsized_icon_view() -> impl waterui::View {
    waterui::component::scroll::scroll_both(
        waterui_icons_lucide::house()
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("Unsized home"),
    )
}

#[waterui::test(unsized_icon_view, theme = TestStyle::new(), offscreen, viewport = (120, 120))]
fn unsized_lucide_icon_defaults_to_viewbox_size(app: &mut OffscreenApp) {
    let bounds = app
        .query()
        .role(Role::IMAGE)
        .label("Unsized home")
        .single()
        .bounds();
    assert!(
        (bounds.width() - 24.0).abs() < 0.5 && (bounds.height() - 24.0).abs() < 0.5,
        "unsized icon should lay out at its 24x24 viewBox, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}
