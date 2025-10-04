use waterui_color::{Color, Srgb};

use crate::styled::{Style, StyledStr, ToStyledStr};

pub trait Highlighter: Send + Sync {
    fn highlight<'a>(&self, text: &'a str) -> Vec<HighlightChunk<'a>>;
}

pub async fn highlight_text(text: &str, highlighter: impl Highlighter) -> StyledStr {
    // TODO: use async thread pool
    highlighter.highlight(text).into_iter().fold(
        StyledStr::empty(),
        |mut s, chunk| { s.push(chunk.text.to_string(), Style::default().foreground(chunk.color)); s },
    )
}

pub struct HighlightChunk<'a> {
    pub text: &'a str,
    pub color: Srgb,
}

impl HighlightChunk<'_> {
    #[must_use]
    pub fn attributed(self) -> StyledStr {
        self.text.to_string().foreground(self.color)
    }
}
