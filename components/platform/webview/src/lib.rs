//! `WebView` component for `WaterUI` framework.
//!
//! This module provides a web view component for embedding web content in `WaterUI` applications.
//!
//! # Architecture
//!
//! - [`WebViewHandle`] - Imperative trait that native backends implement
//! - [`AnyWebViewHandle`] - Type-erased wrapper with downcast support
//! - [`WebViewController`] - Engine-specific factory exposed through `NativeBackend::WebViewController`
//! - [`WebView`] - Reactive wrapper with `Binding<T>` state
//!
//! # Platform behaviour
//!
//! On macOS, a web view consumes every scroll-wheel event over its uncovered
//! area, including when the page cannot scroll in that direction: `WKWebView`
//! never forwards an unhandled wheel event and exposes no public way to learn
//! it went unused. A web view whose page does not scroll therefore does not
//! chain the wheel to an enclosing scroll view while the pointer is over it,
//! as with `WKWebView` inside an `NSScrollView` in an `AppKit` app. Touch
//! scrolling and other platforms are not affected.
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

pub mod assets;
#[cfg(not(target_arch = "wasm32"))]
mod bundled;
mod config;
mod controller;
#[cfg(not(target_arch = "wasm32"))]
mod cookies;

pub use assets::{
    ASSET_HOST, ASSET_HTTPS_HOST, ASSET_HTTPS_ORIGIN, ASSET_ORIGIN, ASSET_SCHEME, AssetMethod,
    AssetRequest, AssetResponse, AssetServer,
};
#[cfg(not(target_arch = "wasm32"))]
pub use bundled::{DEFAULT_CSP, DirectoryServer, dev_url};
pub use config::WebViewConfig;
#[cfg(not(target_arch = "wasm32"))]
pub use config::{DocumentScripts, UserScript};
pub use controller::*;
#[cfg(not(target_arch = "wasm32"))]
pub use cookie::Cookie;
#[cfg(not(target_arch = "wasm32"))]
pub use cookies::cookie_expiry;
use std::{cell::Cell, fmt, rc::Rc};
#[macro_use]
mod handle_layers;
mod handler;
pub use handler::{AnyWebViewHandle, WeakWebViewHandle, WebViewHandle};
#[cfg(not(target_arch = "wasm32"))]
pub use handler::{HandlerFuture, HandlerResult, ScriptInjectionTime, ScriptMessageHandler};
use handler::{Opened, opened_handle};
mod unavailable;
pub use unavailable::{UnavailableWebViewController, UnavailableWebViewHandle, WebViewOpenError};
#[cfg(all(feature = "conformance", not(target_arch = "wasm32")))]
pub mod conformance;
mod proxy;
pub use proxy::WebViewProxy;
use proxy::{DeferredProxySlot, WebViewPageState};

// Re-export waterui-internal types for FFI layer
pub use waterui_url::{IntoUrl, Url};
pub use waterui_watcher_set::{WatcherGuard, WatcherSet};
#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;
#[cfg(not(target_arch = "wasm32"))]
mod script;
#[cfg(not(target_arch = "wasm32"))]
pub use script::{
    BRIDGE_SCRIPT, DOCUMENT_START_SCRIPT, EVAL_SCRIPT, JsError, JsExpr, JsOutcome, JsProgram,
    raw_evaluation,
};
#[cfg(not(target_arch = "wasm32"))]
mod navigation;
/// The Apple OS version `WebViewOpen::navigation_policy` needs, as `(major,
/// minor)`: `WKNavigationAction.mainFrameNavigation` is public from there. The
/// `web-frontend-apple-deployment-targets` framework metadata names the same
/// version for every Apple platform, which the tests check.
#[cfg(not(target_arch = "wasm32"))]
const NAVIGATION_POLICY_OS: (u16, u16) = (27, 0);
#[cfg(not(target_arch = "wasm32"))]
type NavigationPolicyOs =
    apple_availability::AppleOs<{ NAVIGATION_POLICY_OS.0 }, { NAVIGATION_POLICY_OS.1 }>;
#[cfg(not(target_arch = "wasm32"))]
pub use navigation::{NavigationDecision, NavigationPolicy, NavigationRequest, stay_on_origin};
#[cfg(not(target_arch = "wasm32"))]
mod origins;
#[cfg(not(target_arch = "wasm32"))]
pub use origins::{BridgeOrigins, IntoBridgeOrigins, OriginPolicy, OriginRule};

#[cfg(not(target_arch = "wasm32"))]
mod js_api;
#[cfg(not(target_arch = "wasm32"))]
pub use js_api::{
    JsApi, JsApiMethod, JsApiParam, JsApiReturn, JsApiSchema, JsApiStateField, JsReplySchema,
    typescript_declarations,
};
pub use waterui_ts_schema::{self as ts_schema, TsType};

#[cfg(not(target_arch = "wasm32"))]
/// Re-exported so `#[js_api]` can derive `Deserialize` for its argument structs
/// without the calling crate having to depend on `serde` itself.
#[doc(hidden)]
pub use serde;
#[cfg(not(target_arch = "wasm32"))]
/// Re-exported for the same reason as [`serde`]: `exec!` and `eval!` serialize
/// each `@{...}` hole, and the calling crate must not have to depend on
/// `serde_json` just to write a JavaScript literal.
#[doc(hidden)]
pub use serde_json;
#[cfg(not(target_arch = "wasm32"))]
mod big_integers;
#[cfg(not(target_arch = "wasm32"))]
mod message;
#[cfg(not(target_arch = "wasm32"))]
pub use message::{Bytes, HandlerName, IntoJsReply, JsReply, Json};
#[cfg(not(target_arch = "wasm32"))]
mod state;
#[cfg(not(target_arch = "wasm32"))]
pub use state::{FieldEntry, JsField, StateDeclarations, StateWriteError};

use suiteki::Str;
use waterui_core::{
    Binding, Computed, Environment, Native, Signal, State, View, binding, env::with,
    layout::StretchAxis, reactive::signal::IntoComputed,
};

/// Something that happened in a `WebView`.
///
/// There is no "nothing happened" variant: the absence of an event is expressed
/// by [`WebView::events`] yielding `None`, which is the same fact stated in the
/// type rather than smuggled into the enum. Nor is there a variant for
/// navigation state — that is [`WebView::can_go_back`] and
/// [`WebView::can_go_forward`], and it used to appear here as a `#[doc(hidden)]`
/// variant every `match` still had to spell out.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum BackendEvent {
    /// Something worth telling the application about.
    Event(WebViewEvent),
    #[cfg(not(target_arch = "wasm32"))]
    /// The history changed.
    NavigationState {
        /// Whether the web view can navigate back.
        can_go_back: bool,
        /// Whether the web view can navigate forward.
        can_go_forward: bool,
    },
    /// The current page title changed.
    TitleChanged(Str),
    /// The current page URL changed.
    UrlChanged(Url),
}

/// Errors that can occur in the `WebView` component.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
/// the native backend emits `WebViewEvent::StateChanged` events.
///
/// `WebView` implements [`View`] so it can be used directly in the view hierarchy.
pub struct WebView {
    event: Binding<Option<WebViewEvent>>,
    handle: AnyWebViewHandle,
    state: WebViewPageState,
    /// Signal subscriptions that must outlive this view: the URL and user-agent
    /// bindings. Erased because their value types differ; only their lifetime
    /// matters here.
    retained: Vec<Rc<dyn core::any::Any>>,
    /// Keeps the internal watcher that drives page state and the public event
    /// signal subscribed for as long as any clone lives.
    state_watcher: Rc<WatcherGuard>,
}

impl Clone for WebView {
    fn clone(&self) -> Self {
        Self {
            event: self.event.clone(),
            handle: self.handle.clone(),
            state: self.state.clone(),
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
            .field("state", &self.state)
            .field("retained_subscriptions", &self.retained.len())
            .finish_non_exhaustive()
    }
}

impl WebView {
    /// Creates a new `WebView` component with the given handle.
    #[must_use]
    pub(crate) fn from_handle(handle: AnyWebViewHandle) -> Self {
        Self::from_handle_with_state(handle, WebViewPageState::new())
    }

    #[must_use]
    fn from_handle_with_state(handle: AnyWebViewHandle, state: WebViewPageState) -> Self {
        let event: Binding<Option<WebViewEvent>> = binding(None);
        #[cfg(not(target_arch = "wasm32"))]
        state.seed_navigation_state(&handle);

        let state_watcher = handle.watch({
            let event = event.clone();
            let state = state.clone();
            move |backend_event| {
                state.apply(&backend_event);
                if let BackendEvent::Event(webview_event) = backend_event {
                    event.set(Some(webview_event));
                }
            }
        });

        Self {
            handle,
            event,
            state,
            retained: Vec::new(),
            state_watcher: Rc::new(state_watcher),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
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
    /// [`WebViewOpen::proxy_scope`] to obtain a [`WebViewProxy`] with live
    /// page state and imperative commands while choosing your own layout.
    ///
    /// `url` is reactive: writing a new [`Url`] into a bound `Binding<Url>`
    /// navigates the existing native web view without rebuilding it.
    ///
    /// ```ignore
    /// let url = binding(Url::new("https://waterui.dev"));
    /// let webview = WebView::open(url.clone());
    /// url.set(Url::new("https://waterui.dev/docs"));
    /// ```
    pub fn open(url: impl IntoComputed<Url>) -> WebViewOpen {
        WebViewOpen {
            target: WebViewTarget::Url(url.into_computed()),
            #[cfg(not(target_arch = "wasm32"))]
            redirects_enabled: None,
            #[cfg(not(target_arch = "wasm32"))]
            user_agent: None,
            #[cfg(not(target_arch = "wasm32"))]
            scripts: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            handlers: Vec::new(),
            event_watchers: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            state: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            bridge_origins: BridgeOrigins::default(),
            #[cfg(not(target_arch = "wasm32"))]
            navigation_policy: None,
            proxy_slot: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Opens a new `WebView` serving `server` under the engine's local asset
    /// origin, and navigates it to `entry`.
    ///
    /// The engine answers requests on that origin by calling `server` rather
    /// than the network: `waterui://localhost` on `WKWebView`, `WebKitGTK`, WPE and
    /// CEF, `https://waterui.localhost` on Android's `WebView` and CDP-driven
    /// Chromium. The page it produces is a secure context on every engine, so
    /// bundled web content keeps `isSecureContext`, module scripts, `fetch`
    /// and WASM without touching `file://`.
    ///
    /// `server` is [`AssetServer`]: GET and HEAD only, paths handed over still
    /// percent-encoded, traversal refused before the server is consulted. See
    /// [`assets`] for the contract every engine routes through.
    ///
    /// # Contract violation
    ///
    /// At render time, when the selected [`WebViewController`]'s engine has no
    /// interception facility to stand an asset origin on: a debug build
    /// panics, a release build logs it and the view is not opened.
    pub fn open_assets(
        server: impl Fn(&AssetRequest) -> AssetResponse + Send + Sync + 'static,
        entry: impl Into<Str>,
    ) -> WebViewOpen {
        WebViewOpen {
            target: WebViewTarget::Assets {
                server: std::sync::Arc::new(server),
                entry: entry.into(),
            },
            redirects_enabled: None,
            user_agent: None,
            scripts: Vec::new(),
            handlers: Vec::new(),
            event_watchers: Vec::new(),
            state: Vec::new(),
            bridge_origins: BridgeOrigins::default(),
            navigation_policy: None,
            proxy_slot: None,
        }
    }

    /// Returns a [`WebViewProxy`] sharing this web view's handle and reactive
    /// page state.
    #[must_use]
    pub fn proxy(&self) -> WebViewProxy {
        WebViewProxy::from_parts(
            self.handle.clone(),
            self.state.clone(),
            Rc::clone(&self.state_watcher),
        )
    }

    /// Injects this web view's [`WebViewProxy`] into `content`'s environment
    /// and passes it directly to `content`.
    ///
    /// Controls built inside `content` can read live page state
    /// (`can_go_back`, `can_go_forward`, `url`, `title`, `progress`) from the
    /// proxy argument, and handlers inside `content` can extract `WebViewProxy`
    /// through the same `Extractor` machinery that powers `Button::action`.
    ///
    /// This returns `content` with the proxy attached and nothing else: it does
    /// not place the web view, so the layout is yours.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use waterui::prelude::*;
    /// use waterui_webview::{WebView, WebViewProxy};
    ///
    /// let toolbar = webview.proxy_scope(|web| {
    ///     hstack((
    ///         button("←")
    ///             .action(|p: WebViewProxy| p.go_back())
    ///             .disabled(web.can_go_back().map(|can| !can)),
    ///         button("→")
    ///             .action(|p: WebViewProxy| p.go_forward())
    ///             .disabled(web.can_go_forward().map(|can| !can)),
    ///         text(web.title()),
    ///         progress(web.progress()),
    ///     ))
    /// });
    /// vstack((toolbar, backend_surface))   // or hstack, or zstack — your choice
    /// ```
    pub fn proxy_scope<V, F>(&self, content: F) -> impl View + use<V, F>
    where
        V: View,
        F: FnOnce(WebViewProxy) -> V,
    {
        let proxy = self.proxy();
        with(content(proxy.clone()), State(proxy))
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Applies `user_agent` now and on every later change.
    fn bind_user_agent(mut self, user_agent: Computed<Str>) -> Self {
        let handle = self.handle.clone();
        let guard = user_agent.watch(move |context| {
            handle.set_user_agent(context.into_value().as_str());
        });
        self.handle.set_user_agent(user_agent.snapshot().as_str());
        self.retained.push(Rc::new((user_agent, guard)));
        self
    }

    fn bind_navigation(mut self, url: Computed<Url>) -> Self {
        let handle = self.handle.clone();
        let guard = subscribe_navigation(&url, move |url| handle.go_to(&url));
        self.retained.push(Rc::new((url, guard)));
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Refreshes the current page.
    pub fn refresh(&self) {
        self.handle.refresh();
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Stops the current loading operation.
    pub fn stop(&self) {
        self.handle.stop();
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Navigates back in the web view's history.
    pub fn go_back(&self) {
        self.handle.go_back();
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Navigates forward in the web view's history.
    pub fn go_forward(&self) {
        self.handle.go_forward();
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Returns a reactive signal for whether the web view can navigate back.
    #[must_use]
    pub fn can_go_back(&self) -> Computed<bool> {
        self.state.can_go_back()
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Returns a reactive signal for whether the web view can navigate forward.
    #[must_use]
    pub fn can_go_forward(&self) -> Computed<bool> {
        self.state.can_go_forward()
    }

    /// Returns a reactive signal for the current page URL, or `None` before
    /// any navigation has started.
    #[must_use]
    pub fn url(&self) -> Computed<Option<Url>> {
        self.state.url()
    }

    /// Returns a reactive signal for the current page title.
    #[must_use]
    pub fn title(&self) -> Computed<Str> {
        self.state.title()
    }

    /// Returns a reactive signal for the current load progress (`0.0` to `1.0`).
    #[must_use]
    pub fn progress(&self) -> Computed<f64> {
        self.state.progress()
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Evaluates the JavaScript expression `script` in the web view and answers
    /// the JSON encoding of its value.
    ///
    /// See [`WebViewHandle::run_javascript`] for the contract every engine
    /// meets. [`eval`](Self::eval) is the typed form, and the one that awaits a
    /// promise.
    ///
    /// # Errors
    ///
    /// Returns the engine's message when the expression throws, does not parse,
    /// or has a value `JSON.stringify` rejects.
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Evaluates a JavaScript expression and decodes its result as `T`.
    ///
    /// Prefer the `eval!` macro, which checks the source
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Runs a JavaScript program for its effects.
    ///
    /// Prefer the `exec!` macro.
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Runs a wrapped call and parses the envelope it returns.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and JavaScript execution are main-thread-affine"
    )]
    async fn run_wrapped(&self, call: &str) -> Result<JsOutcome, JsError> {
        run_wrapped_on(&self.handle, call).await
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets a cookie in this web view's native cookie store.
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        self.handle.set_cookie(cookie);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Retrieves the current cookies without blocking the UI thread.
    ///
    /// # Errors
    ///
    /// As documented on [`WebViewHandle::get_cookies`]: a store that cannot be
    /// read, or a cookie it returns that cannot be represented, fails rather
    /// than reporting an empty list.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and cookie stores are main-thread-affine"
    )]
    pub fn get_cookies(
        &self,
    ) -> impl core::future::Future<Output = Result<Vec<Cookie<'static>>, waterui_core::Error>> + '_
    {
        self.handle.get_cookies()
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        self.handle.set_user_agent(user_agent);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Injects a script that will run on every later page load.
    ///
    /// `key` names the script. Injecting again under the same key replaces it,
    /// so a script whose content depends on current state can be kept current
    /// without stacking a new copy on every update.
    ///
    /// # Contract violation
    ///
    /// When `key` names a script the view was opened with
    /// ([`WebViewOpen::inject`]): those are fixed for its life, on every
    /// backend alike. A debug build panics, a release build logs it and
    /// ignores the call.
    pub fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        self.handle.inject_script(key, script, time);
    }

    #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
/// Runs a wrapped call on `handle` and parses the envelope it resolves with.
///
/// Free rather than a method on [`WebView`] because the conformance checks
/// drive a bare [`AnyWebViewHandle`] that no [`WebView`] owns.
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
        navigate(url.snapshot());
    }
    guard
}

#[cfg(not(target_arch = "wasm32"))]
/// A handler the page can call, boxed for storage before the web view exists.
/// A handler recorded before the web view exists. Built once the rendering
/// environment is known, so its extractors resolve against the right scope.
type BoxedMessageHandler = Box<dyn FnOnce(Environment) -> Box<ScriptMessageHandler>>;

/// Where a [`WebViewOpen`] points the native view once it exists.
enum WebViewTarget {
    /// A reactive URL — writing a new one navigates the live view.
    Url(Computed<Url>),
    #[cfg(not(target_arch = "wasm32"))]
    /// A bundled asset set served through the engine's local asset origin.
    Assets {
        /// The serving function behind the origin.
        server: AssetServer,
        /// The path inside the asset set the view navigates to.
        entry: Str,
    },
}

impl fmt::Debug for WebViewTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => f.debug_tuple("Url").field(url).finish(),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Assets { entry, .. } => f
                .debug_struct("Assets")
                .field("entry", entry)
                .finish_non_exhaustive(),
        }
    }
}

/// A deferred web view created by [`WebView::open`].
///
/// Native creation happens when the backend realizes this view through its
/// [`WebViewController`]. Everything configured here is applied to the opened
/// handle the moment it exists, so a web view can be fully described before a
/// backend is anywhere in sight.
#[must_use = "a WebViewOpen must be rendered to create its native web view"]
pub struct WebViewOpen {
    target: WebViewTarget,
    #[cfg(not(target_arch = "wasm32"))]
    redirects_enabled: Option<Computed<bool>>,
    #[cfg(not(target_arch = "wasm32"))]
    user_agent: Option<Computed<Str>>,
    #[cfg(not(target_arch = "wasm32"))]
    scripts: Vec<UserScript>,
    #[cfg(not(target_arch = "wasm32"))]
    handlers: Vec<(Str, BoxedMessageHandler)>,
    event_watchers: Vec<Box<dyn Fn(WebViewEvent)>>,
    #[cfg(not(target_arch = "wasm32"))]
    state: Vec<state::PendingField>,
    #[cfg(not(target_arch = "wasm32"))]
    bridge_origins: BridgeOrigins,
    #[cfg(not(target_arch = "wasm32"))]
    navigation_policy: Option<NavigationPolicy>,
    proxy_slot: Option<DeferredProxySlot>,
}

/// The result of realizing a [`WebViewOpen`] with a concrete
/// [`WebViewController`], carrying both the typed backend handle and the
/// reactive [`WebView`] keeping subscriptions alive.
pub struct OpenedWebView<H: WebViewHandle> {
    typed: Rc<Opened<H>>,
    webview: WebView,
}

impl<H: WebViewHandle> fmt::Debug for OpenedWebView<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenedWebView")
            .field("webview", &self.webview)
            .finish_non_exhaustive()
    }
}

impl<H: WebViewHandle> OpenedWebView<H> {
    /// Returns the concrete backend handle opened for this web view.
    #[must_use]
    pub fn handle(&self) -> &H {
        opened_handle(&self.typed)
    }

    /// Returns the reactive [`WebView`] wrapper owning subscriptions for this view.
    #[must_use]
    pub const fn webview(&self) -> &WebView {
        &self.webview
    }

    /// Consumes this wrapper and returns the reactive [`WebView`].
    #[must_use]
    pub fn into_webview(self) -> WebView {
        self.webview
    }
}

impl fmt::Debug for WebViewOpen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("WebViewOpen");
        debug.field("target", &self.target);
        #[cfg(not(target_arch = "wasm32"))]
        debug
            .field("redirects_enabled", &self.redirects_enabled)
            .field("user_agent", &self.user_agent)
            .field("scripts", &self.scripts.len())
            .field("handlers", &self.handlers.len());
        debug.field("event_watchers", &self.event_watchers.len());
        #[cfg(not(target_arch = "wasm32"))]
        debug
            .field("state_fields", &self.state.len())
            .field("bridge_origins", &self.bridge_origins)
            .field("navigation_policy", &self.navigation_policy.is_some());
        debug
            .field("proxy_slot", &self.proxy_slot.is_some())
            .finish()
    }
}

impl WebViewOpen {
    #[cfg(not(target_arch = "wasm32"))]
    /// Sets the reactive redirect policy applied when the web view is created.
    pub fn redirects_enabled(mut self, enabled: impl IntoComputed<bool>) -> Self {
        self.redirects_enabled = Some(enabled.into_computed());
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets the user agent, reactively.
    pub fn user_agent(mut self, user_agent: impl IntoComputed<Str>) -> Self {
        self.user_agent = Some(user_agent.into_computed());
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Injects a script that runs on every page load the bridge policy admits.
    ///
    /// `key` names the script; injecting again under the same key replaces it.
    /// The scripts reach the engine with the view's creation-time
    /// configuration, so the first document already runs them.
    pub fn inject(
        mut self,
        key: impl Into<Str>,
        script: impl Into<Str>,
        time: ScriptInjectionTime,
    ) -> Self {
        let script = UserScript::new(key, script, time);
        match self
            .scripts
            .iter_mut()
            .find(|existing| existing.key() == script.key())
        {
            Some(existing) => *existing = script,
            None => self.scripts.push(script),
        }
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Registers a handler the page can call as `waterui.invoke(name, payload)`.
    ///
    /// # Contract violation
    ///
    /// When rendered without a [`WebViewController`] in the environment: a
    /// debug build panics, a release build logs it and renders nothing.
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Mirrors a reactive value into the page.
    ///
    /// The page reads it as `waterui.state.<name>` — a local property, so reading
    /// never crosses the boundary — and subscribes with `waterui.watch(name, cb)`.
    /// A [`Binding`] is writable from JavaScript and the write flows back into it;
    /// anything else is read-only, and assigning to it throws.
    ///
    /// No value is part of what the engine injects: a document asks for the
    /// values once it starts, and `waterui.ready` resolves when they arrive.
    /// Reading or assigning a key before then throws rather than producing a
    /// made-up value, so a page reads state after `await waterui.ready`;
    /// `waterui.watch` may be called earlier, and its callback hears the first
    /// values.
    ///
    /// Granularity follows the signal: exposing a `Binding<BigStruct>` and
    /// changing one field re-sends the whole struct. Expose finer bindings when
    /// that matters.
    pub fn expose<F: state::JsField + 'static>(mut self, name: impl Into<Str>, field: F) -> Self {
        self.state.push(state::pending_field(name.into(), field));
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Exposes an object's methods and state to the page in one step.
    ///
    /// Written by hand this is a `.handler(...)` per method and an `.expose(...)`
    /// per field; `#[js_api]` generates exactly that, so the two are equivalent
    /// and the macro adds nothing the manual path cannot express.
    pub fn serve<A: JsApi>(self, api: A) -> Self {
        A::register(std::rc::Rc::new(api), self)
    }

    #[cfg(not(target_arch = "wasm32"))]
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets the policy deciding the navigations the page initiates.
    ///
    /// The engine asks synchronously — `WKWebView`'s
    /// `decidePolicyForNavigationAction`, Android's `shouldOverrideUrlLoading`
    /// — for link activations, `window.location` writes, form submissions,
    /// `window.open` and subframe loads. Loads the application directs — the
    /// target the view was opened at and every write to its URL signal —
    /// are never consulted, and server redirects belong to
    /// [`redirects_enabled`](Self::redirects_enabled), not this policy.
    /// Calling it again replaces the earlier policy.
    ///
    /// Android's `WebView` never offers a `POST` form submission or an
    /// `http(s)` subframe load to its hook, so on Android the policy does
    /// not see those navigations.
    ///
    /// On `WKWebView` a *subframe's* server redirect is offered to the
    /// policy with `is_main_frame == false`: the engine re-asks the hook
    /// for every redirect, and exposes no public signal marking a
    /// subframe's action as one, so the policy sees it as an ordinary
    /// subframe navigation.
    ///
    /// # Apple OS 27
    ///
    /// `WKWebView` tells the load the application issued from the page's own
    /// through `WKNavigationAction.mainFrameNavigation`, public API from
    /// macOS and iOS 27. A build targeting an earlier Apple OS does not
    /// compile a call to this method: the policy's type must satisfy
    /// [`apple_availability::Available`], which holds only when the deployment
    /// target reaches OS 27. Raise the platform under
    /// `[apple.deployment-targets]` in `Water.toml` to `27.0`; other targets
    /// are unaffected. `include_web!` installs [`stay_on_origin`] through this
    /// method and needs the same.
    pub fn navigation_policy<F>(self, policy: F) -> Self
    where
        F: Fn(&NavigationRequest) -> NavigationDecision + Send + Sync + 'static,
        F: apple_availability::Available<NavigationPolicyOs, NavigationPolicy>,
    {
        self.with_navigation_policy(NavigationPolicy::new(policy))
    }

    /// Installs `policy`, replacing the earlier one. The gate on
    /// [`navigation_policy`](Self::navigation_policy) is the only thing it
    /// leaves out.
    #[cfg(not(target_arch = "wasm32"))]
    fn with_navigation_policy(mut self, policy: NavigationPolicy) -> Self {
        self.navigation_policy = Some(policy);
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

    /// Provides a [`WebViewProxy`] bound to this deferred web view while
    /// letting `content` place the view itself in any layout.
    ///
    /// `content` receives both this [`WebViewOpen`] (with all configured
    /// scripts, handlers, state bindings, navigation policy, and event
    /// watchers intact) and its [`WebViewProxy`]. The proxy is also injected
    /// into the returned view's environment so action handlers inside can
    /// extract `WebViewProxy`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use waterui::prelude::*;
    /// use waterui_webview::{WebView, WebViewProxy};
    ///
    /// WebView::open("https://waterui.dev")
    ///     .handler("greet", greet)
    ///     .proxy_scope(|page, web| {
    ///         vstack((
    ///             page,
    ///             hstack((
    ///                 button("←")
    ///                     .action(|p: WebViewProxy| p.go_back())
    ///                     .disabled(web.can_go_back().map(|can| !can)),
    ///                 text(web.title()),
    ///                 progress(web.progress()),
    ///             )),
    ///         ))
    ///     })
    /// ```
    pub fn proxy_scope<V, F>(mut self, content: F) -> impl View
    where
        V: View,
        F: FnOnce(Self, WebViewProxy) -> V,
    {
        let (proxy, slot) = WebViewProxy::deferred();
        self.proxy_slot = Some(slot);
        with(content(self, proxy.clone()), State(proxy))
    }

    /// Opens this deferred web view with `controller`, wiring its initial URL,
    /// scripts, handlers, event watchers, and any [`WebViewProxy`] scope.
    ///
    /// Returns `None` after reporting a contract violation if the engine failed
    /// to open the view or had no asset origin for `WebView::open_assets`.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            unused_variables,
            reason = "the environment reaches only the page's handlers, which the web has none of"
        )
    )]
    pub fn open_with<C: WebViewController>(
        self,
        controller: &C,
        environment: &Environment,
    ) -> Option<OpenedWebView<C::Handle>> {
        let Self {
            target,
            #[cfg(not(target_arch = "wasm32"))]
            redirects_enabled,
            #[cfg(not(target_arch = "wasm32"))]
            user_agent,
            #[cfg(not(target_arch = "wasm32"))]
            scripts,
            #[cfg(not(target_arch = "wasm32"))]
            handlers,
            event_watchers,
            #[cfg(not(target_arch = "wasm32"))]
            state,
            #[cfg(not(target_arch = "wasm32"))]
            bridge_origins,
            #[cfg(not(target_arch = "wasm32"))]
            navigation_policy,
            proxy_slot,
        } = self;
        // Everything a document may receive before its first line of script
        // is resolved here, before the engine creates anything: the URL the
        // view opens at, the admission rules resolved against it, the
        // application's scripts and the state declarations.
        #[cfg(not(target_arch = "wasm32"))]
        let (initial, url, asset_server) = match target {
            WebViewTarget::Url(url) => (url.snapshot(), Some(url), None),
            WebViewTarget::Assets { server, entry } => {
                // The origin is the engine's answer, not a constant the shared
                // layer guesses: it differs across engines by design.
                let Some(entry_url) = asset_entry_url(controller, &entry) else {
                    if let Some(slot) = proxy_slot {
                        slot.bind_unavailable(AnyWebViewHandle::unavailable());
                    }
                    return None;
                };
                (entry_url, None, Some(server))
            }
        };
        #[cfg(target_arch = "wasm32")]
        let (initial, url) = match target {
            WebViewTarget::Url(url) => (url.snapshot(), Some(url)),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let mirror = (!state.is_empty()).then(|| state::prepare(state));
        #[cfg(not(target_arch = "wasm32"))]
        let config = WebViewConfig::new(
            asset_server,
            OriginPolicy::new(bridge_origins, &initial),
            navigation_policy,
            scripts,
            mirror.as_ref().map_or_default(state::Mirror::declarations),
        );
        #[cfg(target_arch = "wasm32")]
        let config = WebViewConfig::default();
        let (handle, typed) =
            match AnyWebViewHandle::open_typed(config, |config| controller.open(config)) {
                Ok(opened) => opened,
                Err(unavailable) => {
                    if let Some(slot) = proxy_slot {
                        slot.bind_unavailable(unavailable);
                    }
                    return None;
                }
            };
        let state = proxy_slot.map_or_else(WebViewPageState::new, |slot| slot.bind(handle.clone()));
        let webview = WebView::from_handle_with_state(handle, state);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(enabled) = redirects_enabled {
            webview.set_redirects_enabled(enabled);
        }
        #[cfg(not(target_arch = "wasm32"))]
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
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mirror) = mirror {
            state::attach(&webview, mirror);
        }
        let webview = if let Some(url) = url {
            webview.bind_navigation(url)
        } else {
            // An asset-origin view has no reactive URL; everything is
            // installed before the first navigation, then it goes to the entry
            // once.
            webview.handle().go_to(&initial);
            webview
        };
        #[cfg(not(target_arch = "wasm32"))]
        let webview = if let Some(user_agent) = user_agent {
            webview.bind_user_agent(user_agent)
        } else {
            webview
        };
        Some(OpenedWebView { typed, webview })
    }
}

/// The URL an asset-origin view opens at: `entry` under the origin the engine
/// serves bundled assets from, or `None` after reporting that the engine has
/// none.
#[cfg(not(target_arch = "wasm32"))]
fn asset_entry_url<C: WebViewController>(controller: &C, entry: &str) -> Option<Url> {
    // The origin is the engine's answer, not a constant the shared layer
    // guesses: it differs across engines by design.
    let Some(origin) = controller.asset_origin() else {
        waterui_core::contract_violation!(
            "WebView::open_assets requires an engine that exposes an \
             interceptable local asset origin; this WebViewController \
             has none; the view is not opened"
        );
        return None;
    };
    let url = format!(
        "{}/{}",
        origin.as_str().trim_end_matches('/'),
        entry.trim_start_matches('/')
    )
    .parse()
    .unwrap_or_else(|_| {
        waterui_core::contract_violation!(
            "the asset entry `{entry}` did not form a URL under {origin}; \
             the view opens at the origin's root"
        );
        origin.clone()
    });
    Some(url)
}

// `WebViewOpen` is realized by the backend through `NativeBackend::web_view`
// using `NativeBackend::WebViewController`. `raw_view!` is not used because
// reaching `body` must not abort.
impl waterui_core::NativeView for WebViewOpen {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for WebViewOpen {
    /// Realized by the backend through `NativeBackend::web_view`.
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::{WebView, WebViewProxy, subscribe_navigation};
    use waterui_core::{AnyView, Binding, Signal, View, reactive::watcher::Context};
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

        assert!(!configured.snapshot());
        redirects.set(true);
        assert!(configured.snapshot());
    }

    /// `proxy_scope` clones the handle it attaches, so the controls it returns
    /// must not borrow the web view: the documented layout moves the web view
    /// into the same container as its controls.
    #[test]
    fn proxy_scope_does_not_borrow_the_web_view() {
        fn controls_outlive_the_web_view<V: View>(
            webview: WebView,
            content: fn(WebViewProxy) -> V,
        ) -> impl View {
            let controls = webview.proxy_scope(content);
            drop(webview);
            controls
        }

        let _ = controls_outlive_the_web_view::<AnyView>;
    }

    #[derive(Clone)]
    struct EmitsDuringSubscription {
        source: Binding<Url>,
        replacement: Url,
    }

    impl Signal for EmitsDuringSubscription {
        type Output = Url;
        type Guard = <Binding<Url> as Signal>::Guard;

        fn snapshot(&self) -> Self::Output {
            self.source.snapshot()
        }

        fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
            self.source.set(self.replacement.clone());
            let watcher = Rc::new(watcher);
            watcher(Context::from(self.replacement.clone()));
            self.source.watch(move |context| watcher(context))
        }
    }

    /// `include_web!` installs `stay_on_origin` and an application's own
    /// call has to win — the builder keeps the last policy it was given.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_later_navigation_policy_replaces_the_earlier_one() {
        use super::{NavigationDecision, NavigationRequest, stay_on_origin};

        use super::NavigationPolicy;

        // Through the crate-internal setter: the public method is gated on the
        // Apple deployment target, which has nothing to do with replacement.
        let open = WebView::open("https://waterui.dev")
            .with_navigation_policy(NavigationPolicy::new(stay_on_origin))
            .with_navigation_policy(NavigationPolicy::new(|_| NavigationDecision::Cancel));
        let request = NavigationRequest::new(
            "https://waterui.dev/docs".parse().expect("a valid URL"),
            Some("https://waterui.dev".parse().expect("a valid URL")),
            true,
            true,
        );
        // `stay_on_origin` would allow this same-origin navigation; the
        // replacement does not.
        assert!(
            !open
                .navigation_policy
                .expect("a policy was configured")
                .allows(&request)
        );
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

    #[cfg(not(target_arch = "wasm32"))]
    mod proxy_scope_tests {
        use std::{
            cell::{Cell, RefCell},
            future::ready,
            rc::Rc,
        };

        use suiteki::Str;
        use waterui_core::{
            AnyView, Computed, Environment, Error, Metadata, Signal, View as _,
            extract::Extractor as _,
        };
        use waterui_url::Url;

        use crate::{
            BackendEvent, Cookie, ScriptInjectionTime, ScriptMessageHandler, WatcherGuard,
            WatcherSet, WebView, WebViewConfig, WebViewController, WebViewEvent, WebViewHandle,
            WebViewOpen, WebViewOpenError, WebViewProxy,
        };

        #[derive(Clone, Default)]
        struct FakeHandle {
            watchers: Rc<WatcherSet<BackendEvent>>,
            handlers: Rc<RefCell<Vec<String>>>,
            navigated: Rc<RefCell<Vec<Url>>>,
            went_back: Rc<Cell<bool>>,
            can_go_back: Rc<Cell<bool>>,
            can_go_forward: Rc<Cell<bool>>,
        }

        impl FakeHandle {
            fn emit(&self, event: &BackendEvent) {
                self.watchers.emit(event);
            }
        }

        impl WebViewHandle for FakeHandle {
            fn go_back(&self) {
                self.went_back.set(true);
            }

            fn go_forward(&self) {}

            fn go_to(&self, url: &Url) {
                self.navigated.borrow_mut().push(url.clone());
                self.emit(&BackendEvent::Event(WebViewEvent::WillNavigate {
                    url: url.clone(),
                }));
            }

            fn stop(&self) {}

            fn refresh(&self) {}

            fn set_user_agent(&self, _user_agent: &str) {}

            fn can_go_back(&self) -> bool {
                self.can_go_back.get()
            }

            fn can_go_forward(&self) -> bool {
                self.can_go_forward.get()
            }

            fn add_handler(&self, name: &str, _handler: Box<ScriptMessageHandler>) {
                self.handlers.borrow_mut().push(name.to_owned());
            }

            fn remove_handler(&self, _name: &str) {}

            fn set_cookie(&self, _cookie: Cookie<'static>) {}

            fn inject_script(&self, _key: &str, _script: &str, _time: ScriptInjectionTime) {}

            fn set_redirects_enabled(&self, _enabled: impl Signal<Output = bool>) {}

            fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
                self.watchers.insert(f)
            }

            fn get_cookies(&self) -> impl Future<Output = Result<Vec<Cookie<'static>>, Error>> {
                ready(Ok(Vec::new()))
            }

            #[expect(
                clippy::future_not_send,
                reason = "a web view and its handle are main-thread-affine"
            )]
            fn run_javascript(&self, _script: &str) -> impl Future<Output = Result<Str, Str>> {
                ready(Ok(Str::from_static("null")))
            }

            #[expect(
                clippy::future_not_send,
                reason = "a web view and its handle are main-thread-affine"
            )]
            fn call_async_javascript(&self, _body: &str) -> impl Future<Output = Result<Str, Str>> {
                ready(Ok(Str::from_static("null")))
            }
        }

        struct FakeController;

        impl WebViewController for FakeController {
            type Handle = FakeHandle;

            fn open(&self, _config: WebViewConfig) -> Result<Self::Handle, WebViewOpenError> {
                Ok(FakeHandle::default())
            }

            fn asset_origin(&self) -> Option<Url> {
                None
            }
        }

        #[test]
        fn proxy_scope_preserves_handlers_and_updates_page_state() {
            let captured_page: Rc<RefCell<Option<WebViewOpen>>> = Rc::new(RefCell::new(None));
            let captured_proxy: Rc<RefCell<Option<WebViewProxy>>> = Rc::new(RefCell::new(None));

            let scoped = WebView::open("https://waterui.dev")
                .handler("greet", || async { String::from("hi") })
                .proxy_scope({
                    let captured_page = Rc::clone(&captured_page);
                    let captured_proxy = Rc::clone(&captured_proxy);
                    move |page, web| {
                        *captured_page.borrow_mut() = Some(page);
                        *captured_proxy.borrow_mut() = Some(web);
                    }
                });

            let env_view = AnyView::new(scoped.body(&Environment::new()))
                .downcast::<Metadata<Environment, ()>>()
                .expect("proxy_scope injects WebViewProxy into the environment");
            let extracted =
                WebViewProxy::extract(&env_view.value).expect("WebViewProxy must be extractable");

            let web = captured_proxy
                .borrow_mut()
                .take()
                .expect("proxy_scope must pass WebViewProxy to its closure");
            let can_back: Computed<bool> = web.can_go_back();
            let can_forward: Computed<bool> = web.can_go_forward();
            let url = web.url();
            let title = web.title();
            let progress = web.progress();

            assert!(!can_back.snapshot());
            assert!(!can_forward.snapshot());
            assert_eq!(url.snapshot(), None);
            assert_eq!(title.snapshot().as_str(), "");
            assert!((progress.snapshot() - 0.0).abs() < f64::EPSILON);

            let page = captured_page
                .borrow_mut()
                .take()
                .expect("proxy_scope must pass WebViewOpen to its closure");
            let opened = page
                .open_with(&FakeController, &Environment::new())
                .expect("fake controller must open the web view");
            let handle = opened.handle();

            assert_eq!(handle.handlers.borrow().as_slice(), &["greet"]);
            assert_eq!(
                url.snapshot().as_ref().map(Url::as_str),
                Some("https://waterui.dev")
            );

            handle.emit(&BackendEvent::Event(WebViewEvent::Loading {
                progress: 0.4,
            }));
            assert!((progress.snapshot() - f64::from(0.4_f32)).abs() < 1e-6);

            handle.emit(&BackendEvent::TitleChanged(Str::from_static(
                "WaterUI Docs",
            )));
            assert_eq!(title.snapshot().as_str(), "WaterUI Docs");
            assert_eq!(opened.webview().title().snapshot().as_str(), "WaterUI Docs");

            let docs_url = Url::new("https://waterui.dev/docs");
            handle.emit(&BackendEvent::UrlChanged(docs_url.clone()));
            assert_eq!(url.snapshot(), Some(docs_url.clone()));
            assert_eq!(opened.webview().url().snapshot(), Some(docs_url));

            handle.emit(&BackendEvent::NavigationState {
                can_go_back: true,
                can_go_forward: false,
            });
            assert!(can_back.snapshot());
            assert!(!can_forward.snapshot());
            assert!(extracted.can_go_back().snapshot());

            handle.emit(&BackendEvent::Event(WebViewEvent::Loaded));
            assert!((progress.snapshot() - 1.0).abs() < f64::EPSILON);

            extracted.go_back();
            assert!(handle.went_back.get());
        }

        #[test]
        fn proxy_keeps_page_state_live_after_webview_is_dropped() {
            let opened = WebView::open("https://waterui.dev")
                .open_with(&FakeController, &Environment::new())
                .expect("fake controller must open the web view");
            let handle = opened.handle().clone();
            let webview = opened.into_webview();
            let proxy = webview.proxy();
            drop(webview);

            let title = proxy.title();
            let url = proxy.url();
            let progress = proxy.progress();

            handle.emit(&BackendEvent::TitleChanged(Str::from_static("After Drop")));
            let next_url = Url::new("https://waterui.dev/after-drop");
            handle.emit(&BackendEvent::UrlChanged(next_url.clone()));
            handle.emit(&BackendEvent::Event(WebViewEvent::Loading {
                progress: 0.75,
            }));

            assert_eq!(title.snapshot().as_str(), "After Drop");
            assert_eq!(url.snapshot(), Some(next_url));
            assert!((progress.snapshot() - f64::from(0.75_f32)).abs() < 1e-6);
        }
    }

    /// The framework metadata `water create` reads to scaffold a web frontend
    /// project names the Apple OS version `navigation_policy` is gated on, for
    /// every Apple platform.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn web_frontend_deployment_targets_match_the_navigation_policy_gate() {
        let manifest: toml::Value =
            toml::from_str(include_str!("../../../../Cargo.toml")).expect("a valid root manifest");
        let targets =
            manifest["package"]["metadata"]["waterui"]["web-frontend-apple-deployment-targets"]
                .as_table()
                .expect("the framework declares web-frontend-apple-deployment-targets");
        let (major, minor) = super::NAVIGATION_POLICY_OS;
        let gate = format!("{major}.{minor}");
        assert_eq!(
            targets.keys().map(String::as_str).collect::<Vec<_>>(),
            ["ios", "macos", "tvos", "visionos", "watchos"],
            "every Apple platform is declared"
        );
        for (platform, version) in targets {
            assert_eq!(version.as_str(), Some(gate.as_str()), "{platform}");
        }
    }
}
