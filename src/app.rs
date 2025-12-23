//! A `WaterUI` application representation.

use nami::signal::IntoComputed;
use waterui_core::{Environment, View};
use waterui_str::Str;

use crate::window::Window;

/// Represents a `WaterUI` application.
#[derive(Debug)]
pub struct App {
    /// Application windows. The first window is the main window.
    pub windows: Vec<Window>,
    /// The application environment containing injected services.
    pub env: Environment,
}

impl App {
    /// Create a new application with the given main content view and environment.
    pub fn new(content: impl View, env: Environment) -> Self {
        Self::new_with_windows([Window::new("WaterUI App", content)], env)
    }

    /// Create a new application with the given windows and environment.
    pub fn new_with_windows(windows: impl Into<Vec<Window>>, env: Environment) -> Self {
        Self {
            windows: windows.into(),
            env,
        }
    }

    /// Get a reference to the main (first) window.
    #[must_use]
    pub fn main_window(&self) -> &Window {
        &self.windows[0]
    }

    /// Get a mutable reference to the main (first) window.
    #[must_use]
    pub fn main_window_mut(&mut self) -> &mut Window {
        &mut self.windows[0]
    }

    /// Add an additional window to the application.
    ///
    /// Use this for multi-window applications on platforms that support it.
    #[must_use]
    pub fn window(mut self, window: Window) -> Self {
        self.windows.push(window);
        self
    }

    /// Set the title of the main application window.
    #[must_use]
    pub fn title(mut self, title: impl IntoComputed<Str>) -> Self {
        self.windows[0].title = title.into_computed();
        self
    }
}
