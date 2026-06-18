use waterui::{Environment, Signal as _, Str};
use waterui_controls::label::Label;

pub fn label_plain_text(label: &Label) -> Str {
    label
        .semantic_text()
        .clone()
        .resolve(&Environment::new())
        .content
        .get()
        .to_plain()
}
