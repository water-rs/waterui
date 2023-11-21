use crate::{
    attributed_string::AttributedString,
    reactive::{IntoRef, Ref},
};

pub struct Text {
    pub text: Ref<AttributedString>,
}

impl Text {
    pub fn new(text: impl IntoRef<AttributedString>) -> Self {
        Self {
            text: text.into_ref(),
        }
    }
}

native_implement!(Text);

pub fn text(text: impl IntoRef<AttributedString>) -> Text {
    Text::new(text)
}
