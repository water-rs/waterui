use std::{any::Any, pin::Pin, rc::Rc};

use cookie::Cookie;
use waterui_core::impl_debug;
use waterui_str::Str;

use crate::WebViewEvent;

type ScriptMessageHandler = dyn Fn(&[u8]) -> Vec<u8> + 'static;

/// When to inject a user script into the web view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptInjectionTime {
    /// Inject at the start of document loading, before the DOM is constructed.
    ///
    /// Use this for:
    /// - Setting up JavaScript-to-native bridges
    /// - Modifying global objects
    /// - Intercepting network requests
    #[default]
    DocumentStart,
    /// Inject after the document has finished loading.
    ///
    /// Use this for:
    /// - Manipulating DOM elements
    /// - Adding event listeners to existing elements
    DocumentEnd,
}

/// A handle to control and interact with a web view component.
///
/// This is a pure imperative API - native backends implement this trait.
/// The `WebView` struct wraps this with nami reactive state.
pub trait WebViewHandle: 'static {
    /// Navigates back in the web view's history.
    fn go_back(&self);
    /// Navigates forward in the web view's history.
    fn go_forward(&self);
    /// Navigates to the specified URL.
    fn go_to(&self, url: &str);

    /// Injects a script that will run on every page load.
    ///
    /// Use [`ScriptInjectionTime::DocumentStart`] to run before the DOM is constructed,
    /// which is ideal for setting up JavaScript-to-native bridges.
    ///
    /// # Example: Setting up a native bridge
    ///
    /// ```ignore
    /// // Inject bridge script at document start
    /// handle.inject_script(r#"
    ///     window.myApp = {
    ///         callNative: function(data) {
    ///             window.webkit.messageHandlers.myHandler.postMessage(data);
    ///         }
    ///     };
    /// "#, ScriptInjectionTime::DocumentStart);
    ///
    /// // Register the native handler
    /// handle.add_handler("myHandler", Box::new(|data| {
    ///     // Handle the call from JavaScript
    ///     vec![]
    /// }));
    /// ```
    fn inject_script(&self, script: &str, time: ScriptInjectionTime);

    /// Adds a handler that can be called from JavaScript.
    ///
    /// JavaScript can call the handler using platform-specific APIs:
    /// - **iOS/macOS**: `window.webkit.messageHandlers.<name>.postMessage(data)`
    /// - **Android**: `window.<name>.postMessage(data)`
    ///
    /// The handler receives data as bytes and returns a response as bytes.
    /// Use [`inject_script`](Self::inject_script) to set up a convenient JavaScript API.
    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>);

    /// Removes a previously added handler.
    fn remove_handler(&self, name: &str);

    /// Stops the current loading operation.
    fn stop(&self);
    /// Refreshes the current page.
    fn refresh(&self);
    /// Sets the user agent string for the web view.
    fn set_user_agent(&self, user_agent: &str);

    /// Enables or disables following redirects.
    ///
    /// Redirect handling is backend-specific; unsupported backends may ignore this.
    fn set_redirects_enabled(&self, _enabled: bool) {}
    /// Watches for web view events.
    ///
    /// Multiple watchers can be active at the same time.
    ///
    /// Watchers are invoked in registration order. Backends should ensure a watcher is not
    /// dropped while it may still be called.
    fn watch(&self, f: impl Fn(WebViewEvent) + 'static);

    /// Returns whether the web view can navigate back in its history.
    fn can_go_back(&self) -> bool;

    /// Returns whether the web view can navigate forward in its history.
    fn can_go_forward(&self) -> bool;

    /// Sets a cookie for the web view.
    fn set_cookie(&self, cookie: Cookie<'static>);

    /// Retrieves all cookies for the current web view.
    fn get_cookies(&self) -> Vec<Cookie<'static>>;

    /// Runs JavaScript code in the context of the currently loaded page.
    ///
    /// This executes the script **after** the page has loaded. For scripts that need
    /// to run before the DOM is constructed (e.g., setting up bridges), use
    /// [`inject_script`](Self::inject_script) with [`ScriptInjectionTime::DocumentStart`].
    ///
    /// Returns the result of the script execution, or an error message.
    fn run_javascript(&self, script: &str) -> impl Future<Output = Result<Str, Str>>;
}

trait WebViewHandleImpl: Any {
    fn go_back(&self);
    fn go_forward(&self);
    fn stop(&self);
    fn refresh(&self);
    fn go_to(&self, url: &str);
    fn inject_script(&self, script: &str, time: ScriptInjectionTime);
    fn watch(&self, f: Box<dyn Fn(WebViewEvent) + 'static>);
    fn set_user_agent(&self, user_agent: &str);
    fn set_redirects_enabled(&self, enabled: bool);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>);
    fn remove_handler(&self, name: &str);
    fn set_cookie(&self, cookie: Cookie<'static>);
    fn get_cookies(&self) -> Vec<Cookie<'static>>;
    fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>>;
}

/// A type-erased handle to control and interact with a web view component.
#[derive(Clone)]
pub struct AnyWebViewHandle {
    inner: Rc<dyn WebViewHandleImpl>,
}

impl<T: WebViewHandle> WebViewHandleImpl for T {
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

    fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        WebViewHandle::inject_script(self, script, time);
    }

    fn watch(&self, f: Box<dyn Fn(WebViewEvent) + 'static>) {
        WebViewHandle::watch(self, f);
    }

    fn set_user_agent(&self, user_agent: &str) {
        WebViewHandle::set_user_agent(self, user_agent);
    }

    fn set_redirects_enabled(&self, enabled: bool) {
        WebViewHandle::set_redirects_enabled(self, enabled);
    }

    fn can_go_back(&self) -> bool {
        WebViewHandle::can_go_back(self)
    }

    fn can_go_forward(&self) -> bool {
        WebViewHandle::can_go_forward(self)
    }

    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>) {
        WebViewHandle::add_handler(self, name, handler);
    }

    fn remove_handler(&self, name: &str) {
        WebViewHandle::remove_handler(self, name);
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        WebViewHandle::set_cookie(self, cookie);
    }

    fn get_cookies(&self) -> Vec<Cookie<'static>> {
        WebViewHandle::get_cookies(self)
    }

    fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>> {
        Box::pin(WebViewHandle::run_javascript(self, script))
    }
}

impl_debug!(AnyWebViewHandle);

impl AnyWebViewHandle {
    /// Creates a new `AnyWebViewHandle` from a type implementing `WebViewHandle`.
    #[must_use]
    pub fn new(handle: impl WebViewHandle) -> Self {
        Self {
            inner: Rc::new(handle),
        }
    }

    /// Navigates to the specified URL.
    pub fn go_to(&self, url: &str) {
        self.inner.go_to(url);
    }

    /// Navigates back in the web view's history.
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

    /// Enables or disables following redirects.
    pub fn set_redirects_enabled(&self, enabled: bool) {
        self.inner.set_redirects_enabled(enabled);
    }

    /// Stops the current loading operation.
    pub fn stop(&self) {
        self.inner.stop();
    }

    /// Refreshes the current page.
    pub fn refresh(&self) {
        self.inner.refresh();
    }

    /// Injects a script that will run on every page load.
    ///
    /// See [`WebViewHandle::inject_script`] for details.
    pub fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        self.inner.inject_script(script, time);
    }

    /// Returns whether the web view can navigate back in its history.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.inner.can_go_back()
    }

    /// Returns whether the web view can navigate forward in its history.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.inner.can_go_forward()
    }

    /// Adds a custom handler that can be called from JavaScript.
    pub fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>) {
        self.inner.add_handler(name, handler);
    }

    /// Removes a previously added custom handler.
    pub fn remove_handler(&self, name: &str) {
        self.inner.remove_handler(name);
    }

    /// Sets a cookie for the web view.
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        self.inner.set_cookie(cookie);
    }

    /// Retrieves all cookies for the current web view.
    #[must_use]
    pub fn get_cookies(&self) -> Vec<Cookie<'static>> {
        self.inner.get_cookies()
    }

    /// Runs the given JavaScript code in the context of the web view.
    ///
    /// The returned future is intentionally thread-local because native web views and
    /// their handles are main-thread-affine.
    #[allow(clippy::future_not_send)]
    pub fn run_javascript<'a>(
        &'a self,
        script: &'a str,
    ) -> impl Future<Output = Result<Str, Str>> + 'a {
        self.inner.run_javascript(script)
    }

    /// Downcasts the handle to a concrete type with runtime checks.
    #[must_use]
    pub fn downcast_ref<T: WebViewHandle>(&self) -> Option<&T> {
        (self.inner.as_ref() as &dyn Any).downcast_ref::<T>()
    }

    /// Downcasts the handle to a concrete type without runtime checks.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the handle was created with type `T`.
    /// This is intended for native backends that control both the creation
    /// and retrieval of the handle (e.g., Swift creates `FfiWebViewHandle`
    /// and later retrieves it via FFI).
    #[must_use]
    pub unsafe fn downcast_ref_unchecked<T: WebViewHandle>(&self) -> &T {
        unsafe { &*std::ptr::from_ref::<dyn Any>(self.inner.as_ref()).cast::<T>() }
    }
}
