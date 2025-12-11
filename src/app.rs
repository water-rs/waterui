//! A WaterUI application representation.

use nami::{Computed, signal::IntoComputed};
use waterui_core::{AnyView, Environment, View};
use waterui_str::Str;

/// Represents a WaterUI application.
#[derive(Debug)]
pub struct App {
    /// The title of the application window.
    pub title: Computed<Str>,
    /// The main content view of the application.
    pub main: AnyView,
    /// The environment configuration for the application.
    pub env: Environment,
}

impl App {
    /// Create a new application with the given main content view.
    pub fn new(content: impl View, env: Environment) -> Self {
        Self {
            title: Computed::constant(Str::from_static("WaterUI App")),
            main: AnyView::new(content),
            env,
        }
    }

    /// Set the title of the application window.
    pub fn title(self, title: impl IntoComputed<Str>) -> Self {
        Self {
            title: title.into_computed(),
            ..self
        }
    }
}
