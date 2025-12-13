//! A `WaterUI` application representation.

use nami::signal::IntoComputed;
use waterui_core::{AnyView, Environment, View};
use waterui_layout::stack::zstack;
use waterui_str::Str;

use crate::overlay::FullScreenOverlayManager;
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
    ///
    /// This injects a `FullScreenOverlayManager` into the environment and wraps
    /// the content with a ZStack overlay layer.
    pub fn new(content: impl View, env: Environment) -> Self {
        // Create overlay manager and view
        let (manager, overlay_view) = FullScreenOverlayManager::new();

        // Install the manager into the environment
        let mut env = env;
        env.install(manager);

        // Wrap content with overlay using ZStack
        let wrapped = zstack((content, overlay_view));

        Self {
            windows: vec![Window::new("WaterUI App", AnyView::new(wrapped))],
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
