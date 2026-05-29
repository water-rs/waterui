//! Imperative escape hatch for [`WebView`].
//!
//! `WebView`'s default surface is reactive — drive navigation by writing to a
//! URL `Binding`, observe loading state by reading reactive bindings, and
//! handle events with `EventHandler`-style callbacks. For the rest (refresh,
//! stop, history navigation, ad-hoc JavaScript evaluation), use
//! [`WebViewProxy`].
//!
//! [`WebViewProxy`] is obtained inside a `WebView::with_proxy(|| children)`
//! scope: the proxy is injected into the rendering environment so any
//! handler in `children` can extract it the same way it would extract
//! `State<T>` for [`Button::action`].

use std::any::type_name;
use std::pin::Pin;

use waterui_core::extract::{ExtractionState, Extractor};
use waterui_core::{Environment, Error, impl_debug};
use waterui_str::Str;

use crate::handler::{AnyWebViewHandle, ScriptInjectionTime};

/// Imperative command surface for a [`WebView`](crate::WebView), extracted
/// from the rendering environment via the same `Extractor` machinery used
/// by `Button::action` parameters.
///
/// Wrap children in `WebView::with_proxy(|| {...})` to inject the proxy
/// into the subtree; any handler inside the closure may then take a
/// `WebViewProxy` parameter and call its imperative methods.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui_webview::{WebView, WebViewProxy};
///
/// let url = binding(Url::parse("https://example.com").unwrap());
/// vstack((
///     hstack((
///         button("←").action(|p: WebViewProxy| p.go_back()),
///         button("→").action(|p: WebViewProxy| p.go_forward()),
///         button("⟳").action(|p: WebViewProxy| p.refresh()),
///     )),
///     WebView::new(&url),
/// ))
/// ```
#[derive(Clone)]
pub struct WebViewProxy {
    handle: AnyWebViewHandle,
}

impl_debug!(WebViewProxy);

impl WebViewProxy {
    /// Constructs a proxy from a typed handle. Native backends and the
    /// `WebView::with_proxy` scope use this internally.
    #[must_use]
    pub const fn new(handle: AnyWebViewHandle) -> Self {
        Self { handle }
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

    /// Runs the given JavaScript code in the context of the web view.
    ///
    /// The returned future is intentionally thread-local because native
    /// web views are main-thread-affine.
    #[allow(clippy::future_not_send)]
    #[must_use]
    pub fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>> {
        Box::pin(self.handle.run_javascript(script))
    }

    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        self.handle.set_user_agent(user_agent);
    }

    /// Injects a script that will run on every page load.
    pub fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        self.handle.inject_script(script, time);
    }

    /// Enables or disables redirect following.
    pub fn set_redirects_enabled(&self, enabled: bool) {
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
                 `WebView::with_proxy(|| ...)` to inject it",
                type_name::<Self>()
            ))
        })
    }

    fn extract_from_action(env: &Environment, _state: &mut ExtractionState) -> Result<Self, Error> {
        Self::extract(env)
    }
}
