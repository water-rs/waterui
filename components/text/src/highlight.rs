use waterui_color::{Color, Srgb};
use waterui_core::Str;

use crate::attributed::AttributedChunk;

pub trait Highlighter{
    fn highlight<'a>(&self, text: &'a str) -> Vec<HighlightChunk<'a>>;
}

pub struct HighlightChunk<'a>{
    pub text: &'a str,
    pub color: Srgb,
}

impl HighlightChunk<'_>{
    pub fn attributed(self) -> AttributedChunk{
        AttributedChunk::new(self.text.to_string()).color(Color::new(self.color))
    }
}
