//! The `WebView` leaf, as this backend bridges it.
//!
//! Hydrolysis bridges exactly one web engine: the platform's own. On macOS that
//! is `WKWebView`, composed into the winit window as a native subview by the
//! `webview-system` feature. Every other engine — CEF, WPE `WebKit` — is a
//! crate the *application* links and installs as a `Hook<WebView>` realization,
//! which intercepts the component before it reaches this backend.
//!
//! A `WebView` that still reaches the backend is a missing realization — a
//! programmer error — and panics at the earliest point it is seen (measure or
//! node build) rather than occupying a layout slot with no page behind it.
//! That includes the macOS bridge today: its record has no native-view layer
//! to present the `WKWebView` through, so it fails at build like every other
//! engine-less path.

// Only macOS can act on this: there the feature names a bridge that exists and
// the diagnostic tells the reader what is missing. Everywhere else the platform
// has no WKWebView to bridge, and the feature selects nothing — which is what
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

use waterui_core::Environment;
use waterui_core::layout::Size as LayoutSize;
use waterui_webview::WebView;

#[cfg(hydrolysis_macos_system_webview)]
mod macos;

#[cfg(hydrolysis_macos_system_webview)]
pub use macos::MacSystemWebViewController;

use crate::renderer::{HydroNativeView, HydroState};

impl HydroNativeView for WebView {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &std::rc::Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        crate::renderer::unsupported_webview()
    }
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    macos::install(env);
}

// No `install_controller` without the macOS bridge. A build that bridges no
// platform engine installs no controller, so a `WebView` can only be created in
// it under a controller the application installed itself — an engine crate's
// `install`, which also installs the `Hook<WebView>` that intercepts the
// component before this backend sees it. A `WebView` that still gets here has
// nothing to draw it, and panics.
