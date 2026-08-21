use std::rc::Rc;

use waterui_core::{impl_debug, impl_extractor};

use crate::{AnyWebViewHandle, WebView, WebViewHandle};

/// A trait for custom web view controllers.
///
/// Native backends must implement this trait and inject into environment to provide web view functionality.
/// The created web view starts blank - use `go_to(url)` to navigate after creation.
pub trait CustomWebViewController: 'static {
    /// Opens a new blank web view and returns its handle.
    fn open(&self) -> impl WebViewHandle;
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

    /// A controller for a build that has no web engine.
    ///
    /// Its web views accept every configuration a page-backed one does, occupy
    /// their layout slot, publish their accessibility node, report an empty
    /// history and load nothing. Install it when this build genuinely has no
    /// engine — a renderer compiled without a `webview-*` feature, an embedded
    /// target, a headless test — so a portable view tree containing a
    /// [`WebView`](crate::WebView) still runs there.
    ///
    /// Nothing installs this on your behalf. A build that expects an engine and
    /// has none still fails where the web view is created, rather than quietly
    /// showing an empty rectangle.
    ///
    /// ```
    /// use waterui_core::Signal;
    /// use waterui_webview::WebViewController;
    ///
    /// let controller = WebViewController::without_engine();
    /// let webview = controller.open();
    /// assert!(!webview.can_go_back().get());
    /// ```
    #[must_use]
    pub fn without_engine() -> Self {
        Self::new(crate::no_engine::NoEngineController)
    }

    /// Opens a new blank web view.
    ///
    /// The web view starts blank - use `go_to(url)` on the returned view to navigate.
    #[must_use]
    pub fn open(&self) -> WebView {
        WebView::from_handle(self.open_handle())
    }

    /// Opens a new blank web view and returns the underlying handle.
    #[must_use]
    pub(crate) fn open_handle(&self) -> AnyWebViewHandle {
        self.controller.open()
    }
}

trait WebViewControllerImpl: 'static {
    fn open(&self) -> AnyWebViewHandle;
}

impl<T: CustomWebViewController> WebViewControllerImpl for T {
    fn open(&self) -> AnyWebViewHandle {
        AnyWebViewHandle::new(CustomWebViewController::open(self))
    }
}

impl_extractor!(WebViewController);
