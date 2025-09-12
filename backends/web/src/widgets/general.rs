//! General widget implementations for the web backend.

use crate::element::WebElement;
use std::collections::HashMap;

/// Render a divider element.
pub fn render_divider() -> WebElement {
    let element = WebElement::create("hr").expect("Failed to create divider element");

    let styles: HashMap<&str, &str> = [
        ("border", "none"),
        ("border-top", "1px solid #ccc"),
        ("margin", "8px 0"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = element.set_styles(&styles);
    element
}

/// Render a horizontal divider.
pub fn render_horizontal_divider() -> WebElement {
    render_divider()
}

/// Render a vertical divider.
pub fn render_vertical_divider() -> WebElement {
    let element = WebElement::create("div").expect("Failed to create vertical divider");

    let styles: HashMap<&str, &str> = [
        ("width", "1px"),
        ("height", "100%"),
        ("background-color", "#ccc"),
        ("margin", "0 8px"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = element.set_styles(&styles);
    element
}

/// Render a progress bar.
pub fn render_progress(progress: f64) -> WebElement {
    let element = WebElement::create("progress").expect("Failed to create progress element");

    let _ = element.set_attribute("value", &progress.to_string());
    let _ = element.set_attribute("max", "1.0");

    element
}

/// Render a loading/indeterminate progress indicator.
pub fn render_loading_progress() -> WebElement {
    let element = WebElement::create("div").expect("Failed to create loading progress");

    let _ = element.add_class("loading-spinner");

    // Basic CSS for a spinning loader
    let styles: HashMap<&str, &str> = [
        ("width", "20px"),
        ("height", "20px"),
        ("border", "2px solid #f3f3f3"),
        ("border-top", "2px solid #3498db"),
        ("border-radius", "50%"),
        ("animation", "spin 1s linear infinite"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = element.set_styles(&styles);
    element
}

/// Render an element with padding applied.
pub fn render_with_padding<F>(padding: f64, render_fn: F) -> WebElement
where
    F: FnOnce() -> WebElement,
{
    let container = WebElement::create("div").expect("Failed to create padding container");

    let _ = container.set_style("padding", &format!("{}px", padding));

    let child = render_fn();
    let _ = container.append_child(&child);

    container
}
