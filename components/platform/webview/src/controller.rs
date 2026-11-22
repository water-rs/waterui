#[cfg(not(target_arch = "wasm32"))]
use waterui_url::Url;

use crate::{AnyWebViewHandle, WebView, WebViewConfig, WebViewHandle, WebViewOpenError};

/// A backend's factory for opening native or engine-backed web views.
///
/// Backends provide their concrete `WebViewController` implementation as
/// `NativeBackend::WebViewController`. The created web view starts blank —
/// `go_to(url)` navigates after creation.
pub trait WebViewController: 'static {
    /// The concrete handle type returned when a web view opens cleanly.
    type Handle: WebViewHandle;

    /// Opens a new blank web view and returns its typed handle.
    ///
    /// `config` carries the inputs a document may receive before its first
    /// line of script, already resolved: the asset server behind the engine's
    /// local asset origin, registered on a `WKWebViewConfiguration`, a
    /// `WebContext` or a request context; the bridge's admission rules; the
    /// application's scripts and the mirrored-state declarations
    /// ([`WebViewConfig::user_scripts`]). They reach the engine before the
    /// first navigation — CEF hands them to its renderer processes with the
    /// browser itself — and the admission rules stay fixed for the view's
    /// life.
    ///
    /// # Errors
    ///
    /// A [`WebViewOpenError`] when the engine cannot create the view — the
    /// caller is off the thread the engine requires, or the platform refused.
    /// The facade reports it as a contract violation and hands the application
    /// an unavailable [`WebView`] rather than a crash.
    fn open(&self, config: WebViewConfig) -> Result<Self::Handle, WebViewOpenError>;

    #[cfg(not(target_arch = "wasm32"))]
    /// The origin this engine serves bundled assets under, known before any
    /// view exists.
    ///
    /// The value is the engine's own answer, not a guess: `waterui://localhost`
    /// on the `WebKit` family and CEF, `https://waterui.localhost` where only
    /// `https` can be a secure context. `None` when the engine has no
    /// interception facility at all, which
    /// [`WebView::open_assets`](crate::WebView::open_assets) treats as a
    /// configuration error.
    fn asset_origin(&self) -> Option<Url>;
}

/// Opens a blank [`WebView`] through `controller` with default creation config.
///
/// # Contract violation
///
/// When the engine cannot open the view: a debug build panics, a release
/// build logs the failure and returns an unavailable view that ignores every
/// operation.
#[must_use]
pub fn open_webview<C: WebViewController>(controller: &C) -> WebView {
    open_webview_with(controller, WebViewConfig::default())
}

/// Opens a blank [`WebView`] through `controller` honoring `config`'s
/// creation-time inputs.
///
/// # Contract violation
///
/// As [`open_webview`].
#[must_use]
pub fn open_webview_with<C: WebViewController>(controller: &C, config: WebViewConfig) -> WebView {
    WebView::from_handle(AnyWebViewHandle::open(config, |config| {
        controller.open(config)
    }))
}
