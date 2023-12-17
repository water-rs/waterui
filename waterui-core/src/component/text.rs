use crate::{layout::Size, reactive::IntoReactive, Reactive};
use std::fmt::Display;

use crate::attributed_string::{AttributedString, Font};

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub(crate) text: Reactive<AttributedString>,
    pub(crate) selectable: Reactive<bool>,
}

raw_view!(Text);

impl Text {
    pub fn new(text: impl IntoReactive<AttributedString>) -> Self {
        Self {
            text: text.into_reactive(),
            selectable: Reactive::new(true),
        }
    }

    pub fn disable_select(self) -> Self {
        *self.selectable.get_mut() = false;
        self
    }

    pub fn display(value: impl Display) -> Self {
        Self::new(value.to_string())
    }

    pub fn bold(self) -> Self {
        self.text.get_mut().set_attribute(.., Font::bold());
        self
    }

    pub fn size(self, size: impl Into<Size>) -> Self {
        self.text
            .get_mut()
            .set_attribute(.., Font::new().size(size));
        self
    }
}
