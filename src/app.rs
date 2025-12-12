//! A WaterUI application representation.

use nami::signal::IntoComputed;
use waterui_core::{AnyView, Environment, View};
use waterui_str::Str;

use crate::window::Window;

/// Represents a WaterUI application.
#[derive(Debug)]
pub struct App {
    /// The main application window.
    pub main: Window,
    /// The environment configuration for the application.
    pub env: Environment,
}

impl App {
    /// Create a new application with the given main content view.
    pub fn new(content: impl View, env: Environment) -> Self {
        Self {
            main: Window::new("WaterUI App", AnyView::new(content)),
            env,
        }
    }

    /// Set the title of the application window.
    pub fn title(mut self, title: impl IntoComputed<Str>) -> Self {
        self.main.title = title.into_computed();
        self
    }
}
