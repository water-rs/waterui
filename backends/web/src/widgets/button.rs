//! Button widget implementations for the web backend.

use crate::{element::WebElement, events::EventTypes};
use std::collections::HashMap;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Configuration for button styling and behavior.
#[derive(Debug, Clone)]
pub struct ButtonConfig {
    pub text: String,
    pub disabled: bool,
    pub variant: ButtonVariant,
}

/// Button style variants.
#[derive(Debug, Clone)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Outline,
    Ghost,
    Destructive,
}

/// Render a clickable button.
pub fn render_button(config: ButtonConfig, on_click: Option<Box<dyn Fn()>>) -> WebElement {
    let element = WebElement::create("button").expect("Failed to create button element");

    // Set button text
    element.set_text_content(&config.text);

    // Set disabled state
    if config.disabled {
        let _ = element.set_attribute("disabled", "true");
    }

    // Apply button styling based on variant
    let (base_styles, variant_styles) = get_button_styles(&config.variant);
    let mut styles = base_styles;
    styles.extend(variant_styles);

    let _ = element.set_styles(&styles);

    // Attach click event listener if callback provided
    if let Some(callback) = on_click
        && let Some(html_element) = element.as_html_element()
    {
        let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            callback();
        }) as Box<dyn Fn(web_sys::Event)>);

        let _ = html_element
            .add_event_listener_with_callback(EventTypes::CLICK, closure.as_ref().unchecked_ref());

        // Keep closure alive (memory leak for now - need proper lifecycle management)
        closure.forget();
    }

    element
}

/// Get base and variant-specific styles for buttons.
fn get_button_styles(
    variant: &ButtonVariant,
) -> (
    HashMap<&'static str, &'static str>,
    HashMap<&'static str, &'static str>,
) {
    let base_styles: HashMap<&'static str, &'static str> = [
        ("padding", "8px 16px"),
        ("border-radius", "4px"),
        ("border", "none"),
        ("cursor", "pointer"),
        ("font-size", "14px"),
        ("font-weight", "500"),
        ("transition", "all 0.2s ease"),
        ("outline", "none"),
    ]
    .iter()
    .cloned()
    .collect();

    let variant_styles: HashMap<&'static str, &'static str> = match variant {
        ButtonVariant::Primary => [("background-color", "#007bff"), ("color", "white")]
            .iter()
            .cloned()
            .collect(),

        ButtonVariant::Secondary => [("background-color", "#6c757d"), ("color", "white")]
            .iter()
            .cloned()
            .collect(),

        ButtonVariant::Outline => [
            ("background-color", "transparent"),
            ("color", "#007bff"),
            ("border", "1px solid #007bff"),
        ]
        .iter()
        .cloned()
        .collect(),

        ButtonVariant::Ghost => [
            ("background-color", "transparent"),
            ("color", "#007bff"),
            ("border", "none"),
        ]
        .iter()
        .cloned()
        .collect(),

        ButtonVariant::Destructive => [("background-color", "#dc3545"), ("color", "white")]
            .iter()
            .cloned()
            .collect(),
    };

    (base_styles, variant_styles)
}

/// Render a simple text button with minimal styling.
pub fn render_text_button(text: &str, on_click: Option<Box<dyn Fn()>>) -> WebElement {
    render_button(
        ButtonConfig {
            text: text.to_string(),
            disabled: false,
            variant: ButtonVariant::Ghost,
        },
        on_click,
    )
}

/// Render a primary action button.
pub fn render_primary_button(text: &str, on_click: Option<Box<dyn Fn()>>) -> WebElement {
    render_button(
        ButtonConfig {
            text: text.to_string(),
            disabled: false,
            variant: ButtonVariant::Primary,
        },
        on_click,
    )
}
