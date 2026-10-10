//! The `WebView` leaf, as this backend bridges it.
//!
//! Hydrolysis bridges exactly one web engine: the platform's own, selected by
//! the `webview-system` feature. On macOS that is `WKWebView`, on the web an
//! `<iframe>` (`web`) — both mounted through Cherenkov `Hosted` content — and
//! on Android `android.webkit.WebView`, mounted as a platform-view *instance*
//! inside the host's view tree. Every other engine — CEF, WPE `WebKit` — is a
//! crate the *application* links and installs as a `Hook<WebView>` realization,
//! which intercepts the component before it reaches this backend.
//!
//! Other engines must install their own hook; reaching the backend without one
//! is a programmer error rather than a layout slot with no page behind it.

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
pub use macos::MacSystemWebViewController;

use waterui_core::Environment;
use waterui_core::layout::Size as LayoutSize;
use waterui_webview::WebView;

#[cfg(hydrolysis_web_system_webview)]
mod web;

#[cfg(hydrolysis_web_system_webview)]
pub use web::WebSystemWebViewController;

use crate::renderer::{HydroNativeView, HydroState};

#[cfg(not(hydrolysis_android_system_webview))]
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

/// The Android system bridge measures like `Native<PlatformView>`: the mounted
/// `WebView` has no intrinsic size and fills the proposal its slot assigns.
#[cfg(hydrolysis_android_system_webview)]
impl HydroNativeView for WebView {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &std::rc::Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &std::rc::Rc<dyn crate::engine::WidgetTheme>,
        proposal: waterui_core::layout::ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        crate::renderer::graphics_dimensions_from_proposal(proposal)
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

/// Insert the Android system bridge's controller into the session
/// environment, beside the `PlatformViewSink`.
///
/// `jni_env` is `nativeCreateSession`'s environment: resolving
/// `dev.waterui.hydrolysis.webview.HydrolysisWebView` needs a thread that
/// entered from Java, and `session` is the `HydrolysisSession` object the
/// wrapper registers its instance on.
///
/// # Errors
///
/// `find_class`'s or `new_global_ref`'s JNI error, unchanged — a missing
/// class means the app was built without the `dev.waterui.hydrolysis:webview`
/// module, which the CLI's scaffold check is supposed to have caught.
#[cfg(hydrolysis_android_system_webview)]
pub fn install_controller(
    env: &mut Environment,
    jni_env: &mut jni::JNIEnv<'_>,
    session: jni::objects::GlobalRef,
) -> Result<(), crate::runner::android::jni::JniError> {
    use waterui_webview::WebViewController;

    // The backend supplies the *default* controller, so an application or
    // test that installed its own keeps it — the same shape `macos::install`
    // documents.
    if env.get::<WebViewController>().is_some() {
        return Ok(());
    }
    env.insert(WebViewController::new(AndroidSystemWebViewController::new(
        jni_env, session,
    )?));
    Ok(())
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
