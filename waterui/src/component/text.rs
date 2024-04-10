use crate::{Compute, Computed};

#[derive(Debug, Clone)]
pub struct Text {
    pub _content: Computed<String>,
    pub _selectable: Computed<bool>,
}

raw_view!(Text);

impl Text {
    pub fn new(text: impl Compute<String>) -> Self {
        Self {
            _content: text.computed(),
            _selectable: Computed::constant(true),
        }
    }

    pub fn selectable(mut self, selectable: impl Compute<bool>) -> Self {
        self._selectable = selectable.computed();
        self
    }
}

pub fn text(text: impl Compute<String>) -> Text {
    Text::new(text)
}
