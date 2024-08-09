use waterui_reactive::{compute::ToComputed, Computed};
use waterui_str::Str;

use crate::View;

configurable!(Text, TextConfig);

#[derive(Debug)]
#[non_exhaustive]
pub struct TextConfig {
    pub content: Computed<Str>,
    pub font: Font,
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

impl Default for Text {
    fn default() -> Self {
        text("")
    }
}

impl Text {
    pub fn new(content: impl ToComputed<Str>) -> Self {
        Self(TextConfig {
            content: content.to_computed(),
            font: Font::default(),
        })
    }

    pub fn font(mut self, font: Font) -> Self {
        self.0.font = font;
        self
    }

    pub fn size(mut self, size: f64) -> Self {
        self.0.font.size = size;
        self
    }
}

pub fn text(text: impl ToComputed<Str>) -> Text {
    Text::new(text)
}

macro_rules! impl_text {
    ($($ty:ty),*) => {
        $(
            impl View for $ty {
                fn body(self, _env: crate::Environment) -> impl View {
                    text(self)
                }
            }
        )*
    };
}

impl_text!(&'static str);
