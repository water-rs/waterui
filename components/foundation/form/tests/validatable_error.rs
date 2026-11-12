//! An invalid write through a `ValidatableView` must surface its error text.

use std::time::Duration;

use waterui::{Binding, View};
use waterui_controls::text_field::TextField;
use waterui_form::valid::{Plain, Required, ValidatableView};
use waterui_testing::{Role, Selector, SemanticApp};

fn email_view() -> impl View {
    let value = Binding::container(waterui::Str::from("a@b.c"));
    ValidatableView::new(TextField::new("Email", &value), Plain(Required))
}

#[waterui::test(email_view)]
fn invalid_write_shows_error(app: &mut SemanticApp) {
    app.query()
        .role(Role::TEXT_INPUT)
        .label("Email")
        .set_text("");
    let error = Selector::default()
        .role(Role::LABEL)
        .label("Value is required.");
    assert!(
        app.wait_for(
            &[app.expect_exists(error)],
            waterui_testing::WaitOptions::new(Duration::from_millis(200)),
        ) == waterui_testing::WaitResult::Completed,
        "an invalid write must show the validator's error text"
    );
}
