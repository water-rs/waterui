//! Markdown example for WaterUI.
use waterui::{prelude::*, widget::RichText};

pub fn init() -> Environment {
    Environment::new()
}

const MARKDOWN: &str = r#"# WaterUI Markdown Example    
This is an example of using **WaterUI** to render Markdown content in a cross-platform application.

Supports **bold**, *italic*, and `code` text styles. blocks

"#;

pub fn main() -> impl View {
    vstack((
        text("Hello, WaterUI Markdown!").size(24).bold(),
        RichText::from_markdown(MARKDOWN),
    ))
    .padding()
}

waterui_ffi::export!();
