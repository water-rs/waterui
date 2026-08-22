//! `WebView` component for `WaterUI` framework.
//!
//! This module provides a web view component for embedding web content in `WaterUI` applications.
//!
//! # Architecture
//!
//! - [`WebViewHandle`] - Imperative trait that native backends implement
//! - [`AnyWebViewHandle`] - Type-erased wrapper with downcast support
//! - [`WebViewController`] - Factory injected into Environment by native backends
//! - [`WebView`] - Reactive wrapper with `Binding<T>` state
//!
//! # Example
//!
//! ```ignore
//! use waterui_webview::{WebViewController, WebView};
//!
//! // Get controller from environment and create a web view
//! let webview = controller.open();
//! webview.go_to("https://waterui.dev");
//!
//! // Use reactive state for UI
//! let can_go_back = webview.can_go_back();  // Computed<bool>
//! ```

mod controller;

pub use controller::*;
pub use cookie::Cookie;
use std::{cell::Cell, fmt, rc::Rc};
#[macro_use]
mod handle_layers;
mod handler;
pub use handler::*;
mod no_engine;
mod proxy;
pub use proxy::WebViewProxy;

// Re-export waterui-internal types for FFI layer
pub use waterui_url::{IntoUrl, Url};
mod url_signal;
pub use url_signal::IntoUrlSignal;
mod watcher;
pub use watcher::{WatcherGuard, WatcherSet};
pub mod bridge;
mod script;
pub use script::{DOCUMENT_START_SCRIPT, JsError, JsExpr, JsOutcome, JsProgram};
mod origins;
pub use origins::{BridgeOrigins, IntoBridgeOrigins, OriginPolicy, OriginRule};

/// An object whose methods and state are exposed to the page.
///
/// Implemented by `#[js_api]`; there is no reason to write one by hand, and
/// nothing stops you calling [`WebViewOpen::handler`] and
/// [`WebViewOpen::expose`] directly instead — the macro emits exactly those
/// calls, so the two paths stay interchangeable.
///
/// How a method is exposed follows from its shape:
///
/// | Shape | Exposed as | Page writes it back? |
/// | --- | --- | --- |
/// | `async fn(&self, ..) -> R` | a handler the page awaits | — |
/// | `fn(&self) -> Binding<T>` | mirrored state | yes |
/// | `fn(&self) -> Computed<T>` | mirrored state | no |
/// | `#[js(skip)]` anything | not exposed | — |
///
/// ```
/// use waterui::webview::WebView;
/// use waterui::{Binding, js_api};
///
/// struct Counter {
///     count: Binding<u32>,
/// }
///
/// #[js_api(namespace = "counter")]
/// impl Counter {
///     /// The page reads and assigns `counter.count`.
///     fn count(&self) -> Binding<u32> {
///         self.count.clone()
///     }
///
///     /// The page calls `await counter.add({ by: 2 })`.
///     async fn add(&self, by: u32) {
///         self.count.set(self.count.get() + by);
///     }
/// }
///
/// let view = WebView::open("https://app.waterui.dev").serve(Counter {
///     count: Binding::container(0),
/// });
/// ```
///
/// A synchronous method that takes arguments is neither of those things — it
/// cannot be state, and a handler must be able to await a reply — so it is
/// rejected rather than silently ignored:
///
/// ```compile_fail
/// use waterui::js_api;
///
/// struct Api;
///
/// #[js_api]
/// impl Api {
///     fn add(&self, by: u32) -> u32 {
///         by + 1
///     }
/// }
/// ```
pub trait JsApi: Sized + 'static {
    /// Registers everything this object exposes.
    fn register(api: std::rc::Rc<Self>, builder: WebViewOpen) -> WebViewOpen;

    /// The members of this surface, as TypeScript.
    ///
    /// One line per exposed method or field, indented to sit inside an
    /// interface. [`typescript_declarations`] wraps these into a file a page's
    /// tooling can consume; this is the part `#[js_api]` knows.
    fn typescript() -> &'static str;
}

/// A `.d.ts` describing what `A` exposes, for a page's TypeScript to consume.
///
/// Write it next to the page's sources from a test, the way `ts-rs` does, so a
/// surface that changes without the declarations being regenerated fails on the
/// page rather than in production:
///
/// ```ignore
/// #[test]
/// fn declarations_are_current() {
///     let generated = waterui::webview::typescript_declarations::<PageApi>();
///     let checked_in = std::fs::read_to_string("web/waterui.d.ts").unwrap();
///     assert_eq!(generated, checked_in, "run with UPDATE=1 to refresh");
/// }
/// ```
///
/// # What the types are worth
///
/// Names, arity and the shape of each argument object are exact: the macro reads
/// them from the signature. Payload *types* are named only for the ones this
/// boundary deals in directly — strings, numbers, booleans, arrays, options,
/// maps, bytes. An application's own struct becomes `unknown`, because a
/// declaration TypeScript believes is worse than one that admits it does not
/// know. Catching `invoke("gret", ...)` and a misspelled argument is most of the
/// value, and both work without knowing what a `Greeting` is.
#[must_use]
pub fn typescript_declarations<A: JsApi>() -> String {
    format!(
        "// Generated from a #[js_api] surface. Do not edit.\n\
         \n\
         interface WaterUiApi {{\n\
         {}\n\
         \x20 watch(key: string, callback: (value: unknown) => void): () => void;\n\
         }}\n\
         \n\
         declare const waterui: WaterUiApi;\n",
        A::typescript()
    )
}

/// Re-exported so `#[js_api]` can derive `Deserialize` for its argument structs
/// without the calling crate having to depend on `serde` itself.
#[doc(hidden)]
pub use serde;
/// Re-exported for the same reason as [`serde`]: `exec!` and `eval!` serialize
/// each `@{...}` hole, and the calling crate must not have to depend on
/// `serde_json` just to write a JavaScript literal.
#[doc(hidden)]
pub use serde_json;
mod big_integers;
mod message;
pub use message::{Bytes, HandlerName, IntoJsReply, JsReply, Json};
mod state;
pub use state::{FieldEntry, JsField, StateWriteError};

use waterui_core::{
    Binding, Computed, Environment, Native, Signal, View, binding, layout::StretchAxis,
    reactive::signal::IntoComputed,
};
use waterui_layout::spacer;
use waterui_layout::stack::vstack;
use waterui_str::Str;

/// Something that happened in a `WebView`.
///
/// There is no "nothing happened" variant: the absence of an event is expressed
/// by [`WebView::events`] yielding `None`, which is the same fact stated in the
/// type rather than smuggled into the enum. Nor is there a variant for
/// navigation state — that is [`WebView::can_go_back`] and
/// [`WebView::can_go_forward`], and it used to appear here as a `#[doc(hidden)]`
/// variant every `match` still had to spell out.
#[derive(Debug, Clone, PartialEq)]
pub enum WebViewEvent {
    /// The web view is about to navigate to a new URL.
    WillNavigate {
        /// The URL being navigated to.
        url: Url,
    },
    /// The web view is loading content.
    Loading {
        /// The progress of the loading operation (0.0 to 1.0).
        progress: f32,
    },
    /// The web view has finished loading the content.
    Loaded,
    /// A redirect occurred during navigation.
    Redirect {
        /// The original URL.
        from: Url,
        /// The redirected URL.
        to: Url,
    },
    /// An error occurred during navigation or loading.
    Error(WebViewError),
}

impl From<WebViewEvent> for BackendEvent {
    fn from(event: WebViewEvent) -> Self {
        Self::Event(event)
    }
}

/// What a backend reports to its `WebView`.
///
/// Navigation state is not a user-facing event — it is what
/// [`WebView::can_go_back`] and [`WebView::can_go_forward`] read — so it travels
/// here rather than in [`WebViewEvent`].
#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    /// Something worth telling the application about.
    Event(WebViewEvent),
    /// The history changed.
    NavigationState {
        /// Whether the web view can navigate back.
        can_go_back: bool,
        /// Whether the web view can navigate forward.
        can_go_forward: bool,
    },
}

/// Errors that can occur in the `WebView` component.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum WebViewError {
    /// A network error occurred.
    #[error("Network error: {0}")]
    Network(Str),
    /// An SSL/TLS error occurred.
    #[error("SSL error at {url}: {message}")]
    Ssl {
        /// The URL that caused the error.
        url: Url,
        /// The error message.
        message: Str,
    },
    /// Failed to load the page.
    #[error("Load failed: {0}")]
    LoadFailed(Str),
}

/// A `WebView` component that displays web content and handles navigation events.
///
/// This struct wraps [`AnyWebViewHandle`] and adds reactive state via nami bindings.
/// The `can_go_back` and `can_go_forward` bindings are automatically updated when
/// the native backend emits [`WebViewEvent::StateChanged`] events.
///
/// `WebView` implements [`View`] so it can be used directly in the view hierarchy.
pub struct WebView {
    event: Binding<Option<WebViewEvent>>,
    handle: AnyWebViewHandle,
    can_go_back: Binding<bool>,
    can_go_forward: Binding<bool>,
    /// Signal subscriptions that must outlive this view: the URL and user-agent
    /// bindings. Erased because their value types differ; only their lifetime
    /// matters here.
    retained: Vec<Rc<dyn core::any::Any>>,
    /// Keeps the internal watcher that drives `can_go_back` / `can_go_forward`
    /// and the public event signal subscribed for as long as any clone lives.
    state_watcher: Rc<WatcherGuard>,
}

impl Clone for WebView {
    fn clone(&self) -> Self {
        Self {
            event: self.event.clone(),
            handle: self.handle.clone(),
            can_go_back: self.can_go_back.clone(),
            can_go_forward: self.can_go_forward.clone(),
            retained: self.retained.clone(),
            state_watcher: Rc::clone(&self.state_watcher),
        }
    }
}

impl fmt::Debug for WebView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebView")
            .field("event", &self.event)
            .field("handle", &self.handle)
            .field("can_go_back", &self.can_go_back)
            .field("can_go_forward", &self.can_go_forward)
            .field("retained_subscriptions", &self.retained.len())
            .finish_non_exhaustive()
    }
}

impl WebView {
    /// Creates a new `WebView` component with the given handle.
    #[must_use]
    pub(crate) fn from_handle(handle: AnyWebViewHandle) -> Self {
        let event: Binding<Option<WebViewEvent>> = binding(None);
        let can_go_back = binding(handle.can_go_back());
        let can_go_forward = binding(handle.can_go_forward());

        // Set up event handler to update reactive state
        let state_watcher = handle.watch({
            let event = event.clone();
            let can_go_back = can_go_back.clone();
            let can_go_forward = can_go_forward.clone();
            move |backend_event| match backend_event {
                BackendEvent::NavigationState {
                    can_go_back: back,
                    can_go_forward: forward,
                } => {
                    can_go_back.set(back);
                    can_go_forward.set(forward);
                }
                BackendEvent::Event(e) => event.set(Some(e)),
            }
        });

        Self {
            handle,
            event,
            can_go_back,
            can_go_forward,
            retained: Vec::new(),
            state_watcher: Rc::new(state_watcher),
        }
    }

    /// Sets whether redirects are allowed, using a reactive signal.
    ///
    /// The native backend will automatically sync with the signal's value.
    /// When the signal changes, the redirect setting is updated immediately.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let allow_redirects = binding(false);
    /// let webview = controller.open().redirects_enabled(allow_redirects.clone());
    /// ```
    #[must_use]
    pub fn redirects_enabled(self, enabled: impl IntoComputed<bool>) -> Self {
        self.handle.set_redirects_enabled(enabled);
        self
    }

    /// Opens a new `WebView` and navigates to the specified URL.
    ///
    /// This is the L1 fire-and-forget entry point. Native creation and the
    /// initial navigation are deferred until the view is rendered in an
    /// environment containing a [`WebViewController`]. Use
    /// [`WebViewOpen::with_proxy`] to attach an imperative
    /// [`WebViewProxy`] when you need refresh / history navigation /
    /// `run_javascript` from a child handler.
    ///
    /// `url` is reactive: writing a new [`Url`] into a bound `Binding<Url>`
    /// navigates the existing native web view without rebuilding it.
    ///
    /// ```ignore
    /// let url = binding(Url::new("https://waterui.dev"));
    /// let webview = WebView::open(url.clone());
    /// url.set(Url::new("https://waterui.dev/docs"));
    /// ```
    pub fn open(url: impl IntoUrlSignal) -> WebViewOpen {
        WebViewOpen {
            url: url.into_url_signal(),
            redirects_enabled: None,
            user_agent: None,
            scripts: Vec::new(),
            handlers: Vec::new(),
            event_watchers: Vec::new(),
            state: Vec::new(),
            bridge_origins: BridgeOrigins::default(),
        }
    }

    /// Injects this web view's [`WebViewProxy`] into `content`'s environment.
    ///
    /// Any handler inside `content` can then take a `WebViewProxy` parameter and
    /// drive the web view, through the same `Extractor` machinery that powers
    /// `Button::action`.
    ///
    /// This returns `content` with the proxy attached and nothing else: it does
    /// not place the web view, so the layout is yours. An earlier version wrapped
    /// both in a `vstack`, which silently decided that controls go above the page
    /// and made an overlay or a side panel impossible to express.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use waterui::prelude::*;
    /// use waterui_webview::{WebView, WebViewProxy};
    ///
    /// let webview = controller.open();
    /// let toolbar = webview.proxy_scope(|| {
    ///     hstack((
    ///         button("←").action(|p: WebViewProxy| p.go_back()),
    ///         button("→").action(|p: WebViewProxy| p.go_forward()),
    ///         button("⟳").action(|p: WebViewProxy| p.refresh()),
    ///     ))
    /// });
    /// vstack((toolbar, webview))   // or hstack, or zstack — your choice
    /// ```
    pub fn proxy_scope<V, F>(&self, content: F) -> impl View
    where
        V: View,
        F: FnOnce() -> V + 'static,
    {
        use waterui_core::env::with;
        with(content(), WebViewProxy::new(self.handle.clone()))
    }

    /// The most recent event, or `None` before anything has happened.
    ///
    /// Named `events` rather than `event` because `ViewExt::event` shadows an
    /// inherent method of that name, forcing callers to spell out
    /// `WebView::event(&view)`.
    ///
    /// For reacting to every event, prefer
    /// [`WebViewOpen::on_event`], which needs no guard.
    #[must_use]
    pub fn events(&self) -> impl Signal<Output = Option<WebViewEvent>> {
        self.event.clone()
    }

    /// Navigates to the specified URL.
    ///
    /// `url` is anything that already names a URL — a literal, a [`Url`], or a
    /// `const` built with [`Url::new`]. Text that only exists at runtime has to be
    /// parsed first, with [`Url::parse_user_input`] or `str::parse`, so a
    /// malformed address is handled where it originates instead of at the backend.
    pub fn go_to(&self, url: impl IntoUrl) {
        self.handle.go_to(&url.into_url());
    }

    /// Applies `user_agent` now and on every later change.
    fn bind_user_agent(mut self, user_agent: Computed<Str>) -> Self {
        let handle = self.handle.clone();
        let guard = user_agent.watch(move |context| {
            handle.set_user_agent(context.into_value().as_str());
        });
        self.handle.set_user_agent(user_agent.get().as_str());
        self.retained.push(Rc::new((user_agent, guard)));
        self
    }

    fn bind_navigation(mut self, url: Computed<Url>) -> Self {
        let handle = self.handle.clone();
        let guard = subscribe_navigation(&url, move |url| handle.go_to(&url));
        self.retained.push(Rc::new((url, guard)));
        self
    }

    /// Refreshes the current page.
    pub fn refresh(&self) {
        self.handle.refresh();
    }

    /// Stops the current loading operation.
    pub fn stop(&self) {
        self.handle.stop();
    }

    /// Navigates back in the web view's history.
    pub fn go_back(&self) {
        self.handle.go_back();
    }

    /// Navigates forward in the web view's history.
    pub fn go_forward(&self) {
        self.handle.go_forward();
    }

    /// Returns a reactive signal for whether the web view can navigate back.
    #[must_use]
    pub fn can_go_back(&self) -> Computed<bool> {
        Computed::from(self.can_go_back.clone())
    }

    /// Returns a reactive signal for whether the web view can navigate forward.
    #[must_use]
    pub fn can_go_forward(&self) -> Computed<bool> {
        Computed::from(self.can_go_forward.clone())
    }

    /// Runs the given JavaScript code in the web view and returns the result.
    ///
    /// # Errors
    ///
    /// Returns an error string from the native backend when script execution fails.
    ///
    /// The returned future is intentionally thread-local because `WebView` state is
    /// bound to the native UI thread.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and JavaScript execution are main-thread-affine"
    )]
    pub fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> impl core::future::Future<Output = Result<Str, Str>> + 'a {
        self.handle.run_javascript(script)
    }

    /// Evaluates a JavaScript expression and decodes its result as `T`.
    ///
    /// Prefer the [`eval!`](waterui_macros::eval) macro, which checks the source
    /// is a literal and passes interpolated values out of band.
    ///
    /// # Errors
    ///
    /// Returns [`JsError::Exception`] when the script throws,
    /// [`JsError::Unserializable`] when its result cannot be represented, and
    /// [`JsError::Decode`] when the result is not a `T`. Those three used to be
    /// one opaque string.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and JavaScript execution are main-thread-affine"
    )]
    pub fn eval<T: serde::de::DeserializeOwned>(
        &self,
        expr: &JsExpr,
    ) -> impl core::future::Future<Output = Result<T, JsError>> + '_ {
        let call = expr.wrapped_call();
        async move { self.run_wrapped(&call).await?.decode() }
    }

    /// Runs a JavaScript program for its effects.
    ///
    /// Prefer the [`exec!`](waterui_macros::exec) macro.
    ///
    /// # Errors
    ///
    /// As for [`eval`](Self::eval), except that the program's value is discarded.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and JavaScript execution are main-thread-affine"
    )]
    pub fn exec(
        &self,
        program: &JsProgram,
    ) -> impl core::future::Future<Output = Result<(), JsError>> + '_ {
        let call = program.wrapped_call();
        async move { self.run_wrapped(&call).await?.decode() }
    }

    /// Runs a wrapped call and parses the envelope it returns.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and JavaScript execution are main-thread-affine"
    )]
    async fn run_wrapped(&self, call: &str) -> Result<JsOutcome, JsError> {
        run_wrapped_on(&self.handle, call).await
    }

    /// Sets a cookie in this web view's native cookie store.
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        self.handle.set_cookie(cookie);
    }

    /// Retrieves the current cookies without blocking the UI thread.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and cookie stores are main-thread-affine"
    )]
    pub fn get_cookies(&self) -> impl core::future::Future<Output = Vec<Cookie<'static>>> + '_ {
        self.handle.get_cookies()
    }

    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        self.handle.set_user_agent(user_agent);
    }

    /// Injects a script that will run on every page load.
    ///
    /// `key` names the script. Injecting again under the same key replaces it,
    /// so a script whose content depends on current state can be kept current
    /// without stacking a new copy on every update.
    pub fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        self.handle.inject_script(key, script, time);
    }

    /// Enables or disables following redirects.
    ///
    pub fn set_redirects_enabled(&self, enabled: impl IntoComputed<bool>) {
        self.handle.set_redirects_enabled(enabled);
    }

    /// Returns the underlying handle.
    ///
    /// Use this to access lower-level functionality or to downcast to a native type.
    #[must_use]
    pub const fn handle(&self) -> &AnyWebViewHandle {
        &self.handle
    }
}

/// Runs a wrapped call on `handle` and parses the envelope it resolves with.
///
/// Free rather than a method because the mirrored-state bridge holds only a
/// [`WeakWebViewHandle`] — it lives inside a handler the backend owns, so
/// keeping a whole [`WebView`] there is what stopped the native web view from
/// ever being destroyed.
#[expect(
    clippy::future_not_send,
    reason = "native web views and JavaScript execution are main-thread-affine"
)]
pub(crate) async fn run_wrapped_on(
    handle: &AnyWebViewHandle,
    call: &str,
) -> Result<JsOutcome, JsError> {
    let raw = handle
        .call_async_javascript(call)
        .await
        .map_err(|message| JsError::Exception {
            message,
            stack: None,
        })?;
    serde_json::from_str(raw.as_str()).map_err(|source| JsError::Decode {
        expected: "JsOutcome",
        source,
    })
}

fn subscribe_navigation<S, F>(url: &S, navigate: F) -> S::Guard
where
    S: Signal<Output = Url>,
    F: Clone + Fn(Url) + 'static,
{
    let emitted_during_subscription = Rc::new(Cell::new(false));
    let guard = url.watch({
        let emitted_during_subscription = Rc::clone(&emitted_during_subscription);
        let navigate = navigate.clone();
        move |context| {
            emitted_during_subscription.set(true);
            navigate(context.into_value());
        }
    });
    if !emitted_during_subscription.get() {
        navigate(url.get());
    }
    guard
}

/// A handler the page can call, boxed for storage before the web view exists.
/// A handler recorded before the web view exists. Built once the rendering
/// environment is known, so its extractors resolve against the right scope.
type BoxedMessageHandler = Box<dyn FnOnce(Environment) -> Box<ScriptMessageHandler>>;

/// A deferred web view created by [`WebView::open`].
///
/// The native handle is created when this view is rendered, once its
/// [`WebViewController`] can be extracted from the live environment. Everything
/// configured here is applied to that handle the moment it exists, so a web view
/// can be fully described before a backend is anywhere in sight.
#[must_use = "a WebViewOpen must be rendered to create its native web view"]
pub struct WebViewOpen {
    url: Computed<Url>,
    redirects_enabled: Option<Computed<bool>>,
    user_agent: Option<Computed<Str>>,
    scripts: Vec<(Str, Str, ScriptInjectionTime)>,
    handlers: Vec<(Str, BoxedMessageHandler)>,
    event_watchers: Vec<Box<dyn Fn(WebViewEvent)>>,
    state: Vec<state::PendingField>,
    bridge_origins: BridgeOrigins,
}

impl fmt::Debug for WebViewOpen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebViewOpen")
            .field("url", &self.url)
            .field("redirects_enabled", &self.redirects_enabled)
            .field("user_agent", &self.user_agent)
            .field("scripts", &self.scripts.len())
            .field("handlers", &self.handlers.len())
            .field("event_watchers", &self.event_watchers.len())
            .field("state_fields", &self.state.len())
            .field("bridge_origins", &self.bridge_origins)
            .finish()
    }
}

impl WebViewOpen {
    /// Sets the reactive redirect policy applied when the web view is created.
    pub fn redirects_enabled(mut self, enabled: impl IntoComputed<bool>) -> Self {
        self.redirects_enabled = Some(enabled.into_computed());
        self
    }

    /// Sets the user agent, reactively.
    pub fn user_agent(mut self, user_agent: impl IntoComputed<Str>) -> Self {
        self.user_agent = Some(user_agent.into_computed());
        self
    }

    /// Injects a script that runs on every page load.
    ///
    /// `key` names the script; injecting again under the same key replaces it.
    pub fn inject(
        mut self,
        key: impl Into<Str>,
        script: impl Into<Str>,
        time: ScriptInjectionTime,
    ) -> Self {
        self.scripts.push((key.into(), script.into(), time));
        self
    }

    /// Registers a handler the page can call as `waterui.invoke(name, payload)`.
    ///
    /// # Panics
    ///
    /// Panics when rendered without a [`WebViewController`] in the environment.
    ///
    /// The closure is written the way `Button::action` is: take the extractors you
    /// need — [`Json<T>`], [`Bytes`], [`HandlerName`], `State<T>`,
    /// [`WebViewProxy`] — and return anything that is [`IntoJsReply`]. It may be
    /// `async`, and returning `Err` rejects the page's promise instead of
    /// resolving it with a failure encoded as success.
    ///
    /// The return type also decides what the page receives: [`Json<T>`], a
    /// string, or `()` resolve as the value itself, while [`Bytes`] and
    /// `Vec<u8>` resolve as a `Uint8Array`.
    ///
    /// ```ignore
    /// .handler("greet", |Json(req): Json<Greet>| async move {
    ///     Json(Greeting { text: format!("Hi {}", req.name) })
    /// })
    /// // await waterui.invoke("greet", { name: "Lexo" }) → { text: "Hi Lexo" }
    /// ```
    pub fn handler<F, Args, Fut, Reply>(mut self, name: impl Into<Str>, handler: F) -> Self
    where
        F: waterui_core::handler::Handler<Args, Fut> + 'static,
        Fut: core::future::Future<Output = Reply> + 'static,
        Reply: message::IntoJsReply + 'static,
    {
        let name = name.into();
        self.handlers.push((
            name.clone(),
            Box::new(move |env| message::boxed_handler(env, name, handler)),
        ));
        self
    }

    /// Mirrors a reactive value into the page.
    ///
    /// The page reads it as `waterui.state.<name>` — a local property, so reading
    /// never crosses the boundary — and subscribes with `waterui.watch(name, cb)`.
    /// A [`Binding`] is writable from JavaScript and the write flows back into it;
    /// anything else is read-only, and assigning to it throws.
    ///
    /// Granularity follows the signal: exposing a `Binding<BigStruct>` and
    /// changing one field re-sends the whole struct. Expose finer bindings when
    /// that matters.
    pub fn expose<F: state::JsField + 'static>(mut self, name: impl Into<Str>, field: F) -> Self {
        self.state.push(state::pending_field(name.into(), field));
        self
    }

    /// Exposes an object's methods and state to the page in one step.
    ///
    /// Written by hand this is a `.handler(...)` per method and an `.expose(...)`
    /// per field; `#[js_api]` generates exactly that, so the two are equivalent
    /// and the macro adds nothing the manual path cannot express.
    pub fn serve<A: JsApi>(self, api: A) -> Self {
        A::register(std::rc::Rc::new(api), self)
    }

    /// Chooses which documents may reach the bridge.
    ///
    /// Defaults to [`BridgeOrigins::Initial`] — the origin the view is opened at,
    /// main frame only. Handlers are capabilities, and a web view that can
    /// navigate can end up somewhere you did not choose, so the default is the
    /// narrowest one that still works.
    pub fn bridge_origins(mut self, origins: impl IntoBridgeOrigins) -> Self {
        self.bridge_origins = origins.into_bridge_origins();
        self
    }

    /// Observes web view events.
    ///
    /// The subscription lives as long as the web view, so there is no guard to
    /// hold. Watching the [`WebView::events`] signal by hand means retaining a
    /// guard, and forgetting to made the events silently stop arriving.
    pub fn on_event(mut self, watcher: impl Fn(WebViewEvent) + 'static) -> Self {
        self.event_watchers.push(Box::new(watcher));
        self
    }

    /// Injects a proxy for the same deferred web view into `content`.
    ///
    /// The content and web view are created together at render time, so every
    /// extracted [`WebViewProxy`] targets the handle displayed by this view.
    ///
    /// The web view is placed below `content`. When you need any other
    /// arrangement, open the web view yourself and use
    /// [`WebView::proxy_scope`], which attaches the proxy without placing
    /// anything.
    ///
    /// # Panics
    ///
    /// Panics when rendered without a [`WebViewController`] in the environment.
    pub fn with_proxy<V, F>(self, content: F) -> impl View
    where
        V: View,
        F: FnOnce() -> V + 'static,
    {
        use waterui_core::env::with;
        waterui_core::env::UseEnv::new(move |env: &Environment| {
            let controller = env
                .get::<WebViewController>()
                .cloned()
                .expect("a WebView needs a WebViewController in its environment");
            let webview = self.create(&controller, env);
            let scoped = with(content(), WebViewProxy::new(webview.handle().clone()));
            vstack((scoped, webview))
        })
    }

    fn create(self, controller: &WebViewController, environment: &Environment) -> WebView {
        let Self {
            url,
            redirects_enabled,
            user_agent,
            scripts,
            handlers,
            event_watchers,
            state,
            bridge_origins,
        } = self;
        let webview = controller.open();
        // The policy is resolved against the URL the view opens at, and it goes
        // in before the first handler exists, so no handler is ever reachable
        // unguarded. It used to be installed last, after every handler was
        // already registered; nothing had navigated yet, so it was only ever
        // safe by accident.
        webview
            .handle()
            .set_bridge_origins(OriginPolicy::new(bridge_origins, &url.get()));
        if let Some(enabled) = redirects_enabled {
            webview.set_redirects_enabled(enabled);
        }
        for (key, script, time) in scripts {
            webview.inject_script(key.as_str(), script.as_str(), time);
        }
        for (name, build) in handlers {
            webview
                .handle()
                .add_handler(name.as_str(), build(environment.clone()));
        }
        for watcher in event_watchers {
            // `on_event` observers see user-facing events only; navigation state
            // is `can_go_back`/`can_go_forward`, not something to match on.
            // The web view owns these for its whole life, which is exactly the
            // case `WatcherGuard::forget` documents.
            webview
                .handle()
                .watch(move |backend_event| {
                    if let BackendEvent::Event(event) = backend_event {
                        watcher(event);
                    }
                })
                .forget();
        }
        if !state.is_empty() {
            state::install(&webview, state);
        }
        let webview = webview.bind_navigation(url);
        if let Some(user_agent) = user_agent {
            webview.bind_user_agent(user_agent)
        } else {
            webview
        }
    }
}

impl View for WebViewOpen {
    fn body(self, _env: &Environment) -> impl View {
        waterui_core::env::UseEnv::new(move |env: &Environment| {
            let controller = env
                .get::<WebViewController>()
                .cloned()
                .expect("a WebView needs a WebViewController in its environment");
            self.create(&controller, env)
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

// `WebView` is a raw view: a backend with a web engine intercepts it before
// `View::body` ever runs. `raw_view!` is not used because reaching `body` must not
// abort — see the fallback below.
impl waterui_core::NativeView for WebView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for WebView {
    fn body(self, _env: &Environment) -> impl View {
        // Reaching `body` means the running backend has no web engine compiled in.
        // Render an empty, still-accessible leaf rather than panicking: every
        // component owes the accessibility tree a node, and a backend without an
        // engine is a missing feature, not a crash.
        Native::new(self).with_fallback(spacer())
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::{WebView, subscribe_navigation};
    use waterui_core::{Binding, Signal, reactive::watcher::Context};
    use waterui_url::Url;

    /// The redirect policy is a signal the web view keeps reading, not a `bool`
    /// sampled once while the view was being described. Writing to the binding
    /// after the fact has to reach the web view, so what the builder stores must
    /// still be attached to the source.
    #[test]
    fn redirect_policy_retains_the_reactive_signal() {
        let redirects = Binding::container(false);
        let open = WebView::open("https://waterui.dev").redirects_enabled(redirects.clone());
        let configured = open
            .redirects_enabled
            .expect("the redirect policy was configured");

        assert!(!configured.get());
        redirects.set(true);
        assert!(configured.get());
    }

    #[derive(Clone)]
    struct EmitsDuringSubscription {
        source: Binding<Url>,
        replacement: Url,
    }

    impl Signal for EmitsDuringSubscription {
        type Output = Url;
        type Guard = <Binding<Url> as Signal>::Guard;

        fn get(&self) -> Self::Output {
            self.source.get()
        }

        fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
            self.source.set(self.replacement.clone());
            let watcher = Rc::new(watcher);
            watcher(Context::from(self.replacement.clone()));
            self.source.watch(move |context| watcher(context))
        }
    }

    #[test]
    fn navigation_subscription_does_not_repeat_synchronous_emission() {
        let replacement = Url::new("https://waterui.dev/docs");
        let source = Binding::container(Url::new("https://waterui.dev"));
        let signal = EmitsDuringSubscription {
            source: source.clone(),
            replacement: replacement.clone(),
        };
        let navigations = Rc::new(RefCell::new(Vec::new()));

        let _guard = subscribe_navigation(&signal, {
            let navigations = Rc::clone(&navigations);
            move |url| navigations.borrow_mut().push(url)
        });
        let next = Url::new("https://waterui.dev/components");
        source.set(next.clone());

        assert_eq!(*navigations.borrow(), vec![replacement, next]);
    }
}
