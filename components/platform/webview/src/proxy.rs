//! Imperative and reactive proxy for [`WebView`](crate::WebView).
//!
//! `WebView`'s default surface is reactive — drive navigation by writing to a
//! URL `Binding`, observe loading state by reading reactive signals, and
//! handle events with callbacks. Use [`WebViewProxy`] when controls placed
//! outside the web view need both live page state (`can_go_back`,
//! `can_go_forward`, `url`, `title`, `progress`) and imperative commands
//! (`go_back`, `go_forward`, `refresh`, `stop`, `go_to`, `run_javascript`).
//!
//! Obtain a [`WebViewProxy`] through
//! [`WebViewOpen::proxy_scope`](crate::WebViewOpen::proxy_scope) or
//! [`WebView::proxy_scope`](crate::WebView::proxy_scope), which pass the proxy
//! directly to the scope closure and inject it into the rendering environment
//! so any handler inside can extract it the same way it extracts `State<T>`.

use std::{any::type_name, cell::OnceCell, fmt, rc::Rc};
#[cfg(not(target_arch = "wasm32"))]
use std::{future::ready, pin::Pin};

use suiteki::Str;
use waterui_core::extract::{ExtractionState, Extractor};
#[cfg(not(target_arch = "wasm32"))]
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{
    Binding, Computed, Environment, Error, binding, contract_violation, impl_debug,
};
use waterui_url::{IntoUrl, Url};

#[cfg(not(target_arch = "wasm32"))]
use crate::Cookie;
use crate::handler::AnyWebViewHandle;
#[cfg(not(target_arch = "wasm32"))]
use crate::handler::ScriptInjectionTime;
use crate::{BackendEvent, WatcherGuard, WebViewEvent};

/// Reactive page state shared between a [`WebView`](crate::WebView) and its
/// [`WebViewProxy`] handles.
#[derive(Clone)]
pub struct WebViewPageState {
    #[cfg(not(target_arch = "wasm32"))]
    can_go_back: Binding<bool>,
    #[cfg(not(target_arch = "wasm32"))]
    can_go_forward: Binding<bool>,
    url: Binding<Option<Url>>,
    title: Binding<Str>,
    progress: Binding<f64>,
}

impl fmt::Debug for WebViewPageState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("WebViewPageState");
        #[cfg(not(target_arch = "wasm32"))]
        debug
            .field("can_go_back", &self.can_go_back)
            .field("can_go_forward", &self.can_go_forward);
        debug
            .field("url", &self.url)
            .field("title", &self.title)
            .field("progress", &self.progress)
            .finish()
    }
}

impl WebViewPageState {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            can_go_back: binding(false),
            #[cfg(not(target_arch = "wasm32"))]
            can_go_forward: binding(false),
            url: binding(None),
            title: binding(Str::new()),
            progress: binding(0.0),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn seed_navigation_state(&self, handle: &AnyWebViewHandle) {
        self.can_go_back.set(handle.can_go_back());
        self.can_go_forward.set(handle.can_go_forward());
    }

    pub(crate) fn apply(&self, event: &BackendEvent) {
        match event {
            #[cfg(not(target_arch = "wasm32"))]
            BackendEvent::NavigationState {
                can_go_back,
                can_go_forward,
            } => {
                self.can_go_back.set(*can_go_back);
                self.can_go_forward.set(*can_go_forward);
            }
            BackendEvent::TitleChanged(title) => {
                self.title.set(title.clone());
            }
            BackendEvent::UrlChanged(url) => {
                self.url.set(Some(url.clone()));
            }
            BackendEvent::Event(WebViewEvent::WillNavigate { url }) => {
                self.url.set(Some(url.clone()));
                self.progress.set(0.0);
            }
            BackendEvent::Event(WebViewEvent::Redirect { to, .. }) => {
                self.url.set(Some(to.clone()));
            }
            BackendEvent::Event(WebViewEvent::Loading { progress }) => {
                self.progress.set(f64::from(*progress).clamp(0.0, 1.0));
            }
            BackendEvent::Event(WebViewEvent::Loaded) => {
                self.progress.set(1.0);
            }
            BackendEvent::Event(WebViewEvent::Error(_)) => {}
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn can_go_back(&self) -> Computed<bool> {
        Computed::from(self.can_go_back.clone())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn can_go_forward(&self) -> Computed<bool> {
        Computed::from(self.can_go_forward.clone())
    }

    pub(crate) fn url(&self) -> Computed<Option<Url>> {
        Computed::from(self.url.clone())
    }

    pub(crate) fn title(&self) -> Computed<Str> {
        Computed::from(self.title.clone())
    }

    pub(crate) fn progress(&self) -> Computed<f64> {
        Computed::from(self.progress.clone())
    }
}

/// Deferred binding slot populated when [`WebViewOpen::open_with`](crate::WebViewOpen::open_with)
/// realizes the web view backing a [`WebViewProxy`].
pub struct DeferredProxySlot {
    handle: Rc<OnceCell<AnyWebViewHandle>>,
    state: WebViewPageState,
}

impl DeferredProxySlot {
    pub(crate) fn bind(self, handle: AnyWebViewHandle) -> WebViewPageState {
        let _ = self.handle.set(handle);
        self.state
    }

    pub(crate) fn bind_unavailable(self, handle: AnyWebViewHandle) {
        let _ = self.handle.set(handle);
    }
}

/// Command and page-state handle for a [`WebView`](crate::WebView), passed
/// directly to [`WebViewOpen::proxy_scope`](crate::WebViewOpen::proxy_scope) /
/// [`WebView::proxy_scope`](crate::WebView::proxy_scope) and extracted from
/// the rendering environment by handlers such as `Button::action`.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui_webview::{WebView, WebViewProxy};
///
/// WebView::open("https://waterui.dev").proxy_scope(|page, web| {
///     vstack((
///         hstack((
///             button("←")
///                 .action(|p: WebViewProxy| p.go_back())
///                 .disabled(web.can_go_back().map(|can| !can)),
///             button("→")
///                 .action(|p: WebViewProxy| p.go_forward())
///                 .disabled(web.can_go_forward().map(|can| !can)),
///             text(web.title()),
///             progress(web.progress()),
///         )),
///         page,
///     ))
/// })
/// ```
#[derive(Clone)]
pub struct WebViewProxy {
    handle: Rc<OnceCell<AnyWebViewHandle>>,
    state: WebViewPageState,
    /// Keeps the state watcher alive when constructed directly via
    /// [`WebViewProxy::new`] without an owning [`WebView`](crate::WebView).
    _watcher: Option<Rc<WatcherGuard>>,
}

impl_debug!(WebViewProxy);

impl WebViewProxy {
    /// Constructs a proxy from a handle and subscribes its page-state signals
    /// to events from that handle.
    #[must_use]
    pub fn new(handle: AnyWebViewHandle) -> Self {
        let state = WebViewPageState::new();
        #[cfg(not(target_arch = "wasm32"))]
        state.seed_navigation_state(&handle);
        let watched = state.clone();
        let guard = handle.watch(move |event| watched.apply(&event));
        Self {
            handle: Rc::new(OnceCell::from(handle)),
            state,
            _watcher: Some(Rc::new(guard)),
        }
    }

    pub(crate) fn from_parts(
        handle: AnyWebViewHandle,
        state: WebViewPageState,
        watcher: Rc<WatcherGuard>,
    ) -> Self {
        Self {
            handle: Rc::new(OnceCell::from(handle)),
            state,
            _watcher: Some(watcher),
        }
    }

    pub(crate) fn deferred() -> (Self, DeferredProxySlot) {
        let handle = Rc::new(OnceCell::new());
        let state = WebViewPageState::new();
        let proxy = Self {
            handle: Rc::clone(&handle),
            state: state.clone(),
            _watcher: None,
        };
        (proxy, DeferredProxySlot { handle, state })
    }

    fn resolve(&self) -> Option<&AnyWebViewHandle> {
        let resolved = self.handle.get();
        if resolved.is_none() {
            contract_violation!("WebViewProxy was used before its WebViewOpen was realized");
        }
        resolved
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

    /// Navigates to the specified URL.
    ///
    /// Takes the same input as [`WebView::go_to`](crate::WebView::go_to): a
    /// literal or a [`Url`](waterui_url::Url), never unparsed runtime text.
    pub fn go_to(&self, url: impl IntoUrl) {
        if let Some(handle) = self.resolve() {
            handle.go_to(&url.into_url());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Refreshes the current page.
    pub fn refresh(&self) {
        if let Some(handle) = self.resolve() {
            handle.refresh();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Stops the current loading operation.
    pub fn stop(&self) {
        if let Some(handle) = self.resolve() {
            handle.stop();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Navigates back in the web view's history.
    pub fn go_back(&self) {
        if let Some(handle) = self.resolve() {
            handle.go_back();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Navigates forward in the web view's history.
    pub fn go_forward(&self) {
        if let Some(handle) = self.resolve() {
            handle.go_forward();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Evaluates the JavaScript expression `script` in the web view and answers
    /// the JSON encoding of its value.
    ///
    /// See [`WebViewHandle::run_javascript`](crate::WebViewHandle::run_javascript)
    /// for the contract every engine meets. The returned future is intentionally thread-local because native
    /// web views are main-thread-affine.
    #[must_use = "the script only runs once the returned future is awaited"]
    pub fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>> {
        let Some(handle) = self.resolve() else {
            return Box::pin(ready(Err(Str::from_static(
                "WebViewProxy was used before its WebViewOpen was realized",
            ))));
        };
        Box::pin(handle.run_javascript(script))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets a cookie in the scoped web view.
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        if let Some(handle) = self.resolve() {
            handle.set_cookie(cookie);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Retrieves current cookies without blocking the UI thread.
    ///
    /// # Errors
    ///
    /// As documented on [`WebViewHandle::get_cookies`](crate::WebViewHandle::get_cookies).
    #[expect(
        clippy::future_not_send,
        reason = "native web views and cookie stores are main-thread-affine"
    )]
    pub async fn get_cookies(&self) -> Result<Vec<Cookie<'static>>, Error> {
        let Some(handle) = self.resolve() else {
            return Ok(Vec::new());
        };
        handle.get_cookies().await
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        if let Some(handle) = self.resolve() {
            handle.set_user_agent(user_agent);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Injects a script that will run on every later page load.
    ///
    /// `key` names the script; injecting again under the same key replaces it.
    ///
    /// # Contract violation
    ///
    /// When `key` names a script the view was opened with
    /// ([`WebViewOpen::inject`](crate::WebViewOpen::inject)): those are fixed
    /// for its life, on every backend alike. A debug build panics, a release
    /// build logs it and ignores the call.
    pub fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        if let Some(handle) = self.resolve() {
            handle.inject_script(key, script, time);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Enables or disables redirect following.
    pub fn set_redirects_enabled(&self, enabled: impl IntoComputed<bool>) {
        if let Some(handle) = self.resolve() {
            handle.set_redirects_enabled(enabled);
        }
    }
}

impl Extractor for WebViewProxy {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>().cloned().ok_or_else(|| {
            Error::msg(format!(
                "{} not found in environment — wrap the surrounding view in \
                 `WebView::open(url).proxy_scope(|page, web| ...)` to inject it",
                type_name::<Self>()
            ))
        })
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}
