use std::rc::Rc;

use waterui_core::impl_extractor;

use crate::{AnyWebViewHandle, WebViewHandle};

pub trait CustomWebViewController {
    fn open(&self, url: &str) -> impl WebViewHandle;
}

#[derive(Clone)]
pub struct WebViewController {
    controller: Rc<dyn WebViewControllerImpl>,
}

impl WebViewController {
    pub fn open(&self) -> AnyWebViewHandle {
        self.controller.open()
    }
}

trait WebViewControllerImpl {
    fn open(&self) -> AnyWebViewHandle;
}

impl_extractor!(WebViewController);
