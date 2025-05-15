use alloc::vec::Vec;
use waterui_core::{Color, Str};

use crate::font::Font;

#[derive(Debug, uniffi::Enum)]
pub enum Attribute {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Color(Color),
    BackgroundColor(Color),
    Font(Font),
}

#[derive(Debug, uniffi::Record)]
pub struct AttributedStr {
    string: Vec<AttributedStrChunk>,
}

#[derive(Debug, uniffi::Record)]
struct AttributedStrChunk {
    text: Str,
    attributes: Vec<Attribute>,
}
