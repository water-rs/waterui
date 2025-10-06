use core::{fmt::Display, mem::take, ops::Add};

use crate::{
    font::{Font, FontWeight},
    text,
};
use alloc::{string::String, vec::Vec};
use core::ops::AddAssign;
use nami::impl_constant;
use waterui_color::Color;
use waterui_core::{Str, View};

/// A set of text attributes for rich text formatting.
#[derive(Debug, Clone, Default)]
pub struct Style {
    /// The font to use.
    pub font: Font,
    /// The foreground (text) color.
    pub foreground: Option<Color>,
    /// The background color.
    pub background: Option<Color>,
    /// Whether the text is italic.
    pub italic: bool,
    /// Whether the text has an underline.
    pub underline: bool,
    /// Whether the text has a strikethrough.
    pub strikethrough: bool,
}

impl Style {
    /// Creates a new default `Style`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the font.
    #[must_use]
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// Sets the text color.
    #[must_use]
    pub fn foreground(mut self, color: impl Into<Color>) -> Self {
        self.foreground = Some(color.into());
        self
    }

    /// Sets the background color.
    #[must_use]
    pub fn background(mut self, color: impl Into<Color>) -> Self {
        self.background = Some(color.into());
        self
    }

    /// Sets the font weight.
    #[must_use]
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.font = self.font.weight(weight);
        self
    }

    /// Sets the bold style.
    /// Equal to calling `self.weight(FontWeight::Bold)`.
    #[must_use]
    pub fn bold(mut self) -> Self {
        self.font = self.font.bold();
        self
    }

    /// Sets the font size in points.
    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.font = self.font.size(size);
        self
    }

    /// Sets the italic style.
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Disables the italic style.
    #[must_use]
    pub const fn not_italic(mut self) -> Self {
        self.italic = false;
        self
    }

    /// Sets the underline style.
    #[must_use]
    pub const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// Disables the underline style.
    #[must_use]
    pub const fn not_underline(mut self) -> Self {
        self.underline = false;
        self
    }

    /// Sets the strikethrough style.
    #[must_use]
    pub const fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    /// Disables the strikethrough style.
    #[must_use]
    pub const fn not_strikethrough(mut self) -> Self {
        self.strikethrough = false;
        self
    }
}

/// A string with associated text attributes for rich text formatting.
#[derive(Debug, Clone, Default)]
pub struct StyledStr {
    chunks: Vec<(Str, Style)>,
}

impl StyledStr {
    /// Creates a new empty `StyledStr`.
    #[must_use]
    pub const fn empty() -> Self {
        Self { chunks: Vec::new() }
    }

    /// Creates a styled string from markdown text. Not yet implemented.
    #[must_use]
    pub fn from_markdown(_markdown: &str) -> Self {
        todo!("Markdown parsing is not implemented yet");
    }

    /// Creates a plain attributed string with a single unstyled chunk.
    #[must_use]
    pub fn plain(text: impl Into<Str>) -> Self {
        let mut s = Self::empty();
        s.push(text.into(), Style::default());
        s
    }

    /// Adds a new text chunk with the specified style.
    pub fn push(&mut self, text: impl Into<Str>, style: Style) {
        let text = text.into();
        self.chunks.push((text, style));
    }

    /// Appends text to the last chunk, or creates a new chunk if empty.
    pub fn push_str(&mut self, text: impl Into<Str>) {
        let text = text.into();
        if let Some(last) = self.chunks.last_mut() {
            let (last_text, _) = last;
            last_text.add_assign(text);
        } else {
            self.chunks.push((text, Style::default()));
        }
    }

    /// Returns the total length of the attributed string.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.iter().map(|(text, _)| text.len()).sum()
    }

    /// Checks if the attributed string is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Converts the attributed string into its plain representation.
    #[must_use]
    pub fn to_plain(&self) -> Str {
        if self.chunks.len() == 1 {
            return self.chunks[0].0.clone();
        }

        let mut result = String::new();
        for (text, _) in &self.chunks {
            result.push_str(text);
        }
        result.into()
    }

    /// Consumes the attributed string and returns its constituent chunks.
    #[must_use]
    pub fn into_chunks(self) -> Vec<(Str, Style)> {
        self.chunks
    }

    fn apply_style(mut self, f: impl Fn(&mut Style)) -> Self {
        if self.chunks.is_empty() {
            return self;
        }
        let old_chunks = core::mem::take(&mut self.chunks);
        for (text, mut style) in old_chunks {
            f(&mut style);
            self.push(text, style);
        }
        self
    }

    /// Sets the font size for all chunks.
    #[must_use]
    pub fn size(self, size: f32) -> Self {
        self.apply_style(|s| *s = take(s).size(size))
    }

    /// Sets the font for all chunks.
    #[must_use]
    pub fn font(self, font: &Font) -> Self {
        self.apply_style(|s| s.font = font.clone())
    }

    /// Sets the foreground color for all chunks.
    #[must_use]
    pub fn foreground(self, color: &Color) -> Self {
        self.apply_style(|s| s.foreground = Some(color.clone()))
    }

    /// Sets the background color for all chunks.
    #[must_use]
    pub fn background_color(self, color: &Color) -> Self {
        self.apply_style(|s| s.background = Some(color.clone()))
    }

    /// Sets the font weight for all chunks.
    #[must_use]
    pub fn weight(self, weight: FontWeight) -> Self {
        self.apply_style(|s| {
            *s = take(s).weight(weight);
        })
    }

    /// Sets the font to bold for all chunks.
    #[must_use]
    pub fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    /// Sets the italic style for all chunks.
    #[must_use]
    pub fn italic(self, italic: bool) -> Self {
        self.apply_style(|s| s.italic = italic)
    }

    /// Sets the underline style for all chunks.
    #[must_use]
    pub fn underline(self, underline: bool) -> Self {
        self.apply_style(|s| s.underline = underline)
    }

    /// Sets the strikethrough style for all chunks.
    #[must_use]
    pub fn strikethrough(self, strikethrough: bool) -> Self {
        self.apply_style(|s| s.strikethrough = strikethrough)
    }
}

impl View for StyledStr {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        text(self)
    }
}

impl Add for StyledStr {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        for (text, style) in rhs.chunks {
            self.push(text, style);
        }
        self
    }
}

impl Add<&'static str> for StyledStr {
    type Output = Self;

    fn add(mut self, rhs: &'static str) -> Self::Output {
        self.push(rhs, Style::default());
        self
    }
}

impl Extend<(Str, Style)> for StyledStr {
    fn extend<T: IntoIterator<Item = (Str, Style)>>(&mut self, iter: T) {
        for (text, style) in iter {
            self.push(text, style);
        }
    }
}

impl From<Str> for StyledStr {
    fn from(value: Str) -> Self {
        Self::plain(value)
    }
}

impl From<&'static str> for StyledStr {
    fn from(value: &'static str) -> Self {
        Self::plain(value)
    }
}

impl From<String> for StyledStr {
    fn from(value: String) -> Self {
        Self::plain(value)
    }
}

impl Display for StyledStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_plain())
    }
}

/// An extension trait for creating `StyledStr` from strings.
pub trait ToStyledStr: Sized {
    /// Converts the value into an `StyledStr` with the given style.
    fn styled(self, style: Style) -> StyledStr;

    /// Converts the value into an `StyledStr` with a bold style.
    fn bold(self) -> StyledStr {
        self.styled(Style::new().bold())
    }

    /// Converts the value into an `StyledStr` with an italic style.
    fn italic(self) -> StyledStr {
        self.styled(Style::new().italic())
    }

    /// Converts the value into an `StyledStr` with an underline style.
    fn underline(self) -> StyledStr {
        self.styled(Style::new().underline())
    }

    /// Converts the value into an `StyledStr` with a strikethrough style.
    fn strikethrough(self) -> StyledStr {
        self.styled(Style::new().strikethrough())
    }

    /// Converts the value into an `StyledStr` with a specific text color.
    fn foreground(self, color: impl Into<Color>) -> StyledStr {
        self.styled(Style::new().foreground(color))
    }

    /// Converts the value into an `StyledStr` with a specific background color.
    fn background(self, color: Color) -> StyledStr {
        self.styled(Style::new().background(color))
    }

    /// Converts the value into an `StyledStr` with a specific font.
    fn font(self, font: Font) -> StyledStr {
        self.styled(Style::new().font(font))
    }
}

impl<T: Into<Str>> ToStyledStr for T {
    fn styled(self, style: Style) -> StyledStr {
        let mut s = StyledStr::empty();
        s.push(self.into(), style);
        s
    }
}

impl_constant!(Style, StyledStr);
