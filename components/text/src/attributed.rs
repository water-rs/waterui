use alloc::vec::Vec;
use waterui_core::{Color, Str};

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
///
/// This is currently under development and not yet fully implemented.
#[derive(Debug)]
#[allow(dead_code)]
pub struct AttributedStr {
    string: Vec<AttributedStrChunk>,
}

/// A chunk of attributed text with specific formatting.
#[derive(Debug)]
#[allow(dead_code)]
struct AttributedStrChunk {
    text: Str,
    attributes: Vec<Attribute>,
}
