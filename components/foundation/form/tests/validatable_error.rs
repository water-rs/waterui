//! An invalid write through a `ValidatableView` must surface its error text.

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
    app.query()
        .role(Role::TEXT_INPUT)
        .label("Email")
        .set_text("");
    app.query()
        .role(Role::LABEL)
        .label("Value is required.")
        .assert_exists_within(Duration::from_millis(200));
}
