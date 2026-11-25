//! Semantic coverage for secure form fields.

use std::time::Duration;

use waterui::layout::stack::vstack;
use waterui::text::Text;
use waterui::{Binding, SignalExt, View};
use waterui_form::secure;
use waterui_form::secure::Secure;
use waterui_testing::{Role, SemanticApp};

fn secure_view() -> impl View {
    let password = Binding::container(Secure::default());

    vstack((
        secure("Password", &password),
        Text::display(password.map(|password: Secure| format!("len:{}", password.expose().len()))),
    ))
}

#[waterui::test(secure_view)]
fn secure_field_set_text_updates_value(app: &mut SemanticApp) {
    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .assert_exists();
    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .set_text("hunter2");
    app.query()
        .role(Role::LABEL)
        .label("len:7")
        .assert_exists_within(Duration::from_millis(200));
}
