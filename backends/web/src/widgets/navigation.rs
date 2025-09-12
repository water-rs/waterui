//! Navigation widget implementations for the web backend.

use crate::element::WebElement;
use std::collections::HashMap;

/// Render a navigation container.
pub fn render_navigation_view() -> WebElement {
    let nav = WebElement::create("nav").expect("Failed to create nav element");

    let styles: HashMap<&str, &str> = [
        ("display", "flex"),
        ("flex-direction", "column"),
        ("height", "100vh"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = nav.set_styles(&styles);
    nav
}

/// Render a navigation link.
pub fn render_navigation_link(text: &str, href: Option<&str>) -> WebElement {
    let link = WebElement::create("a").expect("Failed to create link element");

    link.set_text_content(text);

    if let Some(url) = href {
        let _ = link.set_attribute("href", url);
    }

    let styles: HashMap<&str, &str> = [
        ("color", "#007bff"),
        ("text-decoration", "none"),
        ("padding", "8px 12px"),
        ("border-radius", "4px"),
        ("transition", "background-color 0.2s ease"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = link.set_styles(&styles);
    link
}

/// Render a tab view container.
pub fn render_tab_view() -> WebElement {
    let container = WebElement::create("div").expect("Failed to create tab container");

    let styles: HashMap<&str, &str> = [("display", "flex"), ("flex-direction", "column")]
        .iter()
        .cloned()
        .collect();

    let _ = container.set_styles(&styles);

    // Create tab header
    let tab_header = WebElement::create("div").expect("Failed to create tab header");
    let header_styles: HashMap<&str, &str> = [
        ("display", "flex"),
        ("border-bottom", "1px solid #dee2e6"),
        ("background-color", "#f8f9fa"),
    ]
    .iter()
    .cloned()
    .collect();
    let _ = tab_header.set_styles(&header_styles);

    // Create tab content
    let tab_content = WebElement::create("div").expect("Failed to create tab content");
    let content_styles: HashMap<&str, &str> = [("flex", "1"), ("padding", "16px")]
        .iter()
        .cloned()
        .collect();
    let _ = tab_content.set_styles(&content_styles);

    let _ = container.append_child(&tab_header);
    let _ = container.append_child(&tab_content);

    container
}

/// Render a tab item.
pub fn render_tab_item(text: &str, active: bool) -> WebElement {
    let tab = WebElement::create("button").expect("Failed to create tab button");

    tab.set_text_content(text);

    let mut styles: HashMap<&str, &str> = [
        ("padding", "12px 16px"),
        ("border", "none"),
        ("background", "transparent"),
        ("cursor", "pointer"),
        ("border-bottom", "2px solid transparent"),
        ("transition", "all 0.2s ease"),
    ]
    .iter()
    .cloned()
    .collect();

    if active {
        styles.insert("border-bottom-color", "#007bff");
        styles.insert("color", "#007bff");
        styles.insert("font-weight", "600");
    }

    let _ = tab.set_styles(&styles);
    tab
}

/// Render a modal sheet overlay.
pub fn render_sheet() -> WebElement {
    let overlay = WebElement::create("div").expect("Failed to create sheet overlay");

    let overlay_styles: HashMap<&str, &str> = [
        ("position", "fixed"),
        ("top", "0"),
        ("left", "0"),
        ("right", "0"),
        ("bottom", "0"),
        ("background-color", "rgba(0, 0, 0, 0.5)"),
        ("display", "flex"),
        ("align-items", "center"),
        ("justify-content", "center"),
        ("z-index", "1000"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = overlay.set_styles(&overlay_styles);

    // Create the modal content
    let modal = WebElement::create("div").expect("Failed to create modal");
    let modal_styles: HashMap<&str, &str> = [
        ("background-color", "white"),
        ("border-radius", "8px"),
        ("padding", "24px"),
        ("max-width", "400px"),
        ("width", "90%"),
        ("max-height", "80vh"),
        ("overflow", "auto"),
        ("box-shadow", "0 4px 20px rgba(0, 0, 0, 0.15)"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = modal.set_styles(&modal_styles);
    let _ = overlay.append_child(&modal);

    overlay
}

/// Render an alert dialog.
pub fn render_alert(_title: &str, _message: &str, _button_text: &str) -> WebElement {
    // For a complete implementation, we'd need to access the modal child
    // and add proper HTML content. This is a simplified placeholder.

    render_sheet()
}
