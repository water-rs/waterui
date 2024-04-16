use alloc::string::String;

use crate::{Compute, Computed};

#[derive(Debug)]
#[non_exhaustive]
pub struct Text {
    pub _content: Computed<String>,
    pub _selectable: Computed<bool>,
    pub _font: Font,
}

#[derive(Debug)]
#[repr(C)]
pub struct Font {
    size: f64,
}

impl Default for Font {
    fn default() -> Self {
        Self { size: f64::NAN }
    }
}

raw_view!(Text);

impl Text {
    pub fn new(text: impl Compute<Output = String>) -> Self {
        Self {
            _content: text.computed(),
            _selectable: Computed::constant(true),
            _font: Font::default(),
        }
    }

    pub fn font(mut self, font: Font) -> Self {
        self._font = font;
        self
    }

    pub fn size(mut self, size: f64) -> Self {
        self._font.size = size;
        self
    }

    pub fn selectable(mut self, selectable: impl Compute<Output = bool>) -> Self {
        self._selectable = selectable.computed();
        self
    }
}

pub fn text(text: impl Compute<Output = String>) -> Text {
    Text::new(text)
}
