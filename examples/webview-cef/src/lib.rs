//! Standard `WebView` running with the bundled CEF engine.
//!
//! The view and controls are shared with the system-WebView example. Only
//! `Water.toml` selects CEF, proving that the standard semantic component does
//! not change when the browser engine changes.

use waterui::Environment;
use waterui::app::App;

/// Creates the standard WebView example with the backend-selected CEF engine.
pub fn app(env: Environment) -> App {
    webview_example::app(env)
}
