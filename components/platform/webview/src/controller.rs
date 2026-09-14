use std::rc::Rc;

use waterui_core::{impl_debug, impl_extractor};

use crate::{AnyWebViewHandle, WebView, WebViewConfig, WebViewHandle};

/// A trait for custom web view controllers.
///
/// Native backends must implement this trait and inject into environment to provide web view functionality.
/// The created web view starts blank - use `go_to(url)` to navigate after creation.
pub trait CustomWebViewController: 'static {
    /// Opens a new blank web view and returns its handle.
    ///
    /// `config` carries creation-time inputs the engine can only honor while
    /// the native view is constructed — the asset server behind the engine's
    /// local asset origin is registered on a `WKWebViewConfiguration`, a
    /// `WebContext` or a request context before the first navigation.
    fn open(&self, config: WebViewConfig) -> impl WebViewHandle;
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

    /// Opens a new blank web view and returns the underlying handle.
    #[must_use]
    pub(crate) fn open_handle(&self, config: WebViewConfig) -> AnyWebViewHandle {
        self.controller.open(config)
    }
}

trait WebViewControllerImpl: 'static {
    fn open(&self, config: WebViewConfig) -> AnyWebViewHandle;
}

impl<T: CustomWebViewController> WebViewControllerImpl for T {
    fn open(&self, config: WebViewConfig) -> AnyWebViewHandle {
        AnyWebViewHandle::new(CustomWebViewController::open(self, config))
    }
}

impl_extractor!(WebViewController);
