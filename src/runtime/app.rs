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

/// What this application is called, or empty when nothing said.
///
/// An application is named by whoever launches it: the `water` CLI passes the
/// name the project gave itself, which is the same name its bundle carries.
/// Nothing inside `WaterUI` can know it, so a build started by other means —
/// a bundle opened directly, a test harness — reports nothing and leaves the
/// question to the platform, which for a bundle can answer it.
#[must_use]
pub fn application_name() -> Str {
    std::env::var("WATERUI_APP_NAME").map_or_else(|_| Str::default(), Str::from)
}

impl App {
    /// Create a new application with the given main content view and environment.
    ///
    /// The application's main window opens immediately (state is initialized
    /// to [`WindowState::Normal`](crate::window::WindowState::Normal) rather than the type's `default()`, which is
    /// `Closed`).
    ///
    /// The window is given no title of its own, which is what an empty title
    /// means: it is shown under the application's own name, which the platform
    /// knows and this does not. Use [`Window::title`] to say something else.
    pub fn new(content: impl ViewBuilder, env: Environment) -> Self {
        let state = nami::binding(crate::window::WindowState::Normal);
        Self::new_with_windows([Window::new("", state, content)], env)
    }

    /// Create a new application with the given windows and environment.
    ///
    /// # Panics
    ///
    /// Panics if no windows are provided.
    pub fn new_with_windows(windows: impl Into<Vec<Window>>, mut env: Environment) -> Self {
        let mut iter = windows.into().into_iter();
        let main_window = iter
            .next()
            .expect("App::new_with_windows requires at least one window");
        if env
            .get::<Computed<waterui_core::layout::LayoutDirection>>()
            .is_none()
            && env
                .get::<nami::Binding<waterui_core::layout::LayoutDirection>>()
                .is_none()
            && env.get::<waterui_core::layout::LayoutDirection>().is_none()
        {
            env.insert(waterui_core::layout::AutomaticLayoutDirection(
                waterui_locale::layout_direction_computed(&env),
            ));
        }
        // Realizations have to be in place before any view resolves, and this
        // is the last moment the environment is still the composition root's.
        crate::realization::install(&mut env);
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
    #[must_use = "iterators are lazy; dropping this one visits no windows"]
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

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal};
    use waterui_core::layout::{LayoutDirection, layout_direction};
    use waterui_locale::locales;

    use super::*;

    #[test]
    fn application_direction_tracks_locale_binding() {
        let locale = Binding::container(locales::AR);
        let mut env = Environment::new();
        env.insert(locale.clone());
        let app = App::new(|| (), env);
        let direction = layout_direction(&app.env);

        assert_eq!(direction.get(), LayoutDirection::RightToLeft);
        locale.set(locales::EN);
        assert_eq!(direction.get(), LayoutDirection::LeftToRight);
    }

    #[test]
    fn explicit_application_direction_overrides_locale() {
        let mut env = Environment::new();
        env.insert(locales::AR);
        env.insert(LayoutDirection::LeftToRight);
        let app = App::new(|| (), env);

        assert_eq!(
            layout_direction(&app.env).get(),
            LayoutDirection::LeftToRight
        );
    }
}
