use waterui_core::raw_view;
use waterui_reactive::{compute::ToComputed, Computed};
use waterui_str::Str;

#[derive(Debug)]
#[non_exhaustive]
pub struct Text {
    pub _content: Computed<Str>,
    pub _selectable: Computed<bool>,
    pub _font: Font,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct Font {
    pub size: f64,
    pub bold: bool,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            size: f64::NAN,
            bold: false,
        }
    }
}

raw_view!(Text);

impl Text {
    pub fn new(text: impl ToComputed<Str>) -> Self {
        Self {
            _content: text.to_computed(),
            _selectable: true.to_computed(),
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

    pub fn selectable(mut self, selectable: impl ToComputed<bool>) -> Self {
        self._selectable = selectable.to_computed();
        self
    }
}

pub fn text(text: impl ToComputed<Str>) -> Text {
    Text::new(text)
}
