use core::any::Any;

use alloc::{boxed::Box, vec::Vec};
use waterui_core::{Color, Str};

use crate::font::Font;

pub enum Attribute {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Color(Color),
    BackgroundColor(Color),
    Font(Font),
    Other(Box<dyn Any>),
}

pub struct AttributedStr(Vec<(Str, Attribute)>);
