use std::{any::Any, pin::Pin, rc::Rc};

use waterui_core::impl_debug;

use crate::WebViewEvent;

/// A handle to control and interact with a web view component.
pub trait WebViewHandle {
    /// Navigates back in the web view's history.
    fn go_back(&self);
    /// Navigates forward in the web view's history.
    fn go_forward(&self);
    /// Navigates to the specified URL.
    fn go_to(&self, url: &str);

    /// Stops the current loading operation.
    fn stop(&self);
    /// Refreshes the current page.
    fn refresh(&self);
    /// Sets the user agent string for the web view.
    fn set_user_agent(&self, user_agent: &str);
    /// Watches for web view events.
    ///
    /// Only one watcher can be active at a time; setting a new watcher replaces the previous one.
    fn watch(&self, f: impl Fn(WebViewEvent) + 'static);
    fn run_javascript(&self, script: &str) -> impl Future<Output = Result<String, String>>;
}

trait WebViewHandleImpl: Any {
    fn go_back(&self);
    fn go_forward(&self);
    fn stop(&self);
    fn refresh(&self);
    fn go_to(&self, url: &str);
    fn watch(&self, f: Box<dyn Fn(WebViewEvent) + 'static>);
    fn set_user_agent(&self, user_agent: &str);
    fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<String, String>>>>;
}

#[derive(Clone)]
pub struct AnyWebViewHandle {
    inner: Rc<dyn WebViewHandleImpl>,
}

impl<T: WebViewHandle + 'static> WebViewHandleImpl for T {
    fn go_back(&self) {
        WebViewHandle::go_back(self);
    }

    fn go_to(&self, url: &str) {
        WebViewHandle::go_to(self, url);
    }

    fn go_forward(&self) {
        WebViewHandle::go_forward(self);
    }

    fn stop(&self) {
        WebViewHandle::stop(self);
    }

    fn refresh(&self) {
        WebViewHandle::refresh(self);
    }

    fn watch(&self, f: Box<dyn Fn(WebViewEvent) + 'static>) {
        WebViewHandle::watch(self, f)
    }

    fn set_user_agent(&self, user_agent: &str) {
        WebViewHandle::set_user_agent(self, user_agent);
    }

    fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<String, String>>>> {
        Box::pin(WebViewHandle::run_javascript(self, script))
    }
}

impl_debug!(AnyWebViewHandle);

impl AnyWebViewHandle {
    /// Creates a new `AnyWebViewHandle` from a type implementing `WebViewHandle`.
    pub fn new(handle: impl WebViewHandle + 'static) -> Self {
        Self {
            inner: Rc::new(handle),
        }
    }

    /// Navigates to the specified URL.
    pub fn go_to(&self, url: &str) {
        self.inner.go_to(url);
    }

    /// Navigates to the specified URL.
    pub fn go_back(&self) {
        self.inner.go_back();
    }

    /// Navigates forward in the web view's history.
    pub fn go_forward(&self) {
        self.inner.go_forward();
    }

    /// Watches for web view events.
    pub fn watch(&self, f: impl Fn(WebViewEvent) + 'static) {
        self.inner.watch(Box::new(f));
    }

    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        self.inner.set_user_agent(user_agent);
    }

    /// Stops the current loading operation.
    pub fn stop(&self) {
        self.inner.stop();
    }

    /// Refreshes the current page.
    pub fn refresh(&self) {
        self.inner.refresh();
    }

    /// Sets the user agent string for the web view.
    pub async fn run_javascript(&self, script: &str) -> Result<String, String> {
        self.inner.run_javascript(script).await
    }

    /// Sets the user agent string for the web view.
    pub fn downcast<T: WebViewHandle + 'static>(self) -> Option<T> {
        Rc::downcast::<T>(self.inner as Rc<dyn Any>)
            .ok()
            .map(|rc| Rc::try_unwrap(rc).ok().unwrap())
    }
}
