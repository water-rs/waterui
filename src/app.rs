//! A `WaterUI` application representation.

use nami::{Computed, signal::IntoComputed};
use waterui_core::{Environment, handler::ViewBuilder};
use waterui_str::Str;

use crate::{
    component::menu::{Menu, MenuBarView},
    window::Window,
};

/// Represents a `WaterUI` application.
#[derive(Debug)]
pub struct App {
    /// Main application window.
    main_window: Window,
    /// Additional application windows.
    windows: Vec<Window>,
    /// Optional system menu bar menus.
    pub menu_bar: Computed<Vec<Menu>>,
    /// The application environment containing injected services.
    pub env: Environment,
}

impl App {
    /// Create a new application with the given main content view and environment.
    pub fn new(content: impl ViewBuilder, env: Environment) -> Self {
        Self::new_with_windows([Window::new("WaterUI App", content)], env)
    }

    /// Create a new application with the given windows and environment.
    ///
    /// # Panics
    ///
    /// Panics if no windows are provided.
    pub fn new_with_windows(windows: impl Into<Vec<Window>>, env: Environment) -> Self {
        let mut iter = windows.into().into_iter();
        let main_window = iter
            .next()
            .expect("App::new_with_windows requires at least one window");
        Self {
            main_window,
            windows: iter.collect(),
            menu_bar: Computed::constant(Vec::new()),
            env,
        }
    }

    /// Get a reference to the main (first) window.
    #[must_use]
    pub const fn main_window(&self) -> &Window {
        &self.main_window
    }

    /// Get a mutable reference to the main (first) window.
    #[must_use]
    pub const fn main_window_mut(&mut self) -> &mut Window {
        &mut self.main_window
    }

    /// Get an iterator over all windows (main window first).
    pub fn windows(&self) -> impl DoubleEndedIterator<Item = &Window> {
        std::iter::once(&self.main_window).chain(self.windows.iter())
    }

    /// Get a mutable iterator over all windows (main window first).
    pub fn windows_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Window> {
        std::iter::once(&mut self.main_window).chain(self.windows.iter_mut())
    }

    /// Add an additional window to the application.
    ///
    /// Use this for multi-window applications on platforms that support it.
    #[must_use]
    pub fn window(mut self, window: Window) -> Self {
        self.windows.push(window);
        self
    }

    /// Sets the application system menu bar.
    #[must_use]
    pub fn menu_bar(mut self, menus: impl MenuBarView) -> Self {
        self.menu_bar = menus.into_menus();
        self
    }

    /// Consume the app and return all windows with the main window first.
    #[must_use]
    pub fn into_windows(self) -> Vec<Window> {
        self.into_parts().0
    }

    /// Consume the app and return `(windows, menu_bar, env)`.
    #[must_use]
    pub fn into_parts(self) -> (Vec<Window>, Computed<Vec<Menu>>, Environment) {
        let mut windows = Vec::with_capacity(1 + self.windows.len());
        windows.push(self.main_window);
        windows.extend(self.windows);
        (windows, self.menu_bar, self.env)
    }

    /// Set the title of the main application window.
    #[must_use]
    pub fn title(mut self, title: impl IntoComputed<Str>) -> Self {
        self.main_window.title = title.into_computed();
        self
    }
}
