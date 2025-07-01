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

#[derive(Debug)]
pub struct AttributedStr {
    string: Vec<AttributedStrChunk>,
}

#[derive(Debug)]
struct AttributedStrChunk {
    text: Str,
    attributes: Vec<Attribute>,
}
