use alloc::vec::Vec;
use waterui_color::Color;
use waterui_core::{ Str};

use crate::font::Font;

/// Text attributes for rich text formatting.
///
/// This enum defines the various styling attributes that can be applied
/// to text, including weight, style, decorations, and colors.
#[derive(Debug)]
pub enum Attribute {
    /// Bold text weight.
    Bold,
    /// Italic text style.
    Italic,
    /// Underlined text decoration.
    Underline,
    /// Strikethrough text decoration.
    Strikethrough,
    /// Text foreground color.
    Color(Color),
    /// Text background color.
    BackgroundColor(Color),
    /// Font styling including size and other properties.
    Font(Font),
}

/// A string with associated text attributes for rich text formatting.
#[derive(Debug)]
pub struct AttributedStr {
    string: Vec<AttributedChunk>,
}

impl AttributedStr {
    /// Creates a new empty `AttributedStr`.
    pub fn new() -> Self {
        Self { string: Vec::new() }
    }

    /// Adds a chunk of text with specific attributes to the `AttributedStr`.
    pub fn push_chunk(&mut self, text: Str, attributes: Vec<Attribute>) {
        self.string.push(AttributedChunk { text, attributes });
    }

    /// Returns the total length of the attributed string.
    pub fn len(&self) -> usize {
        self.string.iter().map(|chunk| chunk.text.len()).sum()
    }

    /// Checks if the attributed string is empty.
    pub fn is_empty(&self) -> bool {
        self.string.is_empty()
    }


}


/// A chunk of attributed text with specific formatting.
#[derive(Debug)]
pub struct AttributedChunk {
    text: Str,
    attributes: Vec<Attribute>,
}


impl AttributedChunk{
    pub fn new(text: impl Into<Str>) -> Self {
        Self { text: text.into(), attributes: Vec::new() }
    }

    pub fn into_inner(self) -> (Str, Vec<Attribute>) {
        (self.text, self.attributes)
    }

    pub fn with_attributes(mut self,attributes: Vec<Attribute>) -> Self{
        // merge attributes
        self.attributes.extend(attributes);
        self
    }

    pub fn with_attribute(mut self,attribute: Attribute) -> Self{
        self.attributes.push(attribute);
        self
    }

    pub fn color(mut self,color: Color) -> Self{
        self.with_attribute(Attribute::Color(color))
    }

    pub fn background_color(mut self,color: Color) -> Self{
        self.with_attribute(Attribute::BackgroundColor(color))
    }

    pub fn bold(mut self) -> Self{
        self.with_attribute(Attribute::Bold)
    }

    pub fn italic(mut self) -> Self{
        self.with_attribute(Attribute::Italic)
    }

    pub fn underline(mut self) -> Self{
        self.with_attribute(Attribute::Underline)
    }

    pub fn strikethrough(mut self) -> Self{
        self.with_attribute(Attribute::Strikethrough)
    }

    pub fn font(mut self,font: Font) -> Self{
        self.with_attribute(Attribute::Font(font))
    }

    pub fn attributed(self) -> AttributedStr{
        let mut attributed_str = AttributedStr::new();
        attributed_str.push_chunk(self.text, self.attributes);
        attributed_str
    }
}