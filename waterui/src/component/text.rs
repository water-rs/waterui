use crate::CowStr;
use waterui_reactive::{compute::IntoComputed, Computed};
#[derive(Debug)]
#[non_exhaustive]
pub struct Text {
    pub _content: Computed<CowStr>,
    pub _selection: Computed<bool>,
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
    pub fn new(text: impl IntoComputed<CowStr>) -> Self {
        Self {
            _content: text.into_computed(),
            _selection: true.into_computed(),
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

    pub fn selection(mut self, selection: impl IntoComputed<bool>) -> Self {
        self._selection = selection.into_computed();
        self
    }
}

pub fn text(text: impl IntoComputed<CowStr>) -> Text {
    Text::new(text)
}
