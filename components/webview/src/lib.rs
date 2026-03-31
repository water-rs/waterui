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
//! webview.go_to("https://example.com");
//!
//! // Use reactive state for UI
//! let can_go_back = webview.can_go_back();  // Computed<bool>
//! ```

mod controller;

pub use controller::*;
use std::fmt;
mod handler;
pub use handler::*;

// Re-export dependencies for FFI layer
pub use cookie;
pub use waterui_url::Url;

use waterui_core::{
    Binding, Computed, Signal, View, binding, env::use_env, layout::StretchAxis, raw_view,
    reactive::watcher::BoxWatcherGuard,
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
    redirects_watcher: Option<std::rc::Rc<BoxWatcherGuard>>,
}

impl Clone for WebView {
    fn clone(&self) -> Self {
        Self {
            event: self.event.clone(),
            handle: self.handle.clone(),
            can_go_back: self.can_go_back.clone(),
            can_go_forward: self.can_go_forward.clone(),
            redirects_watcher: self.redirects_watcher.clone(),
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
            redirects_watcher: None,
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
    pub fn redirects_enabled(mut self, enabled: impl Into<Computed<bool>>) -> Self {
        let enabled = enabled.into();

        // Initial sync
        self.handle.set_redirects_enabled(enabled.get());

        // Watch for changes
        let handle = self.handle.clone();
        let guard = enabled.watch(move |ctx| {
            handle.set_redirects_enabled(ctx.into_value());
        });

        self.redirects_watcher = Some(std::rc::Rc::new(guard));
        self
    }

    /// Opens a new `WebView` and navigates to the specified URL.
    pub fn open(url: impl AsRef<str>) -> impl View {
        let url = url.as_ref().to_string();
        use_env(move |controller: WebViewController| {
            let webview = controller.open();
            webview.go_to(&url);
            webview
        })
    }

    /// Opens a new `WebView`, navigates to the specified URL, and applies a
    /// configuration function.
    pub fn open_then(
        url: impl AsRef<str>,
        f: impl FnOnce(AnyWebViewHandle) + 'static,
    ) -> impl View {
        let url = url.as_ref().to_string();
        use_env(move |controller: WebViewController| {
            let handle = controller.open_handle();
            f(handle.clone());
            let webview = Self::from_handle(handle);
            webview.go_to(&url);
            webview
        })
    }

    /// Returns a signal that emits `WebView` events.
    #[must_use]
    pub fn event(&self) -> impl Signal<Output = WebViewEvent> {
        self.event.clone()
    }

    /// Navigates to the specified URL.
    pub fn go_to(&self, url: &str) {
        self.handle.go_to(url);
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
    #[allow(clippy::future_not_send)]
    pub fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> impl core::future::Future<Output = Result<Str, Str>> + 'a {
        self.handle.run_javascript(script)
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
    /// Unsupported backends may ignore this setting.
    pub fn set_redirects_enabled(&self, enabled: bool) {
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

// WebView is a raw view - native backends render it directly
raw_view!(WebView, StretchAxis::Both);
