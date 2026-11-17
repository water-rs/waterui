//! Standard `WebView` running with the bundled CEF engine.
//!
//! The view and controls are shared with the system-`WebView` example. Only the
//! engine this crate links and scopes differs, proving that the standard
//! semantic component does not change when the browser engine changes.

use waterui::app::{App, Windows};
use waterui::{Environment, ViewExt as _};
use waterui_browser_cef::{CefWebView, install};

/// Creates the standard `WebView` example drawn by the bundled CEF engine.
///
/// `install` starts the packaged Chromium runtime and supplies the
/// `WebViewController` that opens CEF pages, and the `CefWebView` hook scoped
/// over the shared example draws them. Nothing in the shared example code
/// mentions an engine.
#[must_use]
pub fn app(mut env: Environment) -> App<impl Windows> {
    install(&mut env);
    App::new(|| webview_example::demo().hooks(CefWebView), env)
}
