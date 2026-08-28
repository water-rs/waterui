use std::{any::Any, pin::Pin, rc::Rc};

use cookie::Cookie;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Computed, Signal, impl_debug};
use waterui_str::Str;

use crate::{BackendEvent, WatcherGuard};
use waterui_url::Url;

/// What a handler returns: the answer to resolve the page's promise with, or a
/// message to reject it.
pub type HandlerResult = Result<crate::message::JsReply, String>;

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
/// A type-erased handle to control and interact with a web view component.
#[derive(Clone)]
pub struct AnyWebViewHandle {
    inner: Rc<dyn WebViewHandleImpl>,
}

webview_handle! {
    /// A handle to control and interact with a web view component.
    ///
    /// This is a pure imperative API - native backends implement this trait.
    /// The `WebView` struct wraps this with nami reactive state.
    pub trait WebViewHandle;
    trait WebViewHandleImpl;
    impl AnyWebViewHandle.inner;

    uniform {
        /// Navigates back in the web view's history.
        fn go_back(&self);
        /// Navigates forward in the web view's history.
        fn go_forward(&self);
        /// Navigates to the specified URL.
        fn go_to(&self, url: &Url);
        /// Stops the current loading operation.
        fn stop(&self);
        /// Refreshes the current page.
        fn refresh(&self);
        /// Sets the user agent string for the web view.
        fn set_user_agent(&self, user_agent: &str);
        /// Returns whether the web view can navigate back in its history.
        fn can_go_back(&self) -> bool;
        /// Returns whether the web view can navigate forward in its history.
        fn can_go_forward(&self) -> bool;

        /// Injects a script that will run on every page load.
        ///
        /// The script is re-injected on every navigation, so a page the view
        /// moves to gets it too.
        ///
        /// `key` identifies the script: injecting again under a key already in
        /// use **replaces** that script rather than adding a second copy. The
        /// mirrored-state seed depends on that — its values are only correct for
        /// the document it was rendered for, so it is re-rendered and replaced
        /// before every navigation, and without replacement a view that
        /// navigated ten times would run ten seed scripts, each staler than the
        /// last.
        fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime);

        /// Adds a handler that can be called from JavaScript.
        ///
        /// The page calls it as `waterui.invoke(name, payload)`, the same way on
        /// every backend.
        ///
        /// The handler receives the payload as bytes and answers with a future,
        /// so it may await before replying. An `Err` rejects the page's promise
        /// rather than resolving it with an error encoded into the success
        /// channel.
        fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>);

        /// Removes a previously added handler.
        fn remove_handler(&self, name: &str);

        /// Chooses which documents may reach the bridge.
        ///
        /// A backend enforces this as natively as it can — restricting where the
        /// bridge script is injected, and checking the origin of the frame a call
        /// arrives from — and refuses calls that do not match.
        fn set_bridge_origins(&self, policy: crate::OriginPolicy);

        /// Sets a cookie for the web view.
        fn set_cookie(&self, cookie: Cookie<'static>);
    }

    // The four below change shape at the boundary: the public form takes or
    // returns an `impl Trait` that a `dyn` cannot hold. Each layer says its own
    // signature so the conversion between them stays where it can be read.
    public extra {
        /// Enables or disables following redirects.
        fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>);

        /// Watches for web view events.
        ///
        /// Multiple watchers can be active at the same time and are invoked in
        /// registration order. Dropping the returned guard unregisters the
        /// watcher; backends get that bookkeeping from
        /// [`WatcherSet`](crate::WatcherSet) rather than implementing it each.
        fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard;

        /// Retrieves all cookies for the current web view.
        ///
        /// # Attributes are not available everywhere
        ///
        /// Most backends return each cookie with its attributes — domain, path,
        /// expiry, `Secure`, `HttpOnly`, `SameSite`. **Android does not.**
        /// `android.webkit.CookieManager` exposes only
        /// [`getCookie(url)`][getCookie], which returns the request-header form
        /// (`name=value; name2=value2`), and the platform offers no other way to
        /// enumerate the store. On Android every returned cookie therefore
        /// carries a name and a value and nothing else, and code that reads
        /// `domain()`, `expires()` or `same_site()` will see `None` there even
        /// when the cookie does have those attributes.
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
        /// This executes the script **after** the page has loaded. For scripts
        /// that need to run before the DOM is constructed (e.g., setting up
        /// bridges), use [`inject_script`](Self::inject_script) with
        /// [`ScriptInjectionTime::DocumentStart`].
        ///
        /// Returns the result of the script execution, or an error message.
        ///
        /// This is the raw path: the value comes back however the engine
        /// marshals it. Everything typed — [`WebView::eval`](crate::WebView::eval),
        /// [`WebView::exec`](crate::WebView::exec) and the mirrored-state
        /// push — goes through [`call_async_javascript`](Self::call_async_javascript)
        /// instead, because it needs the promise awaited.
        fn run_javascript(&self, script: &str) -> impl Future<Output = Result<Str, Str>>;

        /// Runs `body` as the body of an `async` function and **awaits** the
        /// promise it returns.
        ///
        /// This is the entry point every typed evaluation uses, and the
        /// distinction from [`run_javascript`](Self::run_javascript) is not
        /// cosmetic: the shared wrapper in `js/eval.js` is `async`, so the value
        /// of the expression a backend evaluates is always a `Promise`. An
        /// engine API that does not await one — `evaluateJavaScript`,
        /// `webkit_web_view_evaluate_javascript`, `WebView.evaluateJavascript` —
        /// hands back the promise object rather than the JSON envelope, and
        /// every `eval!` / `exec!` fails with a decode error while mirrored
        /// state silently stops reaching the page.
        ///
        /// Engines spell the awaiting form differently — `WebKit`'s
        /// `callAsyncJavaScript`, `WebKitGTK`'s
        /// `webkit_web_view_call_async_javascript_function`, CDP's
        /// `Runtime.evaluate` with `awaitPromise` — which is exactly why it is a
        /// method rather than something the caller assembles. A backend whose
        /// engine has no awaiting API resolves the promise in JavaScript and
        /// reports the result through its own bridge.
        fn call_async_javascript(&self, body: &str) -> impl Future<Output = Result<Str, Str>>;
    }

    shim extra {
        fn set_redirects_enabled(&self, enabled: Computed<bool>);
        fn watch(&self, f: Box<dyn Fn(BackendEvent) + 'static>) -> WatcherGuard;
        fn get_cookies<'a>(&'a self) -> Pin<Box<dyn 'a + Future<Output = Vec<Cookie<'static>>>>>;
        fn run_javascript<'a>(
            &'a self,
            script: &'a str,
        ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>>;
        fn call_async_javascript<'a>(
            &'a self,
            body: &'a str,
        ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>>;
    }

    bridge extra {
        // A `Computed` is a `Signal` and a `Box<dyn Fn>` is an `Fn`, so these
        // two need nothing but the call; only the futures have to be boxed.
        fn set_redirects_enabled(&self, enabled: Computed<bool>) {
            WebViewHandle::set_redirects_enabled(self, enabled);
        }

        fn watch(&self, f: Box<dyn Fn(BackendEvent) + 'static>) -> WatcherGuard {
            WebViewHandle::watch(self, f)
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

        fn call_async_javascript<'a>(
            &'a self,
            body: &'a str,
        ) -> Pin<Box<dyn 'a + Future<Output = Result<Str, Str>>>> {
            Box::pin(WebViewHandle::call_async_javascript(self, body))
        }
    }

    wrapper extra {
        /// Enables or disables following redirects.
        ///
        /// Takes anything that becomes a `Computed`, so a plain `bool` reads as
        /// naturally here as a binding does.
        pub fn set_redirects_enabled(&self, enabled: impl IntoComputed<bool>) {
            self.inner.set_redirects_enabled(enabled.into_computed());
        }

        /// Watches for web view events. Dropping the guard unregisters the
        /// watcher.
        pub fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
            self.inner.watch(Box::new(f))
        }

        /// Retrieves all cookies for the current web view.
        ///
        /// See [`WebViewHandle::get_cookies`] for what a backend can and cannot
        /// report.
        #[expect(
            clippy::future_not_send,
            reason = "native web views and their handles are main-thread-affine"
        )]
        pub fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> + '_ {
            self.inner.get_cookies()
        }

        /// Runs the given JavaScript code in the context of the web view.
        ///
        /// The returned future is intentionally thread-local because native web
        /// views and their handles are main-thread-affine.
        #[expect(
            clippy::future_not_send,
            reason = "native web views and their handles are main-thread-affine"
        )]
        pub fn run_javascript<'a>(
            &'a self,
            script: &'a str,
        ) -> impl Future<Output = Result<Str, Str>> + 'a {
            self.inner.run_javascript(script)
        }

        /// Runs `body` as an `async` function body and awaits its promise.
        ///
        /// See [`WebViewHandle::call_async_javascript`] for why this is separate
        /// from [`run_javascript`](Self::run_javascript).
        #[expect(
            clippy::future_not_send,
            reason = "native web views and their handles are main-thread-affine"
        )]
        pub fn call_async_javascript<'a>(
            &'a self,
            body: &'a str,
        ) -> impl Future<Output = Result<Str, Str>> + 'a {
            self.inner.call_async_javascript(body)
        }

        /// A weak reference to this handle.
        ///
        /// The mirrored-state bridge needs one: its flush closure is stored in a
        /// handler the backend owns, so capturing the handle strongly closes a
        /// cycle through the backend and the native web view is never destroyed.
        #[must_use]
        pub fn downgrade(&self) -> WeakWebViewHandle {
            WeakWebViewHandle {
                inner: Rc::downgrade(&self.inner),
            }
        }

        /// Creates a new `AnyWebViewHandle` from a type implementing
        /// `WebViewHandle`.
        #[must_use]
        pub fn new(handle: impl WebViewHandle) -> Self {
            Self {
                inner: Rc::new(handle),
            }
        }
    }
}

/// A weak [`AnyWebViewHandle`], held by anything the backend itself owns.
///
/// A handler lives in the backend's own table, so a closure inside one that
/// captures the handle strongly keeps the handle — and the native web view —
/// alive for as long as the backend holds the handler, which is forever.
#[derive(Clone)]
pub struct WeakWebViewHandle {
    inner: std::rc::Weak<dyn WebViewHandleImpl>,
}

impl_debug!(WeakWebViewHandle);

impl WeakWebViewHandle {
    /// Returns the handle, or `None` once the web view has been dropped.
    #[must_use]
    pub fn upgrade(&self) -> Option<AnyWebViewHandle> {
        self.inner.upgrade().map(|inner| AnyWebViewHandle { inner })
    }
}

impl_debug!(AnyWebViewHandle);

impl AnyWebViewHandle {
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
