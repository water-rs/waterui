use std::rc::Rc;

use waterui_core::{impl_debug, impl_extractor};
#[cfg(not(target_arch = "wasm32"))]
use waterui_url::Url;

use crate::{AnyWebViewHandle, WebView, WebViewConfig, WebViewHandle};

/// A trait for custom web view controllers.
///
/// Native backends must implement this trait and inject into environment to provide web view functionality.
/// The created web view starts blank - use `go_to(url)` to navigate after creation.
pub trait CustomWebViewController: 'static {
    /// Opens a new blank web view and returns its handle.
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
    fn open(&self, config: WebViewConfig) -> impl WebViewHandle;

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

/// A controller for managing web view instances.
///
/// This is a factory for creating web views. It is injected into the Environment
/// by native backends during initialization.
#[derive(Clone)]
pub struct WebViewController {
    controller: Rc<dyn WebViewControllerImpl>,
}

impl_debug!(WebViewController);

impl WebViewController {
    /// Creates a new web view controller from a custom implementation.
    pub fn new(controller: impl CustomWebViewController) -> Self {
        Self {
            controller: Rc::new(controller),
        }
    }

    /// Opens a new blank web view.
    ///
    /// The web view starts blank - use `go_to(url)` on the returned view to navigate.
    #[must_use]
    pub fn open(&self) -> WebView {
        self.open_with(WebViewConfig::default())
    }

    /// Opens a new blank web view honoring `config`'s creation-time inputs.
    #[must_use]
    pub fn open_with(&self, config: WebViewConfig) -> WebView {
        WebView::from_handle(self.open_handle(config))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The origin the engine serves bundled assets under; see
    /// [`CustomWebViewController::asset_origin`].
    #[must_use]
    pub fn asset_origin(&self) -> Option<Url> {
        self.controller.asset_origin()
    }

    /// Opens a new blank web view and returns the underlying handle.
    #[must_use]
    pub(crate) fn open_handle(&self, config: WebViewConfig) -> AnyWebViewHandle {
        self.controller.open(config)
    }
}

trait WebViewControllerImpl: 'static {
    fn open(&self, config: WebViewConfig) -> AnyWebViewHandle;
    #[cfg(not(target_arch = "wasm32"))]
    fn asset_origin(&self) -> Option<Url>;
}

impl<T: CustomWebViewController> WebViewControllerImpl for T {
    fn open(&self, config: WebViewConfig) -> AnyWebViewHandle {
        AnyWebViewHandle::open(config, |config| CustomWebViewController::open(self, config))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn asset_origin(&self) -> Option<Url> {
        CustomWebViewController::asset_origin(self)
    }
}

impl_extractor!(WebViewController);
