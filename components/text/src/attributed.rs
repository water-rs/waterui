use alloc::vec::Vec;
use waterui_core::{Color, Str};

use crate::font::Font;

#[derive(Debug)]
pub enum Attribute {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Color(Color),
    BackgroundColor(Color),
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
