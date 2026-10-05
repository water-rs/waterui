//! A `WaterUI` application representation.

use core::future::Future;
use core::pin::Pin;

use nami::Computed;
use suiteki::Str;
use waterui_core::{
    Environment, State,
    handler::{Handler, HandlerOnce, ViewBuilder, boxed_action, boxed_action_once},
};

use crate::{
    component::menu::{Menu, MenuBarView},
    window::Window,
};

pub use crate::runtime::termination::{
    Quit, Termination, TerminationHandle, TerminationHost, TerminationKind,
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
    /// The termination hooks, carried to the runner inside [`AppParts`].
    termination: Termination,
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
    /// The application's termination hooks. The runner starts the machine
    /// with [`Termination::start`] once its local executor exists, then
    /// reports every quit path through the returned handle.
    pub termination: Termination,
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
            termination: Termination::default(),
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

    /// Injects cloneable state into the application's environment.
    ///
    /// The application-level counterpart of `ViewExt::state`: handlers that
    /// run under the application's environment rather than a view's —
    /// `App::menu_bar` commands, [`App::on_quit_request`] and
    /// [`App::on_terminate`] — extract a `#[state]`-marked type or `State<T>`
    /// the same way view handlers do. Repeated calls on the same type install
    /// positional `State<T>` slots in call order.
    ///
    /// ```
    /// # use waterui::prelude::*;
    /// # use waterui::app::App;
    /// #[waterui::state]
    /// #[derive(Clone)]
    /// struct Store;
    ///
    /// fn app(env: Environment) -> App {
    ///     App::new(|| text!("Counter"), env)
    ///         .state(&Store)
    ///         .menu_bar(Menu::new("App", "Quit".action(|_store: Store| {})))
    /// }
    /// ```
    #[must_use]
    pub fn state<T: Clone + 'static>(mut self, state: &T) -> Self {
        self.env = self.env.extending(State(state.clone()));
        self
    }

    /// Ask the application before it quits.
    ///
    /// `handler` runs when a *cancellable* termination request arrives: the
    /// user choosing Quit, the platform's quit gesture, a declared
    /// `MenuItem::Quit`, or [`Quit::request`]. Returning
    /// [`QuitReply::Cancel`] vetoes the quit and the application keeps
    /// running; [`QuitReply::Quit`] lets termination proceed to
    /// [`App::on_terminate`]. A *required* termination — a termination
    /// signal, the last window closing under [`LastWindowPolicy::Quit`], or
    /// a Windows session that ends whatever the application answered —
    /// never asks. At most one question is open at a time; a required
    /// request arriving while the question is open supersedes it.
    ///
    /// The handler extracts from the application environment like any other
    /// [`Handler`], and its future is driven on the runner's local executor.
    /// iOS, Android and web kill the process without notice and never call
    /// this handler.
    #[must_use]
    pub fn on_quit_request<H, Args, Fut>(mut self, handler: H) -> Self
    where
        H: Handler<Args, Fut>,
        Fut: Future<Output = QuitReply> + 'static,
    {
        let mut action = boxed_action(handler);
        self.termination.on_quit_request = Some(Box::new(
            move |env| -> Pin<Box<dyn Future<Output = QuitReply>>> { Box::pin(action(env)) },
        ));
        self
    }

    /// Run shutdown work once the application is actually ending.
    ///
    /// `handler` runs exactly once, after [`App::on_quit_request`] answered
    /// [`QuitReply::Quit`] — or immediately for a *required* termination,
    /// which skips the question: a termination signal, the last window
    /// closing under [`LastWindowPolicy::Quit`], or a Windows session that
    /// ends whatever the application answered. The runner waits for the
    /// future to complete before tearing down, so this is where state is
    /// persisted and resources released.
    ///
    /// The handler extracts from the application environment like any other
    /// [`HandlerOnce`], and its future is driven on the runner's local
    /// executor. iOS, Android and web kill the process without notice and
    /// never call this handler.
    #[must_use]
    pub fn on_terminate<H, Args, Fut>(mut self, handler: H) -> Self
    where
        H: HandlerOnce<Args, Fut>,
        Fut: Future<Output = ()> + 'static,
    {
        let action = boxed_action_once(handler);
        self.termination.on_terminate =
            Some(Box::new(move |env| -> Pin<Box<dyn Future<Output = ()>>> {
                Box::pin(action(env))
            }));
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
            termination: self.termination,
        }
    }
}

/// An application's answer to "may I quit?", returned by
/// [`App::on_quit_request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitReply {
    /// Allow termination: `on_terminate` runs, then the process ends.
    Quit,
    /// Veto termination: the application keeps running.
    Cancel,
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;

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

        assert!(
            app.windows().is_empty(),
            "expected no windows, got {:?}",
            app.windows()
        );
        assert_eq!(app.last_window_policy(), LastWindowPolicy::StayResident);
        let parts = app.into_parts();
        assert!(
            parts.windows.is_empty(),
            "expected no windows, got {:?}",
            parts.windows
        );
        assert_eq!(parts.last_window, LastWindowPolicy::StayResident);
    }

    #[test]
    fn an_application_quits_after_its_last_window_by_default() {
        let app = App::new(|| (), Environment::new());

        assert_eq!(app.windows().len(), 1);
        assert_eq!(app.last_window_policy(), LastWindowPolicy::Quit);
    }

    #[test]
    fn a_menu_bar_command_extracts_state_installed_with_app_state() {
        use core::cell::Cell;

        use crate::component::menu::{CommandExt, ResolvedMenuItem, resolve_menu_bar_items};

        #[waterui_macros::state]
        #[derive(Clone)]
        struct Tally {
            hits: Rc<Cell<u32>>,
        }

        impl Tally {
            fn bump(&self) {
                self.hits.set(self.hits.get() + 1);
            }
        }

        let hits = Rc::new(Cell::new(0_u32));
        let tally = Tally {
            hits: Rc::clone(&hits),
        };
        let app = App::new(|| (), Environment::new())
            .state(&tally)
            .menu_bar(Menu::new("App", "Bump".action(|tally: Tally| tally.bump())));

        let bars = resolve_menu_bar_items(&app.menu_bar, &app.env).snapshot();
        let [ResolvedMenuItem::Menu(menu)] = bars.as_slice() else {
            panic!("expected one resolved menu, got {bars:?}");
        };
        let command = menu
            .items
            .snapshot()
            .into_iter()
            .find_map(|item| match item {
                ResolvedMenuItem::Command(command) => Some(command),
                _ => None,
            })
            .expect("the menu must contain the declared command");
        command.action.call(&app.env);

        assert_eq!(hits.get(), 1);
    }
}
