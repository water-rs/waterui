//! Layout widget implementations for the web backend.

use crate::element::WebElement;
use std::collections::HashMap;
use waterui::Environment;
use waterui::component::{
    Metadata,
    layout::{Edge, scroll::ScrollView},
};

/// Render a scrollable view container.
pub fn render_scroll_view(_scroll: ScrollView, _env: &Environment) -> WebElement {
    let container = WebElement::create("div").expect("Failed to create scroll container");

    // Apply scroll-specific styling
    let styles: HashMap<&str, &str> = [
        ("overflow", "auto"),
        ("max-height", "100%"),
        ("max-width", "100%"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = container.set_styles(&styles);

    // Render a placeholder for now
    let content_element = WebElement::create("div").expect("Failed to create content element");
    content_element.set_text_content("Scrollable content");
    let _ = container.append_child(&content_element);

    container
}

/// Render a view with padding applied through Metadata<Edge>.
pub fn render_with_padding(_padding_metadata: Metadata<Edge>, _env: &Environment) -> WebElement {
    let container = WebElement::create("div").expect("Failed to create padding container");

    // Apply default padding for now
    let _ = container.set_style("padding", "16px");

    // Create a placeholder since we need proper view rendering
    let content_element = WebElement::create("div").expect("Failed to create content element");
    content_element.set_text_content("Padded content");
    let _ = container.append_child(&content_element);

    container
}

/// Render horizontal stack (HStack).
pub fn render_hstack() -> WebElement {
    let container = WebElement::create("div").expect("Failed to create hstack container");

    let styles: HashMap<&str, &str> = [
        ("display", "flex"),
        ("flex-direction", "row"),
        ("align-items", "center"),
        ("gap", "8px"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = container.set_styles(&styles);
    container
}

/// Render vertical stack (VStack).
pub fn render_vstack() -> WebElement {
    let container = WebElement::create("div").expect("Failed to create vstack container");

    let styles: HashMap<&str, &str> = [
        ("display", "flex"),
        ("flex-direction", "column"),
        ("align-items", "stretch"),
        ("gap", "8px"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = container.set_styles(&styles);
    container
}

/// Render a spacer element.
pub fn render_spacer() -> WebElement {
    let element = WebElement::create("div").expect("Failed to create spacer");

    let styles: HashMap<&str, &str> = [
        ("flex-grow", "1"),
        ("min-height", "1px"),
        ("min-width", "1px"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = element.set_styles(&styles);
    element
}
