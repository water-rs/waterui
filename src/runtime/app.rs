//! A `WaterUI` application representation.

use core::future::Future;
use core::pin::Pin;

use nami::Computed;
use suiteki::Str;
use waterui_core::{
    Environment, State,
    handler::{Handler, HandlerOnce, ViewBuilder, boxed_action, boxed_action_once},
};

#[cfg(feature = "menu")]
use crate::component::{
    label::SemanticLabel,
    menu::{CloseWindowChord, Menu, MenuBarView},
};
use crate::window::{AnyWindow, Window, WindowChannel, WindowContent, WindowToolbar};

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
pub struct App<W> {
    /// The windows opened at startup, in declaration order.
    windows: W,
    /// What the application does once it has no window left.
    last_window: LastWindowPolicy,
    /// Optional system menu bar menus.
    #[cfg(feature = "menu")]
    pub menu_bar: Option<Computed<Vec<Menu<SemanticLabel>>>>,
    /// The application environment containing injected services. Values
    /// installed with [`App::state`] are layered over it when the runner
    /// takes the application apart with [`App::into_parts`].
    pub env: Environment,
    /// The values installed with [`App::state`], the first call's nearest.
    states: Environment,
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each last-window policy variant by variant; a new policy must fail their build instead of falling into a wildcard arm"
)]
pub enum LastWindowPolicy {
    /// End the application once its last window closes, and at startup when
    /// it declares no window at all.
    #[default]
    Quit,
    /// Keep the application running with no window, until it quits
    /// explicitly or the system asks it to.
    StayResident,
}

/// The windows and runtime window channels an application declares, in
/// declaration order, each keeping its content's type up to the runner that
/// opens it.
///
/// A single [`Window`], a [`WindowChannel`] paired with a
/// [`WindowManager`](crate::window::WindowManager), a tuple of window sets
/// (nesting included, which is what [`App::window`] builds), an array or a
/// `Vec` of windows of one type, and `()` for none are window sets.
pub trait Windows: 'static {
    /// How many startup windows the set currently holds.
    fn len(&self) -> usize;

    /// Whether the set holds no startup window.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Hands each window and runtime window channel to `sink`, in declaration
    /// order.
    fn for_each<S: WindowSink>(self, sink: &mut S);

    /// The startup windows with their content types erased, in declaration
    /// order.
    fn erase(self) -> Vec<AnyWindow>
    where
        Self: Sized,
    {
        struct Erase(Vec<AnyWindow>);
        impl WindowSink for Erase {
            fn window<C: ViewBuilder, T: WindowToolbar>(&mut self, window: Window<C, T>) {
                self.0.push(window.erase());
            }

            fn channel<C: ViewBuilder, T: WindowToolbar>(&mut self, channel: WindowChannel<C, T>) {
                self.0
                    .extend(channel.drain().into_iter().map(Window::erase));
            }
        }
        let mut erase = Erase(Vec::with_capacity(self.len()));
        self.for_each(&mut erase);
        erase.0
    }
}

/// Receives an application's startup windows and runtime window channels, each
/// with its own content and toolbar types.
///
/// A runner implements it to turn each window and channel into the form it
/// holds, so the content and toolbar dispatch statically below the window.
pub trait WindowSink {
    /// Receives the next startup window.
    fn window<C: ViewBuilder, T: WindowToolbar>(&mut self, window: Window<C, T>);

    /// Connects a channel of windows opened while the application runs.
    fn channel<C: ViewBuilder, T: WindowToolbar>(&mut self, channel: WindowChannel<C, T>);
}

impl Windows for () {
    fn len(&self) -> usize {
        0
    }

    fn for_each<S: WindowSink>(self, _sink: &mut S) {}
}

impl<C: ViewBuilder, T: WindowToolbar> Windows for Window<C, T> {
    fn len(&self) -> usize {
        1
    }

    fn for_each<S: WindowSink>(self, sink: &mut S) {
        sink.window(self);
    }
}

impl<C: ViewBuilder, T: WindowToolbar> Windows for WindowChannel<C, T> {
    fn len(&self) -> usize {
        self.queued_len()
    }

    fn for_each<S: WindowSink>(self, sink: &mut S) {
        sink.channel(self);
    }
}

impl<C: ViewBuilder, T: WindowToolbar> Windows for Vec<Window<C, T>> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn for_each<S: WindowSink>(self, sink: &mut S) {
        for window in self {
            sink.window(window);
        }
    }
}

impl<C: ViewBuilder, T: WindowToolbar, const N: usize> Windows for [Window<C, T>; N] {
    fn len(&self) -> usize {
        N
    }

    fn for_each<S: WindowSink>(self, sink: &mut S) {
        for window in self {
            sink.window(window);
        }
    }
}

macro_rules! tuple_windows {
    ($($name:ident),+) => {
        impl<$($name: Windows),+> Windows for ($($name,)+) {
            fn len(&self) -> usize {
                #[allow(non_snake_case, reason = "the bindings are named after their type parameters")]
                let ($($name,)+) = self;
                0 $(+ $name.len())+
            }

            fn for_each<S: WindowSink>(self, sink: &mut S) {
                #[allow(non_snake_case, reason = "the bindings are named after their type parameters")]
                let ($($name,)+) = self;
                $($name.for_each(sink);)+
            }
        }
    };
}

tuple_windows!(A);
tuple_windows!(A, B);
tuple_windows!(A, B, C);
tuple_windows!(A, B, C, D);
tuple_windows!(A, B, C, D, E);
tuple_windows!(A, B, C, D, E, F);
tuple_windows!(A, B, C, D, E, F, G);
tuple_windows!(A, B, C, D, E, F, G, H);

/// An [`App`] taken apart for a runner, see [`App::into_parts`].
#[derive(Debug)]
pub struct AppParts<W> {
    /// The windows opened at startup, in declaration order; possibly none.
    pub windows: W,
    /// The application menu bar.
    #[cfg(feature = "menu")]
    pub menu_bar: Option<Computed<Vec<Menu<SemanticLabel>>>>,
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

impl<W> AppParts<W> {
    /// The same parts with the windows replaced by `f(windows)`: a runner
    /// that stores the application turns its windows into the form it holds.
    #[inline]
    pub fn map_windows<V>(self, f: impl FnOnce(W) -> V) -> AppParts<V> {
        let Self {
            windows,
            #[cfg(feature = "menu")]
            menu_bar,
            env,
            last_window,
            termination,
        } = self;
        AppParts {
            windows: f(windows),
            #[cfg(feature = "menu")]
            menu_bar,
            env,
            last_window,
            termination,
        }
    }
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

impl<B: ViewBuilder> App<Window<WindowContent<B>>> {
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
    #[inline]
    pub fn new(content: B, env: Environment) -> Self {
        let state = nami::binding(crate::window::WindowState::Normal);
        Self::new_with_windows(Window::new("", state, content), env)
    }
}

impl<W: Windows> App<W> {
    /// Create an application with the given windows and environment.
    ///
    /// Any number of windows is valid, none included: an application with no
    /// window opens its windows on demand, or lives in a status item, and
    /// pairs this with [`LastWindowPolicy::StayResident`] so its runner keeps
    /// it alive (see [`App::on_last_window_closed`]).
    #[inline]
    pub fn new_with_windows(windows: W, mut env: Environment) -> Self {
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
        Self {
            windows,
            last_window: LastWindowPolicy::default(),
            #[cfg(feature = "menu")]
            menu_bar: None,
            env,
            states: Environment::new(),
            termination: Termination::default(),
        }
    }

    /// The windows opened at startup, in declaration order.
    #[must_use]
    pub const fn windows(&self) -> &W {
        &self.windows
    }

    /// Mutable access to the windows opened at startup.
    #[must_use]
    pub const fn windows_mut(&mut self) -> &mut W {
        &mut self.windows
    }

    /// Add a window or [`WindowChannel`] to the application.
    ///
    /// Use this for multi-window applications on platforms that support it.
    #[must_use]
    pub fn window<N: Windows>(self, window: N) -> App<(W, N)> {
        let Self {
            windows,
            last_window,
            #[cfg(feature = "menu")]
            menu_bar,
            env,
            states,
            termination,
        } = self;
        App {
            windows: (windows, window),
            last_window,
            #[cfg(feature = "menu")]
            menu_bar,
            env,
            states,
            termination,
        }
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
    #[inline]
    #[must_use]
    #[cfg(feature = "menu")]
    pub fn menu_bar(mut self, menus: impl MenuBarView) -> Self {
        self.menu_bar = Some(menus.into_menus());
        self
    }

    /// Injects cloneable state into the application's environment.
    ///
    /// The application-level counterpart of `ViewExt::state`: handlers that
    /// run under the application's environment rather than a view's —
    /// `App::menu_bar` commands, [`App::on_quit_request`] and
    /// [`App::on_terminate`] — extract a `#[state]`-marked type or `State<T>`
    /// the same way view handlers do. Repeated calls on the same type install
    /// positional `State<T>` slots in call order, as a `ViewExt::state`
    /// chain does: the first call's value binds the first `State<T>`
    /// parameter. The values join the environment [`App::into_parts`] hands
    /// the runner.
    ///
    /// ```
    /// # use waterui::prelude::*;
    /// # use waterui::app::{App, Windows};
    /// #[waterui::state]
    /// #[derive(Clone)]
    /// struct Store;
    ///
    /// fn app(env: Environment) -> App<impl Windows> {
    ///     App::new(|| text!("Counter"), env)
    ///         .state(&Store)
    ///         .menu_bar(Menu::new("App", "Quit".action(|_store: Store| {})))
    /// }
    /// ```
    #[must_use]
    pub fn state<T: Clone + 'static>(mut self, state: &T) -> Self {
        // A `ViewExt::state` chain nests, so its first value ends up nearest.
        // The values installed so far are layered over the new one to keep
        // that order.
        self.states = self
            .states
            .layered_on(&Environment::new().extending(State(state.clone())));
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
        // A handler whose parameters cannot be extracted does not run: the
        // failure is reported and the request is answered as if the app had
        // registered no handler, which lets the termination proceed.
        self.termination.on_quit_request = Some(Box::new(
            move |env| -> Pin<Box<dyn Future<Output = QuitReply>>> {
                match action(env) {
                    Ok(question) => Box::pin(question),
                    Err(error) => {
                        error.report();
                        Box::pin(core::future::ready(QuitReply::Quit))
                    }
                }
            },
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
                // A handler whose parameters cannot be extracted does not
                // run: the failure is reported and the shutdown work is
                // empty, so termination still completes.
                match action(env) {
                    Ok(shutdown) => Box::pin(shutdown),
                    Err(error) => {
                        error.report();
                        Box::pin(core::future::ready(()))
                    }
                }
            }));
        self
    }

    /// Consume the app and return its windows, in declaration order.
    #[must_use]
    pub fn into_windows(self) -> W {
        self.windows
    }

    /// Consume the app and return the parts a runner needs, with the values
    /// installed by [`App::state`] layered over its environment, and the
    /// Close Window chord decided from its menu bar installed — every
    /// environment a runner hands its menus carries the one decision.
    #[inline]
    #[must_use]
    pub fn into_parts(self) -> AppParts<W> {
        let env = self.states.layered_on(&self.env);
        #[cfg(feature = "menu")]
        let env = {
            let mut env = env;
            env.insert(
                self.menu_bar
                    .as_ref()
                    .map_or_else(CloseWindowChord::standard, |menu_bar| {
                        CloseWindowChord::new(menu_bar, &env)
                    }),
            );
            env
        };
        AppParts {
            windows: self.windows,
            #[cfg(feature = "menu")]
            menu_bar: self.menu_bar,
            env,
            last_window: self.last_window,
            termination: self.termination,
        }
    }
}

/// An application's answer to "may I quit?", returned by
/// [`App::on_quit_request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed answer: the application quits or cancels the quit"
)]
pub enum QuitReply {
    /// Allow termination: `on_terminate` runs, then the process ends.
    Quit,
    /// Veto termination: the application keeps running.
    Cancel,
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::{Cell, RefCell};

    use nami::{Binding, Signal};
    use waterui_core::layout::{LayoutDirection, layout_direction};
    use waterui_locale::locales;

    use super::*;
    #[cfg(feature = "menu")]
    use crate::component::menu::{CommandExt as _, Shortcut};

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
        let app = App::new_with_windows((), Environment::new())
            .on_last_window_closed(LastWindowPolicy::StayResident);

        assert_eq!(app.last_window_policy(), LastWindowPolicy::StayResident);
        let parts = app.into_parts();
        assert_eq!(parts.last_window, LastWindowPolicy::StayResident);
    }

    #[test]
    fn an_application_quits_after_its_last_window_by_default() {
        let app = App::new(|| (), Environment::new());

        assert_eq!(app.windows().len(), 1);
        assert_eq!(app.last_window_policy(), LastWindowPolicy::Quit);
    }

    /// Resolves `app`'s menu bar under the environment its runner receives
    /// and runs the first command of its first menu.
    #[cfg(feature = "menu")]
    fn run_first_menu_bar_command<W: Windows>(app: App<W>) {
        use crate::component::menu::{ResolvedMenuItem, resolve_menu_bar_items};

        let parts = app.into_parts();
        let menu_bar = parts
            .menu_bar
            .as_ref()
            .expect("the test application must declare a menu bar");
        let bars = resolve_menu_bar_items(menu_bar, &parts.env).snapshot();
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
        command.action.run(&parts.env);
    }

    /// The environment `into_parts` hands a runner decides the Close
    /// Window chord once, from the declared menu bar: a bar that binds ⌘W
    /// on its own command leaves every Close Window item ⇧⌘W.
    #[test]
    #[cfg(feature = "menu")]
    fn the_menu_bar_decides_the_close_window_chord() {
        let parts = App::new(|| (), Environment::new())
            .menu_bar(Menu::new(
                "File",
                "Close Tab"
                    .action(|| {})
                    .shortcut(Shortcut::new('w').command()),
            ))
            .into_parts();
        assert_eq!(
            CloseWindowChord::of(&parts.env),
            Some(Shortcut::new('w').command().shift()),
            "the bar's own ⌘W command moves every Close Window item to ⇧⌘W"
        );
    }

    #[test]
    #[cfg(feature = "menu")]
    fn a_menu_bar_command_extracts_state_installed_with_app_state() {
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
        run_first_menu_bar_command(
            App::new(|| (), Environment::new())
                .state(&tally)
                .menu_bar(Menu::new("App", "Bump".action(|tally: Tally| tally.bump()))),
        );

        assert_eq!(hits.get(), 1);
    }

    #[test]
    #[cfg(feature = "menu")]
    fn same_typed_app_state_binds_in_call_order() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        run_first_menu_bar_command(
            App::new(|| (), Environment::new())
                .state(&"first")
                .state(&"second")
                .menu_bar(Menu::new(
                    "App",
                    "Record".action(
                        move |first: State<&'static str>, second: State<&'static str>| {
                            record.borrow_mut().extend([first.0, second.0]);
                        },
                    ),
                )),
        );

        // The first `State<T>` parameter binds the first call's value, as in
        // a `ViewExt::state` chain.
        assert_eq!(*seen.borrow(), ["first", "second"]);
    }
}
