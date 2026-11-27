//! The `WebView` leaf and its compile-time selected engine.
//!
//! Hydrolysis realizes `WebView` through one compile-time engine feature:
//! - `webview-system` for the platform's own engine (`WKWebView` on macOS, an
//!   `<iframe>` on the web, `android.webkit.WebView` on Android),
//! - `webview-cef` for the bundled CEF runtime (`waterui-browser-cef`), or
//! - `webview-wpe` for the bundled WPE `WebKit` runtime (`waterui-browser-wpe`)
//!   on Linux.
//!
//! Enabling more than one engine feature fails to compile. Reaching the backend
//! where no engine is enabled is a programmer error rather than a layout slot
//! with no page behind it.

// Only macOS can act on this: there the feature names a bridge that exists and
// the diagnostic tells the reader what is missing. On the web the feature
// selects the `<iframe>` bridge, and everywhere else the platform has no web
// engine this backend bridges, and the feature selects nothing — which is what
// the module documents above. It has to stay buildable there, because every
// tool that reads a crate whole turns on every feature on Linux: docs.rs,
// `cargo hack`, and the `cargo semver-checks` release-plz runs before it
// publishes. A hard error on those targets makes the crate impossible to
// document and impossible to release, and protects nobody.
#[cfg(all(
    feature = "webview-system",
    target_os = "macos",
    not(hydrolysis_macos_system_webview)
))]
compile_error!(
    "the `webview-system` feature selects the macOS WKWebView bridge, which needs \
     the `winit` feature (WKWebView is composed into the winit window's AppKit \
     view). Enable `winit`, or link a browser engine crate \
     (`waterui-browser-cef`, `waterui-browser-wpe`) in the application instead."
);

#[cfg(hydrolysis_android_system_webview)]
mod android;
#[cfg(any(test, hydrolysis_android_system_webview))]
mod android_protocol;
#[cfg(hydrolysis_macos_system_webview)]
mod macos;

#[cfg(hydrolysis_android_system_webview)]
pub use android::AndroidSystemWebViewController;
#[cfg(hydrolysis_android_system_webview)]
pub use android::AndroidSystemWebViewHandle;
#[cfg(hydrolysis_macos_system_webview)]
pub use macos::{MacSystemWebViewController, MacSystemWebViewHandle};

#[cfg(hydrolysis_web_system_webview)]
mod web;

#[cfg(hydrolysis_web_system_webview)]
pub use web::{WebIframeHandle, WebSystemWebViewController};

/// The compile-time selected [`WebViewController`](waterui_backend_core::webview::WebViewController)
/// for this Hydrolysis build.
#[cfg(hydrolysis_macos_system_webview)]
pub type SelectedWebViewController = MacSystemWebViewController;

/// The compile-time selected [`WebViewController`](waterui_backend_core::webview::WebViewController)
/// for this Hydrolysis build.
#[cfg(hydrolysis_android_system_webview)]
pub type SelectedWebViewController = AndroidSystemWebViewController;

/// The compile-time selected [`WebViewController`](waterui_backend_core::webview::WebViewController)
/// for this Hydrolysis build.
#[cfg(hydrolysis_web_system_webview)]
pub type SelectedWebViewController = WebSystemWebViewController;

/// The compile-time selected [`WebViewController`](waterui_backend_core::webview::WebViewController)
/// for this Hydrolysis build.
#[cfg(hydrolysis_cef_webview)]
pub type SelectedWebViewController = waterui_browser_cef::CefController;

/// The compile-time selected [`WebViewController`](waterui_backend_core::webview::WebViewController)
/// for this Hydrolysis build.
#[cfg(hydrolysis_wpe_webview)]
pub type SelectedWebViewController = waterui_browser_wpe::WpeController;

/// Fallback [`WebViewController`](waterui_backend_core::webview::WebViewController) when
/// `waterui/webview` is unified without a Hydrolysis engine feature.
#[cfg(all(
    waterui_feature = "webview",
    not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
))]
pub type SelectedWebViewController = waterui_backend_core::webview::UnavailableWebViewController;

/// Wraps a opened `WKWebView` handle in a [`HostedView`](crate::HostedView).
#[cfg(hydrolysis_macos_system_webview)]
pub fn hosted_from_handle(handle: &macos::MacSystemWebViewHandle) -> crate::HostedView {
    crate::HostedView::new(handle.clone())
}

/// Wraps an opened `<iframe>` handle in a [`HostedView`](crate::HostedView).
#[cfg(hydrolysis_web_system_webview)]
pub fn hosted_from_handle(handle: &web::WebIframeHandle) -> crate::HostedView {
    crate::HostedView::new(handle.clone())
}
