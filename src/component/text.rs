use std::fmt::Display;

use crate::{
    attributed_string::{AttributedString, Font},
    view::Frame,
};

pub struct Text {
    frame: Frame,
    pub text: AttributedString,
}

impl Text {
    pub fn new(text: impl Into<AttributedString>) -> Self {
        Self {
            text: text.into(),
            frame: Default::default(),
        }
    }

    pub fn display(value: impl Display) -> Self {
        Self::new(value.to_string())
    }

    pub fn bold(mut self) -> Self {
        self.text.set_attribute(.., Font::bold());
        self
    }
}

native_implement_with_frame!(Text);

pub fn text(text: impl Into<AttributedString>) -> Text {
    Text::new(text)
}
