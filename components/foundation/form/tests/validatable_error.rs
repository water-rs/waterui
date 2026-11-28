//! A `ValidatableView` exposes validation errors through `AccessibilityState`
//! and omits its error row while valid.

use std::time::Duration;

use waterui::{Binding, View};
use waterui_controls::text_field::TextField;
use waterui_form::valid::{Plain, Required, ValidatableView};
use waterui_testing::{Role, SemanticApp};

fn email_view() -> impl View {
    let value = Binding::new(waterui::Str::from("a@b.c"));
    ValidatableView::new(TextField::new("Email", &value), Plain(Required))
}

#[waterui::test(email_view)]
fn invalid_write_shows_error(app: &mut SemanticApp) {
    // While valid, the field is not invalid and no empty error label exists
    // (only the field's own "Email" label).
    let initial = app.query().role(Role::TEXT_INPUT).label("Email").single();
    assert!(!initial.node().invalid());
    assert_eq!(initial.node().error_message(), None);
    assert_eq!(
        app.query().role(Role::LABEL).single().node().label(),
        Some("Email")
    );

    // Clearing the value fails `Required`: the field reports invalid + error text.
    app.query()
        .role(Role::TEXT_INPUT)
        .label("Email")
        .set_text("");
    app.query()
        .role(Role::LABEL)
        .label("Value is required.")
        .assert_exists_within(Duration::from_millis(200));

    let invalid_field = app.query().role(Role::TEXT_INPUT).label("Email").single();
    assert!(invalid_field.node().invalid());
    assert_eq!(
        invalid_field.node().error_message(),
        Some("Value is required.")
    );
    // Visual error row is hidden from accessibility so only the linked error
    // message node exists.
    let _ = app
        .query()
        .role(Role::LABEL)
        .label("Value is required.")
        .single();

    // Restoring a valid value clears invalid state and omits the error node.
    app.query()
        .role(Role::TEXT_INPUT)
        .label("Email")
        .set_text("user@example.com");
    app.query()
        .role(Role::LABEL)
        .label("Value is required.")
        .assert_not_exists_within(Duration::from_millis(200));
    assert_eq!(
        app.query().role(Role::LABEL).single().node().label(),
        Some("Email")
    );
    let valid_field = app.query().role(Role::TEXT_INPUT).label("Email").single();
    assert!(!valid_field.node().invalid());
    assert_eq!(valid_field.node().error_message(), None);
}
