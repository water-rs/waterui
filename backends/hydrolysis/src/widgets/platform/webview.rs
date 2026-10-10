//! The `WebView` leaf, as this backend bridges it.
//!
//! Hydrolysis bridges exactly one web engine: the platform's own, selected by
//! the `webview-system` feature. On macOS that is `WKWebView`; on the web it is
//! an `<iframe>` (`web`). Every other engine — CEF, WPE `WebKit` — is a
//! crate the *application* links and installs as a `Hook<WebView>` realization,
//! which intercepts the component before it reaches this backend.
//!
//! Both bridges mount through Cherenkov `Hosted` content: an `AppKit` port on
//! macOS, the `<iframe>` element on the web. Other engines must install their
//! own hook; reaching the backend without one is a programmer error rather
//! than a layout slot with no page behind it.

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

use waterui_core::Environment;
use waterui_core::layout::Size as LayoutSize;
use waterui_webview::WebView;

#[cfg(hydrolysis_macos_system_webview)]
mod macos;
#[cfg(hydrolysis_web_system_webview)]
mod web;

#[cfg(hydrolysis_macos_system_webview)]
pub use macos::MacSystemWebViewController;
#[cfg(hydrolysis_web_system_webview)]
pub use web::WebSystemWebViewController;

use crate::renderer::{HydroNativeView, HydroState};

impl HydroNativeView for WebView {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &std::rc::Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        #[cfg(hydrolysis_system_webview)]
        {
            LayoutSize::new(0.0, 0.0)
        }
        #[cfg(not(hydrolysis_system_webview))]
        crate::renderer::unsupported_webview()
    }
}

/// The hosted leaf showing `view`'s `WKWebView`. A page another engine opened
/// reaches the backend only when that engine's `Hook<WebView>` is missing.
#[cfg(hydrolysis_macos_system_webview)]
pub fn hosted(view: &WebView) -> crate::HostedView {
    const FOREIGN: &str = "Hydrolysis hosts only pages the system WKWebView controller opened; \
                           install the custom engine's WebView hook";
    let handle = view
        .handle()
        .downcast_ref::<macos::MacSystemWebViewHandle>()
        .expect(FOREIGN);
    crate::HostedView::new(handle.clone())
}

/// The hosted leaf showing `view`'s `<iframe>`. A page another engine opened
/// reaches the backend only when that engine's `Hook<WebView>` is missing.
#[cfg(hydrolysis_web_system_webview)]
pub fn hosted(view: &WebView) -> crate::HostedView {
    const FOREIGN: &str = "Hydrolysis hosts only pages the web <iframe> controller opened; \
                           install the custom engine's WebView hook";
    let handle = view
        .handle()
        .downcast_ref::<web::WebIframeHandle>()
        .expect(FOREIGN);
    crate::HostedView::new(handle.clone())
}

#[cfg(hydrolysis_macos_system_webview)]
pub fn install_controller(env: &mut Environment) {
    macos::install(env);
}

#[cfg(hydrolysis_web_system_webview)]
pub fn install_controller(env: &mut Environment) {
    web::install(env);
}

// No `install_controller` without a system bridge. A build that bridges no
// platform engine installs no controller, so a `WebView` can only be created in
// it under a controller the application installed itself — an engine crate's
// `install`, which also installs the `Hook<WebView>` that intercepts the
// component before this backend sees it. A `WebView` that still gets here has
// nothing to draw it, and panics.
