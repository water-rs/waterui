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

fn unnamed_delete_icon_view() -> impl waterui::View {
    waterui_icons_material_icon::delete()
}

#[waterui::test(unnamed_delete_icon_view)]
fn unnamed_material_icon_is_decorative(app: &mut SemanticApp) {
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("delete").assert_not_exists();
}

fn labeled_delete_icon_view() -> impl waterui::View {
    waterui_icons_material_icon::delete().a11y_label("Remove draft")
}

#[waterui::test(labeled_delete_icon_view)]
fn labeled_material_icon_exposes_single_image_node(app: &mut SemanticApp) {
    assert_eq!(
        app.query()
            .role(Role::IMAGE)
            .label("Remove draft")
            .all()
            .len(),
        1
    );
    app.query().label("delete").assert_not_exists();
}

fn hidden_delete_icon_view() -> impl waterui::View {
    waterui_icons_material_icon::delete()
        .a11y_label("Remove draft")
        .a11y_hidden(true)
}

#[waterui::test(hidden_delete_icon_view)]
fn hidden_material_icon_is_absent(app: &mut SemanticApp) {
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("Remove draft").assert_not_exists();
}

fn delete_button_with_material_icon() -> impl waterui::View {
    waterui::component::button(
        waterui::component::label("Delete").icon(waterui_icons_material_icon::delete()),
    )
    .action(|| {})
}

#[waterui::test(delete_button_with_material_icon)]
fn button_with_material_icon_announces_only_control_label(app: &mut SemanticApp) {
    assert_eq!(
        app.query().role(Role::BUTTON).label("Delete").all().len(),
        1
    );
    app.query().role(Role::IMAGE).assert_not_exists();
    app.query().label("delete").assert_not_exists();
}
