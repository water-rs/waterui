//! Bundled CEF implementation for the standard GTK WebView component.

pub use waterui_browser_cef::CefWebViewHandle as GtkWebViewHandle;
use waterui_core::Environment;

/// Installs the CEF-backed standard WebView controller.
pub fn ensure_webview_controller(env: &mut Environment) {
    let runtime = crate::browser_cef::ensure_runtime(env);
    env.insert(runtime.webview_controller());
}

pub(crate) fn render_webview(handle: &GtkWebViewHandle, env: &Environment) -> gtk4::Widget {
    crate::browser_cef::render_page(handle.page().clone(), env, true)
}
