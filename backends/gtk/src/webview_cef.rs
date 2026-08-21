//! Bundled CEF implementation for the standard GTK WebView component.

pub use waterui_browser_cef::CefWebViewHandle as GtkWebViewHandle;
use waterui_core::Environment;
use waterui_webview::WebViewController;

/// Installs the CEF-backed standard WebView controller.
///
/// A controller already in the environment is left alone, as in every other
/// realization — and checking first also avoids starting a whole CEF runtime for
/// a controller that would immediately be discarded.
pub fn ensure_webview_controller(env: &mut Environment) {
    if env.get::<WebViewController>().is_some() {
        return;
    }
    let runtime = crate::browser_cef::ensure_runtime(env);
    env.insert(runtime.webview_controller());
}

pub(crate) fn render_webview(handle: &GtkWebViewHandle, env: &Environment) -> gtk4::Widget {
    crate::browser_cef::render_page(handle.page().clone(), env, true)
}
