use waterui_core::{AnyView, Environment};

use crate::{dom::DomRoot, error::WebError, renderer::WebRenderer};

/// Builder for [`WebApp`].
#[derive(Debug, Default, Clone)]
pub struct WebAppBuilder {
    root_id: Option<String>,
    inject_default_styles: bool,
}

impl WebAppBuilder {
    /// Creates a new builder with default configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            root_id: None,
            inject_default_styles: true,
        }
    }

    /// Sets the DOM element identifier that should host the application.
    #[must_use]
    pub fn with_root_id(mut self, id: impl Into<String>) -> Self {
        self.root_id = Some(id.into());
        self
    }

    /// Controls whether the backend injects the default `WaterUI` stylesheet.
    #[must_use]
    pub const fn inject_default_styles(mut self, inject: bool) -> Self {
        self.inject_default_styles = inject;
        self
    }

    /// Finalises the builder and creates a [`WebApp`].
    ///
    /// # Errors
    ///
    /// Returns an error if the DOM root element cannot be found or initialized.
    pub fn build(self) -> Result<WebApp, WebError> {
        WebApp::new_with_options(self)
    }
}

impl From<WebAppBuilder> for WebApp {
    fn from(value: WebAppBuilder) -> Self {
        value
            .build()
            .expect("WebAppBuilder::build should succeed when used in wasm32 targets")
    }
}

/// Entry point for running `WaterUI` inside the browser.
#[wasm_bindgen]
#[derive(Debug)]
pub struct WebApp {
    environment: Environment,
    renderer: WebRenderer,
}

impl WebApp {
    #[allow(clippy::needless_pass_by_value)]
    fn new_with_options(builder: WebAppBuilder) -> Result<Self, WebError> {
        let dom_root = DomRoot::new(builder.root_id.as_deref(), builder.inject_default_styles)?;
        let renderer = WebRenderer::new(dom_root);
        Ok(Self {
            environment: Environment::new(),
            renderer,
        })
    }

    /// Provides mutable access to the renderer state for advanced integrations.
    #[must_use]
    pub const fn renderer_mut(&mut self) -> &mut WebRenderer {
        &mut self.renderer
    }

    /// Provides access to the renderer state.
    #[must_use]
    pub const fn renderer(&self) -> &WebRenderer {
        &self.renderer
    }

    /// Returns a mutable reference to the underlying [`Environment`].
    #[must_use]
    pub const fn environment_mut(&mut self) -> &mut Environment {
        &mut self.environment
    }

    /// Returns an immutable reference to the environment.
    #[must_use]
    pub const fn environment(&self) -> &Environment {
        &self.environment
    }

    /// Renders an [`AnyView`] tree. The current implementation shows a placeholder UI
    /// until the dispatcher wiring for core components is completed.
    ///
    /// # Errors
    ///
    /// Returns an error if the renderer fails to render the view into the DOM.
    pub fn render(&mut self, view: AnyView) -> Result<(), WebError> {
        self.renderer.render(&self.environment, view)
    }
}

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl WebApp {
    /// Convenience constructor exposed to JavaScript callers.
    #[wasm_bindgen(constructor)]
    /// Creates a new [`WebApp`] using the default configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the DOM root element cannot be found or initialized.
    pub fn new() -> Result<Self, WebError> {
        Self::new_with_options(WebAppBuilder::new())
    }

    /// Mounts the application into the DOM, rendering a placeholder interface.
    ///
    /// # Errors
    ///
    /// Returns an error if the renderer fails to mount the application into the DOM.
    #[wasm_bindgen]
    pub fn mount(&mut self) -> Result<(), WebError> {
        self.renderer.mount()
    }
}
