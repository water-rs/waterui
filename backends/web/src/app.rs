//! Web application wrapper for WaterUI.

use crate::{element::WebElement, renderer};
use wasm_bindgen::prelude::*;
use waterui::{Environment, View};

/// A Web application wrapper for WaterUI.
pub struct WebApp {
    container_id: String,
    env: Environment,
}

impl WebApp {
    /// Create a new web application that will render into the given container element.
    pub fn new(container_id: &str) -> Self {
        Self {
            container_id: container_id.to_string(),
            env: Environment::new(),
        }
    }

    /// Set the environment for the application.
    pub fn environment(mut self, env: Environment) -> Self {
        self.env = env;
        self
    }

    /// Get the container element from the DOM.
    fn get_container(&self) -> Result<WebElement, JsValue> {
        let window = web_sys::window().ok_or("No global `window` exists")?;
        let document = window
            .document()
            .ok_or("Should have a document on window")?;
        let container = document
            .get_element_by_id(&self.container_id)
            .ok_or_else(|| format!("Container element '{}' not found", self.container_id))?;
        Ok(WebElement::new(container))
    }

    /// Render the given content view into the container.
    pub fn render<V: View>(&self, content: V) -> Result<(), JsValue> {
        let container = self.get_container()?;

        // Clear existing content
        container.clear_children();

        // Render the new content
        let content_element = renderer::render(content, &self.env);
        container.append_child(&content_element)?;

        Ok(())
    }

    /// Render the given content function into the container.
    pub fn render_fn<F, V>(&self, content_fn: F) -> Result<(), JsValue>
    where
        F: Fn() -> V,
        V: View,
    {
        self.render(content_fn())
    }
}
