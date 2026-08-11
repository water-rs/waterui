//! Semantic coverage for secure form fields.

use std::time::Duration;

use waterui::layout::stack::vstack;
use waterui::text::Text;
use waterui::{Binding, SignalExt, View};
use waterui_form::secure;
use waterui_form::secure::Secure;
use waterui_testing::{Role, Selector, SemanticApp};

fn secure_view() -> impl View {
    let password = Binding::container(Secure::default());

    vstack((
        secure("Password", &password),
        Text::display(password.map(|password: Secure| format!("len:{}", password.expose().len()))),
    ))
}

#[waterui::test(secure_view, theme = hydrolysis_m3::install)]
fn secure_field_set_text_updates_value(app: &mut SemanticApp) {
    let selector = Selector::default()
        .role(Role::PASSWORD_INPUT)
        .label("Password");
    app.assert_exists(&selector);
    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .set_text("hunter2");
    assert!(
        app.wait_for(
            &[app.expect_exists(Selector::default().role(Role::LABEL).label("len:7"))],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ) == waterui_testing::WaitResult::Completed,
        "secure field binding should update after set_text"
    );
}
