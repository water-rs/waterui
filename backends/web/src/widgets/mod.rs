//! Widget implementations for the web backend.

pub mod button;
pub mod form;
pub mod general;
pub mod layout;
pub mod media;
pub mod navigation;

use crate::element::WebElement;
use waterui::{Environment, Signal, Str};

/// Render a simple text label.
pub fn render_label(text: Str) -> WebElement {
    let element = WebElement::create("span").expect("Failed to create span for label");
    element.set_text_content(&text);
    element
}

/// Render styled text with configuration.
pub fn render_text(text: waterui::component::Text, _env: &Environment) -> WebElement {
    let element = WebElement::create("span").expect("Failed to create span for text");

    // Get text content from the text component
    let content = text.content();
    let content_str = content.get();
    element.set_text_content(&content_str.to_string());

    // TODO: Apply more styling when needed

    element
}
