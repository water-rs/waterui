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
pub use macos::MacSystemWebViewController;

#[cfg(any(hydrolysis_macos_system_webview, hydrolysis_web_system_webview))]
use waterui::webview::WebView;
#[cfg(any(
    hydrolysis_macos_system_webview,
    hydrolysis_android_system_webview,
    hydrolysis_web_system_webview,
    hydrolysis_cef_webview,
    hydrolysis_wpe_webview
))]
use waterui_core::Environment;

#[cfg(hydrolysis_web_system_webview)]
mod web;

#[cfg(hydrolysis_web_system_webview)]
pub use web::WebSystemWebViewController;

/// The hosted leaf showing `view`'s `WKWebView`, or `None` when there is no
/// page to host: the controller failed to open the view (already reported by
/// `waterui-webview`), or another engine opened a page that reaches the backend
/// only when no hook for that engine is in scope — reported here.
#[cfg(hydrolysis_macos_system_webview)]
pub fn hosted(view: &WebView) -> Option<crate::HostedView> {
    let handle = view.handle();
    if handle.is_unavailable() {
        return None;
    }
    let Some(handle) = handle.downcast_ref::<macos::MacSystemWebViewHandle>() else {
        waterui_core::contract_violation!(
            "Hydrolysis hosts only pages the system WKWebView controller opened; \
             scope the custom engine's WebView hook over the view; the web view is not drawn"
        );
        return None;
    };
    Some(crate::HostedView::new(handle.clone()))
}

/// The hosted leaf showing `view`'s `<iframe>`, or `None` when there is no
/// page to host: the controller failed to open the view (already reported by
/// `waterui-webview`), or another engine opened a page that reaches the backend
/// only when no hook for that engine is in scope — reported here.
#[cfg(hydrolysis_web_system_webview)]
pub fn hosted(view: &WebView) -> Option<crate::HostedView> {
    let handle = view.handle();
    if handle.is_unavailable() {
        return None;
    }
    let Some(handle) = handle.downcast_ref::<web::WebIframeHandle>() else {
        waterui_core::contract_violation!(
            "Hydrolysis hosts only pages the web <iframe> controller opened; \
             scope the custom engine's WebView hook over the view; the web view is not drawn"
        );
        return None;
    };
    Some(crate::HostedView::new(handle.clone()))
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
/// wrapper registers its instance on — the session's own global, shared
/// rather than duplicated.
///
/// # Errors
///
/// `find_class`'s or `new_global_ref`'s JNI error, unchanged — a missing
/// class means the app was built without the `dev.waterui.hydrolysis:webview`
/// module, which the CLI's scaffold check is supposed to have caught.
#[cfg(hydrolysis_android_system_webview)]
pub fn install_controller(
    env: &mut Environment,
    jni_env: &mut jni::Env<'_>,
    session: std::sync::Arc<jni::objects::Global<jni::objects::JObject<'static>>>,
) -> Result<(), crate::runner::android::jni::JniError> {
    use waterui::webview::WebViewController;

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

#[cfg(hydrolysis_cef_webview)]
pub fn install_controller(env: &mut Environment) {
    if env.get::<waterui::webview::WebViewController>().is_some() {
        return;
    }
    env.insert(waterui_browser_cef::webview_controller());
}

#[cfg(hydrolysis_wpe_webview)]
pub fn install_controller(env: &mut Environment) {
    if env.get::<waterui::webview::WebViewController>().is_some() {
        return;
    }
    env.insert(waterui_browser_wpe::webview_controller());
}

// No `install_controller` when no web engine is enabled. A `WebView` that
// reaches a build with no engine reports a missing realization.
