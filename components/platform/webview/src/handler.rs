use std::{any::Any, pin::Pin, rc::Rc};

use cookie::Cookie;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Computed, Signal, impl_debug};
use waterui_str::Str;

use crate::{BackendEvent, WatcherGuard};
use waterui_url::Url;

/// What a handler returns: bytes to resolve the page's promise with, or a
/// message to reject it.
pub type HandlerResult = Result<Vec<u8>, String>;

/// The future a handler produces.
///
/// Boxed and thread-local: handlers run on the UI thread with the web view, and
/// the payload types are `!Send` by design.
pub type HandlerFuture = core::pin::Pin<Box<dyn core::future::Future<Output = HandlerResult>>>;

/// A handler the page can call.
///
/// Asynchronous so a handler can read a file or query a database before
/// answering. Every backend already settles the page's promise through a
/// deferred channel, so this costs them nothing.
pub type ScriptMessageHandler = dyn Fn(&[u8]) -> HandlerFuture + 'static;

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
    fn go_to(&self, url: &Url);

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
    ///     Box::pin(async move { Ok(Vec::new()) })
    /// }));
    /// ```
    fn inject_script(&self, script: &str, time: ScriptInjectionTime);

    /// Adds a handler that can be called from JavaScript.
    ///
    /// The page calls it as `waterui.invoke(name, payload)`, the same way on
    /// every backend.
    ///
    /// The handler receives the payload as bytes and answers with a future, so it
    /// may await before replying. An `Err` rejects the page's promise rather than
    /// resolving it with an error encoded into the success channel.
    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>);

    /// Removes a previously added handler.
    fn remove_handler(&self, name: &str);

    /// Chooses which documents may reach the bridge.
    ///
    /// A backend enforces this as natively as it can — restricting where the
    /// bridge script is injected, and checking the origin of the frame a call
    /// arrives from — and refuses calls that do not match.
    fn set_bridge_origins(&self, policy: crate::OriginPolicy);

    /// Stops the current loading operation.
    fn stop(&self);
    /// Refreshes the current page.
    fn refresh(&self);
    /// Sets the user agent string for the web view.
    fn set_user_agent(&self, user_agent: &str);

    /// Enables or disables following redirects.
    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>);
    /// Watches for web view events.
    ///
    /// Multiple watchers can be active at the same time and are invoked in
    /// registration order. Dropping the returned guard unregisters the watcher;
    /// backends get that bookkeeping from [`WatcherSet`](crate::WatcherSet)
    /// rather than implementing it each.
    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard;

    /// Returns whether the web view can navigate back in its history.
    fn can_go_back(&self) -> bool;

    /// Returns whether the web view can navigate forward in its history.
    fn can_go_forward(&self) -> bool;

    /// Sets a cookie for the web view.
    fn set_cookie(&self, cookie: Cookie<'static>);

    /// Retrieves all cookies for the current web view.
    ///
    /// # Attributes are not available everywhere
    ///
    /// Most backends return each cookie with its attributes — domain, path,
    /// expiry, `Secure`, `HttpOnly`, `SameSite`. **Android does not.**
    /// `android.webkit.CookieManager` exposes only
    /// [`getCookie(url)`][getCookie], which returns the request-header form
    /// (`name=value; name2=value2`), and the platform offers no other way to
    /// enumerate the store. On Android every returned cookie therefore carries
    /// a name and a value and nothing else, and code that reads `domain()`,
    /// `expires()` or `same_site()` will see `None` there even when the cookie
    /// does have those attributes.
    ///
    /// Remembering the attributes of cookies set through
    /// [`set_cookie`](Self::set_cookie) would cover only our own cookies and
    /// not the page's, so the two kinds would come back indistinguishable —
    /// which is worse than the gap being visible.
    ///
    /// [getCookie]: https://developer.android.com/reference/android/webkit/CookieManager#getCookie(java.lang.String)
    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>>;

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
    fn go_to(&self, url: &Url);
    fn inject_script(&self, script: &str, time: ScriptInjectionTime);
    fn watch(&self, f: Box<dyn Fn(BackendEvent) + 'static>) -> WatcherGuard;
    fn set_user_agent(&self, user_agent: &str);
    fn set_redirects_enabled(&self, enabled: Computed<bool>);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>);
    fn remove_handler(&self, name: &str);
    fn set_bridge_origins(&self, policy: crate::OriginPolicy);
    fn set_cookie(&self, cookie: Cookie<'static>);
    fn get_cookies<'a>(&'a self) -> Pin<Box<dyn 'a + Future<Output = Vec<Cookie<'static>>>>>;
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

    fn go_to(&self, url: &Url) {
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

    fn watch(&self, f: Box<dyn Fn(BackendEvent) + 'static>) -> WatcherGuard {
        WebViewHandle::watch(self, f)
    }

    fn set_user_agent(&self, user_agent: &str) {
        WebViewHandle::set_user_agent(self, user_agent);
    }

    fn set_redirects_enabled(&self, enabled: Computed<bool>) {
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

    fn set_bridge_origins(&self, policy: crate::OriginPolicy) {
        WebViewHandle::set_bridge_origins(self, policy);
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        WebViewHandle::set_cookie(self, cookie);
    }

    fn get_cookies<'a>(&'a self) -> Pin<Box<dyn 'a + Future<Output = Vec<Cookie<'static>>>>> {
        Box::pin(WebViewHandle::get_cookies(self))
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
    pub fn go_to(&self, url: &Url) {
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

    /// Watches for web view events. Dropping the guard unregisters the watcher.
    pub fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.inner.watch(Box::new(f))
    }

    /// Sets the user agent string for the web view.
    pub fn set_user_agent(&self, user_agent: &str) {
        self.inner.set_user_agent(user_agent);
    }

    /// Enables or disables following redirects.
    pub fn set_redirects_enabled(&self, enabled: impl IntoComputed<bool>) {
        self.inner.set_redirects_enabled(enabled.into_computed());
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

    /// Chooses which documents may reach the bridge.
    pub fn set_bridge_origins(&self, policy: crate::OriginPolicy) {
        self.inner.set_bridge_origins(policy);
    }

    /// Sets a cookie for the web view.
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        self.inner.set_cookie(cookie);
    }

    /// Retrieves all cookies for the current web view.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and cookie stores are main-thread-affine"
    )]
    pub fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> + '_ {
        self.inner.get_cookies()
    }

    /// Runs the given JavaScript code in the context of the web view.
    ///
    /// The returned future is intentionally thread-local because native web views and
    /// their handles are main-thread-affine.
    #[expect(
        clippy::future_not_send,
        reason = "native web views and JavaScript execution are main-thread-affine"
    )]
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
        // SAFETY: the caller contract requires the erased handler to be a `T`; the
        // borrow is tied to `&self`.
        unsafe { &*std::ptr::from_ref::<dyn Any>(self.inner.as_ref()).cast::<T>() }
    }
}
