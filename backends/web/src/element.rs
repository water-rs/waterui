//! DOM element wrapper for the web backend.

use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, Text};
use std::collections::HashMap;

/// A wrapper around DOM elements for the web backend.
#[derive(Debug, Clone)]
pub struct WebElement {
    element: Element,
}

impl WebElement {
    /// Create a new WebElement from a DOM element.
    pub fn new(element: Element) -> Self {
        Self { element }
    }

    /// Create a new element with the given tag name.
    pub fn create(tag: &str) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or("No global `window` exists")?;
        let document = window.document().ok_or("Should have a document on window")?;
        let element = document.create_element(tag)?;
        Ok(Self::new(element))
    }

    /// Create a text node.
    pub fn create_text(content: &str) -> Result<Text, JsValue> {
        let window = web_sys::window().ok_or("No global `window` exists")?;
        let document = window.document().ok_or("Should have a document on window")?;
        Ok(document.create_text_node(content))
    }

    /// Get the inner DOM element.
    pub fn element(&self) -> &Element {
        &self.element
    }

    /// Convert to HtmlElement if possible.
    pub fn as_html_element(&self) -> Option<HtmlElement> {
        self.element.dyn_ref::<HtmlElement>().cloned()
    }

    /// Set an attribute on the element.
    pub fn set_attribute(&self, name: &str, value: &str) -> Result<(), JsValue> {
        self.element.set_attribute(name, value)
    }

    /// Set the text content of the element.
    pub fn set_text_content(&self, text: &str) {
        self.element.set_text_content(Some(text));
    }

    /// Set the inner HTML of the element.
    pub fn set_inner_html(&self, html: &str) {
        self.element.set_inner_html(html);
    }

    /// Add a CSS class to the element.
    pub fn add_class(&self, class: &str) -> Result<(), JsValue> {
        // Simple approach: just set the class attribute
        self.element.set_attribute("class", class)
    }

    /// Remove a CSS class from the element.
    pub fn remove_class(&self, _class: &str) -> Result<(), JsValue> {
        // Simplified: just remove the class attribute entirely
        let _ = self.element.remove_attribute("class");
        Ok(())
    }

    /// Set multiple CSS styles at once.
    pub fn set_styles(&self, styles: &HashMap<&str, &str>) -> Result<(), JsValue> {
        if let Some(html_element) = self.as_html_element() {
            let style = html_element.style();
            for (property, value) in styles {
                style.set_property(property, value)?;
            }
        }
        Ok(())
    }

    /// Set a single CSS style.
    pub fn set_style(&self, property: &str, value: &str) -> Result<(), JsValue> {
        if let Some(html_element) = self.as_html_element() {
            html_element.style().set_property(property, value)?;
        }
        Ok(())
    }

    /// Append a child element.
    pub fn append_child(&self, child: &WebElement) -> Result<(), JsValue> {
        self.element.append_child(child.element())?;
        Ok(())
    }

    /// Append a text node.
    pub fn append_text(&self, text: &Text) -> Result<(), JsValue> {
        self.element.append_child(text)?;
        Ok(())
    }

    /// Remove all children.
    pub fn clear_children(&self) {
        self.element.set_text_content(Some(""));
    }

    /// Get the element's ID.
    pub fn id(&self) -> String {
        self.element.id()
    }

    /// Set the element's ID.
    pub fn set_id(&self, id: &str) {
        self.element.set_id(id);
    }
}