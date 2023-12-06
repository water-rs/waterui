use std::fmt::Display;

use serde::Deserialize;
use serde::Serialize;

use crate::layout::Size;

use crate::attributed_string::{AttributedString, Font};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Text {
    pub(crate) text: AttributedString,
    pub(crate) selectable: bool,
}

impl Text {
    pub fn new(text: impl Into<AttributedString>) -> Self {
        Self {
            text: text.into(),
            selectable: true,
        }
    }

    pub fn disable_select(mut self) -> Self {
        self.selectable = false;
        self
    }

    pub fn display(value: impl Display) -> Self {
        Self::new(value.to_string())
    }

    pub fn bold(mut self) -> Self {
        self.text.set_attribute(.., Font::bold());
        self
    }

    pub fn size(mut self, size: impl Into<Size>) -> Self {
        self.text.set_attribute(.., Font::new().size(size));
        self
    }
}

native_implement!(Text);

pub fn text(text: impl Into<AttributedString>) -> Text {
    Text::new(text)
}
