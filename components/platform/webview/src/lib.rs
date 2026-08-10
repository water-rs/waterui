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
mod handler;
pub use handler::*;
mod proxy;
pub use proxy::WebViewProxy;

// Re-export waterui-internal types for FFI layer
pub use waterui_url::{IntoUrl, Url};
mod url_signal;
pub use url_signal::IntoUrlSignal;

use waterui_core::{
    Binding, Computed, Environment, Signal, View, binding,
    env::use_env,
    layout::StretchAxis,
    raw_view,
    reactive::{signal::IntoComputed, watcher::BoxWatcherGuard},
};
use waterui_str::Str;

/// Events emitted by the `WebView` component.
#[derive(Debug, Clone)]
pub enum WebViewEvent {
    /// No event (initial state).
    None,
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
    /// Navigation state changed (can_go_back/can_go_forward updated).
    ///
    /// This is an internal event used to update reactive state.
    /// It is filtered out from the public `event()` signal.
    #[doc(hidden)]
    StateChanged {
        /// Whether the web view can navigate back.
        can_go_back: bool,
        /// Whether the web view can navigate forward.
        can_go_forward: bool,
    },
}

/// Errors that can occur in the `WebView` component.
#[derive(Debug, thiserror::Error, Clone)]
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
    event: Binding<WebViewEvent>,
    handle: AnyWebViewHandle,
    can_go_back: Binding<bool>,
    can_go_forward: Binding<bool>,
    navigation: Option<Rc<(Computed<Url>, BoxWatcherGuard)>>,
}

impl Clone for WebView {
    fn clone(&self) -> Self {
        Self {
            event: self.event.clone(),
            handle: self.handle.clone(),
            can_go_back: self.can_go_back.clone(),
            can_go_forward: self.can_go_forward.clone(),
            navigation: self.navigation.clone(),
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
            .field("has_reactive_navigation", &self.navigation.is_some())
            .finish_non_exhaustive()
    }
}

impl WebView {
    /// Creates a new `WebView` component with the given handle.
    #[must_use]
    pub(crate) fn from_handle(handle: AnyWebViewHandle) -> Self {
        let event = binding(WebViewEvent::None);
        let can_go_back = binding(handle.can_go_back());
        let can_go_forward = binding(handle.can_go_forward());

        // Set up event handler to update reactive state
        handle.watch({
            let event = event.clone();
            let can_go_back = can_go_back.clone();
            let can_go_forward = can_go_forward.clone();
            move |e| {
                // Handle StateChanged internally without exposing to users
                if let WebViewEvent::StateChanged {
                    can_go_back: back,
                    can_go_forward: forward,
                } = &e
                {
                    can_go_back.set(*back);
                    can_go_forward.set(*forward);
                    // Don't propagate StateChanged to the public event signal
                    return;
                }
                event.set(e);
            }
        });

        Self {
            handle,
            event,
            can_go_back,
            can_go_forward,
            navigation: None,
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
        }
    }

    /// Wraps the [`WebView`] together with `content` and injects a
    /// [`WebViewProxy`] into `content`'s rendering environment so any
    /// child handler can extract it directly.
    ///
    /// The proxy carries the same imperative surface the [`WebView`]
    /// already exposes (`refresh`, `go_back`, `run_javascript`, etc.), but
    /// scoped to whatever handler asks for it via the same
    /// `Extractor`-style parameter pattern that powers
    /// `Button::action`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use waterui::prelude::*;
    /// use waterui_webview::{WebView, WebViewProxy};
    ///
    /// WebView::open("https://waterui.dev").with_proxy(|| {
    ///     hstack((
    ///         button("←").action(|p: WebViewProxy| p.go_back()),
    ///         button("→").action(|p: WebViewProxy| p.go_forward()),
    ///         button("⟳").action(|p: WebViewProxy| p.refresh()),
    ///     ))
    /// })
    /// ```
    pub fn with_proxy<V, F>(self, content: F) -> impl View
    where
        V: View,
        F: FnOnce() -> V + 'static,
    {
        use waterui_core::env::with;
        use waterui_layout::stack::vstack;
        let proxy = WebViewProxy::new(self.handle.clone());
        let body = content();
        // Children render above the WebView body and have the proxy injected
        // into their environment via `with(...)`. The WebView itself follows
        // unchanged.
        vstack((with(body, proxy), self))
    }

    /// Returns a signal that emits `WebView` events.
    #[must_use]
    pub fn event(&self) -> impl Signal<Output = WebViewEvent> {
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

    fn bind_navigation(mut self, url: Computed<Url>) -> Self {
        let handle = self.handle.clone();
        let guard = subscribe_navigation(&url, move |url| handle.go_to(&url));
        self.navigation = Some(Rc::new((url, guard)));
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
    pub fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        self.handle.inject_script(script, time);
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

/// A deferred web view created by [`WebView::open`].
///
/// The native handle is created when this view is rendered, after its
/// [`WebViewController`] can be extracted from the live environment. Keeping
/// the builder concrete makes configuration and [`WebViewOpen::with_proxy`]
/// chainable without creating a web view ahead of rendering. Its URL signal is
/// retained for the native view's lifetime and drives navigation precisely.
#[must_use = "a WebViewOpen must be rendered to create its native web view"]
pub struct WebViewOpen {
    url: Computed<Url>,
    redirects_enabled: Option<Computed<bool>>,
}

impl fmt::Debug for WebViewOpen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebViewOpen")
            .field("url", &self.url)
            .field("redirects_enabled", &self.redirects_enabled)
            .finish()
    }
}

impl WebViewOpen {
    /// Sets the reactive redirect policy applied when the web view is created.
    pub fn redirects_enabled(mut self, enabled: impl IntoComputed<bool>) -> Self {
        self.redirects_enabled = Some(enabled.into_computed());
        self
    }

    /// Injects a proxy for the same deferred web view into `content`.
    ///
    /// The content and web view are created together at render time, so every
    /// extracted [`WebViewProxy`] targets the handle displayed by this view.
    pub fn with_proxy<V, F>(self, content: F) -> impl View
    where
        V: View,
        F: FnOnce() -> V + 'static,
    {
        use_env(move |controller: WebViewController| self.create(&controller).with_proxy(content))
    }

    fn create(self, controller: &WebViewController) -> WebView {
        let Self {
            url,
            redirects_enabled,
        } = self;
        let webview = controller.open();
        if let Some(enabled) = redirects_enabled {
            webview.set_redirects_enabled(enabled);
        }
        webview.bind_navigation(url)
    }
}

impl View for WebViewOpen {
    fn body(self, _env: &Environment) -> impl View {
        use_env(move |controller: WebViewController| self.create(&controller))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

// WebView is a raw view - native backends render it directly
raw_view!(WebView, StretchAxis::Both);

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::subscribe_navigation;
    use waterui_core::{Binding, Signal, reactive::watcher::Context};
    use waterui_url::Url;

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
