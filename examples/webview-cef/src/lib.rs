//! Standard `WebView` running with the bundled CEF engine.
//!
//! The view and controls are shared with the system-`WebView` example. Only the
//! engine this crate links and scopes differs, proving that the standard
//! semantic component does not change when the browser engine changes.

use waterui::Environment;
use waterui::app::{App, Windows};
use waterui::window::Window;
use waterui_browser_cef as _;

/// Creates the standard `WebView` example drawn by the bundled CEF engine.
///
/// Linking `waterui-browser-cef` tells the `WaterUI` CLI to select the backend's
/// `webview-cef` feature at compile time. Nothing in the shared example code
/// mentions an engine or installs a hook.
#[must_use]
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", webview_example::demo))
}
