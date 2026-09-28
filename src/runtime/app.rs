//! A `WaterUI` application representation.

use nami::Computed;
use suiteki::Str;
use waterui_core::{Environment, handler::ViewBuilder};

use crate::{
    component::menu::{Menu, MenuBarView},
    window::Window,
};

/// Represents a `WaterUI` application.
///
/// An application declares zero or more windows. None of them is structurally
/// special: a runner opens every declared window at startup, and what happens
/// once the last one closes — or when there is none to begin with — is the
/// application's [`LastWindowPolicy`].
#[derive(Debug)]
pub struct App {
    /// The windows opened at startup, in declaration order.
    windows: Vec<Window>,
    /// What the application does once it has no window left.
    last_window: LastWindowPolicy,
    /// Optional system menu bar menus.
    pub menu_bar: Computed<Vec<Menu>>,
    /// The application environment containing injected services.
    pub env: Environment,
}

/// What an application does once it has no open window.
///
/// Every runner consults this at startup and whenever a window closes. The
/// platforms disagree on the convention, so the application states it rather
/// than inheriting whichever one its runner happens to follow:
///
/// - On Linux and Windows an application conventionally ends with its last
///   window, which is the default here on every platform.
/// - On macOS an application conventionally stays running after its last
///   window closes — in the Dock, with its menu bar — and opens a new window
///   when asked. An application that follows that convention says
///   [`StayResident`](Self::StayResident).
/// - A status-item or tray application, or one that opens its windows on
///   demand (a terminal started without an initial window), has no window as
///   its primary surface and stays resident on every platform.
/// - iOS and Android have no windowless foreground state: their runners
///   reject an application that declares no window, whatever its policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LastWindowPolicy {
    /// End the application once its last window closes, and at startup when
    /// it declares no window at all.
    #[default]
    Quit,
    /// Keep the application running with no window, until it quits
    /// explicitly or the system asks it to.
    StayResident,
}

/// An [`App`] taken apart for a runner, see [`App::into_parts`].
#[derive(Debug)]
pub struct AppParts {
    /// The windows opened at startup, in declaration order; possibly none.
    pub windows: Vec<Window>,
    /// The application menu bar.
    pub menu_bar: Computed<Vec<Menu>>,
    /// The application environment, the composition root every window renders
    /// under.
    pub env: Environment,
    /// What the runner does once the application has no open window.
    pub last_window: LastWindowPolicy,
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

/// This application's identifier, or empty when the build was not told it.
///
/// The identifier is the reverse-DNS name the project gave itself
/// (`bundle_identifier` in `Water.toml`), the same one its bundle, package or
/// desktop entry carries. It is a property of the build rather than of the
/// launch, so the `water` CLI compiles it in through `WATERUI_APP_ID`: a
/// packaged application started from its desktop entry knows it as well as one
/// started by `water run`. A build made by other means reports nothing and
/// leaves the question to the platform.
#[must_use]
pub fn application_identifier() -> Str {
    Str::from(option_env!("WATERUI_APP_ID").unwrap_or_default())
}

impl App {
    /// Create an application with a single window showing `content`.
    ///
    /// The window opens immediately (state is initialized to
    /// [`WindowState::Normal`](crate::window::WindowState::Normal) rather than
    /// the type's `default()`, which is `Closed`).
    ///
    /// The window is given no title of its own, which is what an empty title
    /// means: it is shown under the application's own name, which the platform
    /// knows and this does not. To title it, declare the window yourself with
    /// [`Window::title`] and pass it to [`App::new_with_windows`].
    pub fn new(content: impl ViewBuilder, env: Environment) -> Self {
        let state = nami::binding(crate::window::WindowState::Normal);
        Self::new_with_windows([Window::new("", state, content)], env)
    }

    /// Create an application with the given windows and environment.
    ///
    /// Any number of windows is valid, none included: an application with no
    /// window opens its windows on demand, or lives in a status item, and
    /// pairs this with [`LastWindowPolicy::StayResident`] so its runner keeps
    /// it alive (see [`App::on_last_window_closed`]).
    pub fn new_with_windows(windows: impl Into<Vec<Window>>, mut env: Environment) -> Self {
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
            windows: windows.into(),
            last_window: LastWindowPolicy::default(),
            menu_bar: Computed::constant(Vec::new()),
            env,
        }
    }

    /// The windows opened at startup, in declaration order.
    #[must_use]
    pub const fn windows(&self) -> &[Window] {
        self.windows.as_slice()
    }

    /// Mutable access to the windows opened at startup.
    #[must_use]
    pub const fn windows_mut(&mut self) -> &mut [Window] {
        self.windows.as_mut_slice()
    }

    /// Add a window to the application.
    ///
    /// Use this for multi-window applications on platforms that support it.
    #[must_use]
    pub fn window(mut self, window: Window) -> Self {
        self.windows.push(window);
        self
    }

    /// Sets what the application does once it has no open window.
    ///
    /// The default, [`LastWindowPolicy::Quit`], ends the application with its
    /// last window — and immediately, when it declares none. An application
    /// that follows the macOS convention of staying in the Dock, a tray
    /// application, or one that opens its windows on demand passes
    /// [`LastWindowPolicy::StayResident`]. See [`LastWindowPolicy`] for the
    /// conventions of each platform.
    #[must_use]
    pub const fn on_last_window_closed(mut self, policy: LastWindowPolicy) -> Self {
        self.last_window = policy;
        self
    }

    /// What the application does once it has no open window.
    #[must_use]
    pub const fn last_window_policy(&self) -> LastWindowPolicy {
        self.last_window
    }

    /// Sets the application system menu bar.
    #[must_use]
    pub fn menu_bar(mut self, menus: impl MenuBarView) -> Self {
        self.menu_bar = menus.into_menus();
        self
    }

    /// Consume the app and return its windows, in declaration order.
    #[must_use]
    pub fn into_windows(self) -> Vec<Window> {
        self.windows
    }

    /// Consume the app and return the parts a runner needs.
    #[must_use]
    pub fn into_parts(self) -> AppParts {
        AppParts {
            windows: self.windows,
            menu_bar: self.menu_bar,
            env: self.env,
            last_window: self.last_window,
        }
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

        assert_eq!(direction.snapshot(), LayoutDirection::RightToLeft);
        locale.set(locales::EN);
        assert_eq!(direction.snapshot(), LayoutDirection::LeftToRight);
    }

    #[test]
    fn explicit_application_direction_overrides_locale() {
        let mut env = Environment::new();
        env.insert(locales::AR);
        env.insert(LayoutDirection::LeftToRight);
        let app = App::new(|| (), env);

        assert_eq!(
            layout_direction(&app.env).snapshot(),
            LayoutDirection::LeftToRight
        );
    }

    #[test]
    fn an_application_may_declare_no_window() {
        let app = App::new_with_windows(Vec::new(), Environment::new())
            .on_last_window_closed(LastWindowPolicy::StayResident);

        assert!(app.windows().is_empty());
        assert_eq!(app.last_window_policy(), LastWindowPolicy::StayResident);
        let parts = app.into_parts();
        assert!(parts.windows.is_empty());
        assert_eq!(parts.last_window, LastWindowPolicy::StayResident);
    }

    #[test]
    fn an_application_quits_after_its_last_window_by_default() {
        let app = App::new(|| (), Environment::new());

        assert_eq!(app.windows().len(), 1);
        assert_eq!(app.last_window_policy(), LastWindowPolicy::Quit);
    }
}
