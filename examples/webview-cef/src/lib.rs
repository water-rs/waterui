//! Standard `WebView` running with the bundled CEF engine.
//!
//! The view and controls are shared with the system-`WebView` example. Only the
//! engine this crate links and installs differs, proving that the standard
//! semantic component does not change when the browser engine changes.

use waterui::Environment;
use waterui::app::App;

/// Creates the standard `WebView` example drawn by the bundled CEF engine.
///
/// Depending on `waterui-browser-cef` and calling its `install` is the whole
/// selection: it starts the packaged Chromium runtime, supplies the
/// `WebViewController` that opens CEF pages, and registers the realization that
/// draws them. Nothing in the shared example code below mentions an engine.
pub fn app(mut env: Environment) -> App {
    waterui_browser_cef::install(&mut env);
    App::new(webview_example::demo, env)
}
