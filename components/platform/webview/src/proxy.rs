//! Imperative escape hatch for [`WebView`].
//!
//! `WebView`'s default surface is reactive — drive navigation by writing to a
//! URL `Binding`, observe loading state by reading reactive bindings, and
//! handle events with `EventHandler`-style callbacks. For the rest (refresh,
//! stop, history navigation, ad-hoc JavaScript evaluation), use
//! [`WebViewProxy`].
//!
//! [`WebViewProxy`] is obtained inside a `WebView::open(url).with_proxy(|| children)`
//! scope: the proxy is injected into the rendering environment so any
//! handler in `children` can extract it the same way it would extract
//! `State<T>` for [`Button::action`].

use std::any::type_name;
#[cfg(not(target_arch = "wasm32"))]
use std::pin::Pin;

#[cfg(not(target_arch = "wasm32"))]
use suiteki::Str;
use waterui_core::extract::{ExtractionState, Extractor};
#[cfg(not(target_arch = "wasm32"))]
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, Error, impl_debug};

#[cfg(not(target_arch = "wasm32"))]
use crate::Cookie;
use crate::handler::AnyWebViewHandle;
#[cfg(not(target_arch = "wasm32"))]
use crate::handler::ScriptInjectionTime;

/// Imperative command surface for a [`WebView`](crate::WebView), extracted
/// from the rendering environment via the same `Extractor` machinery used
/// by `Button::action` parameters.
///
/// Wrap children in `WebView::open(url).with_proxy(|| {...})` to inject the proxy
/// into the subtree; any handler inside the closure may then take a
/// `WebViewProxy` parameter and call its imperative methods.
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
#[derive(Clone)]
pub struct WebViewProxy {
    handle: AnyWebViewHandle,
}

impl_debug!(WebViewProxy);

impl WebViewProxy {
    /// Constructs a proxy from a typed handle. Native backends and the
    /// `WebView::open(url).with_proxy` scope uses this internally.
    #[must_use]
    pub const fn new(handle: AnyWebViewHandle) -> Self {
        Self { handle }
    }

    /// Navigates to the specified URL.
    ///
    /// Takes the same input as [`WebView::go_to`](crate::WebView::go_to): a
    /// literal or a [`Url`](waterui_url::Url), never unparsed runtime text.
    pub fn go_to(&self, url: impl waterui_url::IntoUrl) {
        self.handle.go_to(&url.into_url());
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
    /// Runs the given JavaScript code in the context of the web view.
    ///
    /// The returned future is intentionally thread-local because native
    /// web views are main-thread-affine.
    #[must_use = "the script only runs once the returned future is awaited"]
    pub fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>> {
        Box::pin(self.handle.run_javascript(script))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets a cookie in the scoped web view.
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        self.handle.set_cookie(cookie);
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
    pub fn get_cookies(&self) -> impl Future<Output = Result<Vec<Cookie<'static>>, Error>> + '_ {
        self.handle.get_cookies()
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        self.handle.set_user_agent(user_agent);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Injects a script that will run on every page load.
    ///
    /// `key` names the script; injecting again under the same key replaces it.
    pub fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        self.handle.inject_script(key, script, time);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Enables or disables redirect following.
    pub fn set_redirects_enabled(&self, enabled: impl IntoComputed<bool>) {
        self.handle.set_redirects_enabled(enabled);
    }

    /// Returns the inner type-erased handle for downcast or low-level access.
    #[must_use]
    pub const fn handle(&self) -> &AnyWebViewHandle {
        &self.handle
    }
}

impl Extractor for WebViewProxy {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>().cloned().ok_or_else(|| {
            Error::msg(format!(
                "{} not found in environment — wrap the surrounding view in \
                 `WebView::open(url).with_proxy(|| ...)` to inject it",
                type_name::<Self>()
            ))
        })
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}
