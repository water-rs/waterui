//! Markdown example for WaterUI.
use waterui::{prelude::*, widget::RichText};

pub fn init() -> Environment {
    Environment::new()
}

const MARKDOWN: &str = include_str!("example.md");

pub fn main() -> impl View {
    scroll(RichText::from_markdown(MARKDOWN).padding())
}

waterui_ffi::export!();
