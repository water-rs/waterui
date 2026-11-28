//! Semantic coverage for secure form fields.

use std::time::Duration;

use waterui::layout::stack::vstack;
use waterui::text::Text;
use waterui::{Binding, SignalExt, View};
use waterui_form::secure;
use waterui_form::secure::{ContentType, Secure, SubmitLabel};
use waterui_testing::{ContentType as SnapshotContentType, Role, SemanticApp};

fn secure_view() -> impl View {
    let password = Binding::new(Secure::default());
    let new_password = Binding::new(Secure::default());
    let submitted = Binding::new(false);

    vstack((
        secure("Password", &password)
            .prompt("Enter your password")
            .on_submit({
                let submitted = submitted.clone();
                move || submitted.set(true)
            }),
        secure("New Password", &new_password)
            .prompt("At least 12 characters")
            .content_type(ContentType::NewPassword)
            .submit_label(SubmitLabel::Done),
        Text::display(password.map(|password: Secure| format!("len:{}", password.expose().len()))),
        Text::display(submitted.map(|done| format!("submitted:{done}"))),
    ))
}

#[waterui::test(secure_view)]
fn secure_field_set_text_updates_value(app: &mut SemanticApp) {
    let password = app
        .query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .single();
    assert_eq!(
        password.node().content_type(),
        Some(SnapshotContentType::Password),
        "SecureField must default to ContentType::Password in the accessibility snapshot"
    );
    assert_eq!(
        password.node().placeholder(),
        Some("Enter your password"),
        "SecureField prompt must reach the accessibility placeholder"
    );

    let new_password = app
        .query()
        .role(Role::PASSWORD_INPUT)
        .label("New Password")
        .single();
    assert_eq!(
        new_password.node().content_type(),
        Some(SnapshotContentType::NewPassword),
        "explicit ContentType::NewPassword must reach the accessibility snapshot"
    );
    assert_eq!(
        new_password.node().placeholder(),
        Some("At least 12 characters")
    );

    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .set_text("hunter2");
    app.query()
        .role(Role::LABEL)
        .label("len:7")
        .assert_exists_within(Duration::from_millis(200));

    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .focus();
    app.press_named_key("Enter");
    app.query()
        .role(Role::LABEL)
        .label("submitted:true")
        .assert_exists_within(Duration::from_millis(200));
}
