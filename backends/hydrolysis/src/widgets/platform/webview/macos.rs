//! The macOS `WKWebView` bridge.
//!
//! # Safety
//!
//! Nearly every `unsafe` here is an Objective-C message send, and they all rest on
//! the same two facts, so the per-site comments state only what is specific to
//! each.
//!
//! * **Receivers are owned and live.** Every object messaged is held in a
//!   `Retained<_>` by this wrapper, or handed to a delegate callback by WebKit
//!   itself, which keeps it alive for the duration of that call.
//! * **Everything runs on the main thread.** `WKWebView` and its collaborators are
//!   `MainThreadOnly`; the wrapper is built with a `MainThreadMarker` and WebKit
//!   dispatches its delegate callbacks on the main thread.
//!
//! What those two facts do not cover — a raw `msg_send!` whose signature is
//! asserted by hand, or a nullable pointer WebKit hands back — is spelled out at
//! the site.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use cookie::time::OffsetDateTime;
use futures::channel::oneshot;
use nami::Signal;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSHTTPCookie, NSObject, NSRect, NSString, NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};
use waterui_core::{Computed, Environment, Str};
use waterui_webview::{
    Cookie, CustomWebViewController, ScriptInjectionTime, Url, WatcherGuard, WatcherSet,
    WebViewController, WebViewError, WebViewEvent, WebViewHandle, bridge,
};

/// Bridges `waterui.invoke` to WebKit's message-handler transport.
///
/// The shared script calls one function; WebKit delivers messages through a named
/// handler object, so this adapts between the two.
const TRANSPORT_SCRIPT: &str = concat!(
    "globalThis.__wateruiSend = function (envelope) {",
    "window.webkit.messageHandlers.__wateruiSend.postMessage(envelope);",
    "};"
);

type JavaScriptHandler = Rc<waterui_webview::ScriptMessageHandler>;

#[derive(Clone)]
struct InjectedScript {
    source: String,
    time: ScriptInjectionTime,
}

struct SharedState {
    watchers: WatcherSet<WebViewEvent>,
    redirects_enabled: RefCell<Computed<bool>>,
    handlers: RefCell<HashMap<String, JavaScriptHandler>>,
    scripts: RefCell<Vec<InjectedScript>>,
    last_navigation_url: RefCell<Option<String>>,
    transport_registered: std::cell::Cell<bool>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            watchers: WatcherSet::new(),
            redirects_enabled: RefCell::new(Computed::new(true)),
            handlers: RefCell::new(HashMap::new()),
            scripts: RefCell::new(Vec::new()),
            last_navigation_url: RefCell::new(None),
            transport_registered: std::cell::Cell::new(false),
        }
    }
}

impl SharedState {
    fn emit(&self, event: WebViewEvent) {
        self.watchers.emit(&event);
    }

    fn emit_navigation_state(&self, web_view: &WKWebView) {
        self.emit(WebViewEvent::StateChanged {
            // SAFETY: main-thread message send to an object this wrapper retains;
            // see the module safety note.
            can_go_back: unsafe { web_view.canGoBack() },
            // SAFETY: main-thread message send to an object this wrapper retains;
            // see the module safety note.
            can_go_forward: unsafe { web_view.canGoForward() },
        });
    }

    fn current_url(web_view: &WKWebView) -> Option<String> {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            web_view
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string())
        }
    }

    fn parse_url(raw: String) -> Url {
        Url::parse(&raw).unwrap_or_else(|| Url::from(raw))
    }
}

struct WebViewDelegateIvars {
    shared: Rc<SharedState>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WuiHydrolysisWebViewDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = WebViewDelegateIvars]
    struct WebViewDelegate;

    unsafe impl NSObjectProtocol for WebViewDelegate {}

    unsafe impl WKNavigationDelegate for WebViewDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        #[allow(non_snake_case)]
        unsafe fn webView_decidePolicyForNavigationAction_decisionHandler(
            &self,
            _web_view: &WKWebView,
            _navigation_action: &WKNavigationAction,
            decision_handler: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            decision_handler.call((WKNavigationActionPolicy::Allow,));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didStartProvisionalNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            if let Some(url) = SharedState::current_url(web_view) {
                self.ivars()
                    .shared
                    .last_navigation_url
                    .replace(Some(url.clone()));
                self.ivars().shared.emit(WebViewEvent::WillNavigate {
                    url: SharedState::parse_url(url),
                });
            }
            self.ivars()
                .shared
                .emit(WebViewEvent::Loading { progress: 0.0 });
            self.ivars().shared.emit_navigation_state(web_view);
        }

        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didReceiveServerRedirectForProvisionalNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            let Some(to) = SharedState::current_url(web_view) else {
                return;
            };
            let from = self
                .ivars()
                .shared
                .last_navigation_url
                .borrow()
                .clone()
                .unwrap_or_else(|| to.clone());
            self.ivars().shared.emit(WebViewEvent::Redirect {
                from: SharedState::parse_url(from),
                to: SharedState::parse_url(to.clone()),
            });
            if self.ivars().shared.redirects_enabled.borrow().get() {
                self.ivars()
                    .shared
                    .last_navigation_url
                    .replace(Some(to.clone()));
                self.ivars().shared.emit(WebViewEvent::WillNavigate {
                    url: SharedState::parse_url(to),
                });
            } else {
                // SAFETY: main-thread message send to an object this wrapper
                // retains; see the module safety note.
                unsafe {
                    web_view.stopLoading();
                }
            }
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didCommitNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            self.ivars().shared.emit(WebViewEvent::Loading {
                // SAFETY: main-thread message send to an object this wrapper
                // retains; see the module safety note.
                progress: unsafe { web_view.estimatedProgress() } as f32,
            });
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFinishNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            self.ivars()
                .shared
                .emit(WebViewEvent::Loading { progress: 1.0 });
            self.ivars().shared.emit(WebViewEvent::Loaded);
            self.ivars().shared.emit_navigation_state(web_view);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFailProvisionalNavigation_withError(
            &self,
            _web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.ivars()
                .shared
                .emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
                    error.localizedDescription().to_string(),
                ))));
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFailNavigation_withError(
            &self,
            _web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.ivars()
                .shared
                .emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
                    error.localizedDescription().to_string(),
                ))));
        }
    }

    unsafe impl WKScriptMessageHandler for WebViewDelegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        #[allow(non_snake_case)]
        unsafe fn userContentController_didReceiveScriptMessage(
            &self,
            _user_content_controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            // SAFETY: main-thread message send to an object this wrapper retains;
            // see the module safety note.
            let body = unsafe { message.body() };
            // Page script reaches this transport directly, so nothing here is fatal.
            let Some(body) = body.downcast_ref::<NSString>() else {
                tracing::warn!("WaterUI bridge received a non-string message body; ignoring");
                return;
            };
            let request = match bridge::Request::parse(&body.to_string()) {
                Ok(request) => request,
                Err(error) => {
                    tracing::warn!(%error, "page script sent a malformed WaterUI bridge request");
                    return;
                }
            };
            // Release the borrow before invoking: a handler may register or remove
            // handlers on the same web view.
            let handler = self
                .ivars()
                .shared
                .handlers
                .borrow()
                .get(&request.name)
                .map(Rc::clone);
            let Some(handler) = handler else {
                tracing::warn!(
                    handler = %request.name,
                    "page script called a WaterUI handler that is not registered"
                );
                let reply = bridge::Reply::failure(&format!(
                    "no WaterUI handler named `{}`",
                    request.name
                ));
                // SAFETY: WebKit sets the source web view on every script
                // message it delivers.
                let web_view = unsafe { message.webView() }
                    .expect("a bridge message must have a source web view");
                let script = NSString::from_str(&reply.resolve_script(request.id));
                // SAFETY: main-thread message send to a retained object; see the
                // module safety note.
                unsafe {
                    web_view.evaluateJavaScript_completionHandler(&script, None);
                }
                return;
            };

            // Handlers are asynchronous, so the promise settles when the future
            // completes rather than when this callback returns.
            let future = handler(&request.payload);
            // SAFETY: WebKit sets the source web view on every script message it
            // delivers, and it is retained for the spawned task.
            let web_view = unsafe { message.webView() }
                .expect("a bridge message must have a source web view");
            executor_core::spawn_local(async move {
                let reply = match future.await {
                    Ok(bytes) => bridge::Reply::Bytes(bytes),
                    Err(message) => bridge::Reply::Failure(message),
                };
                let script = NSString::from_str(&reply.resolve_script(request.id));
                // SAFETY: main-thread message send to a retained object; see the
                // module safety note.
                unsafe {
                    web_view.evaluateJavaScript_completionHandler(&script, None);
                }
            })
            .detach();
        }
    }
);

impl WebViewDelegate {
    fn new(mtm: MainThreadMarker, shared: Rc<SharedState>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WebViewDelegateIvars { shared });
        // SAFETY: `msg_send!` to `super.init` is the designated superclass
        // initializer for a `define_class!` type, and the `->
        // Retained<Self>` signature is the one objc2 expects here.
        unsafe { msg_send![super(this), init] }
    }
}

struct MacSystemWebViewInner {
    web_view: Retained<WKWebView>,
    delegate: Retained<WebViewDelegate>,
    shared: Rc<SharedState>,
}

/// A main-thread WKWebView handle used by Hydrolysis hybrid composition.
#[derive(Clone)]
pub(crate) struct MacSystemWebViewHandle {
    inner: Rc<MacSystemWebViewInner>,
}

impl core::fmt::Debug for MacSystemWebViewHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MacSystemWebViewHandle")
            .finish_non_exhaustive()
    }
}

impl MacSystemWebViewHandle {
    fn new() -> Self {
        let mtm = MainThreadMarker::new()
            .expect("Hydrolysis WKWebView must be created on the macOS main thread");
        let shared = Rc::new(SharedState::default());
        let delegate = WebViewDelegate::new(mtm, Rc::clone(&shared));
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        let web_view = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::ZERO,
                &configuration,
            )
        };
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        let handle = Self {
            inner: Rc::new(MacSystemWebViewInner {
                web_view,
                delegate,
                shared,
            }),
        };
        handle.rebuild_user_scripts();
        handle
    }

    pub(crate) fn native_view(&self) -> Retained<WKWebView> {
        self.inner.web_view.clone()
    }

    fn user_content_controller(&self) -> Retained<WKUserContentController> {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe { self.inner.web_view.configuration().userContentController() }
    }

    fn add_user_script(
        controller: &WKUserContentController,
        source: &str,
        time: ScriptInjectionTime,
    ) {
        let source = NSString::from_str(source);
        let injection_time = match time {
            ScriptInjectionTime::DocumentStart => WKUserScriptInjectionTime::AtDocumentStart,
            ScriptInjectionTime::DocumentEnd => WKUserScriptInjectionTime::AtDocumentEnd,
        };
        let mtm = MainThreadMarker::new()
            .expect("Hydrolysis WKWebView scripts must be installed on the macOS main thread");
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        let script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &source,
                injection_time,
                false,
            )
        };
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            controller.addUserScript(&script);
        }
    }

    /// Registers the single WebKit message handler the bridge transports over.
    ///
    /// One handler serves every WaterUI handler name, so this runs once rather
    /// than per registration.
    fn ensure_transport_registered(&self) {
        if self.inner.shared.transport_registered.replace(true) {
            return;
        }
        let controller = self.user_content_controller();
        let name = NSString::from_str(bridge::SEND_FUNCTION);
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            controller.addScriptMessageHandler_name(
                ProtocolObject::from_ref(&*self.inner.delegate),
                &name,
            );
        }
        self.rebuild_user_scripts();
    }

    fn rebuild_user_scripts(&self) {
        let controller = self.user_content_controller();
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            controller.removeAllUserScripts();
        }
        // Transport first: the shared script calls `__wateruiSend`, so the adapter
        // has to be defined before it. Handlers no longer need a script each --
        // the page reaches all of them through `waterui.invoke`.
        Self::add_user_script(
            &controller,
            TRANSPORT_SCRIPT,
            ScriptInjectionTime::DocumentStart,
        );
        Self::add_user_script(
            &controller,
            waterui_webview::DOCUMENT_START_SCRIPT,
            ScriptInjectionTime::DocumentStart,
        );
        for script in self.inner.shared.scripts.borrow().iter() {
            Self::add_user_script(&controller, &script.source, script.time);
        }
    }

    fn native_cookie(cookie: &Cookie<'static>, web_view: &WKWebView) -> Retained<NSHTTPCookie> {
        let value = cookie.to_string();
        let source_url = SharedState::current_url(web_view).unwrap_or_else(|| {
            let domain = cookie.domain().unwrap_or_else(|| {
                panic!("A cookie set before navigation requires a Domain attribute")
            });
            let domain = domain.trim_start_matches('.');
            let scheme = if cookie.secure() == Some(true) {
                "https"
            } else {
                "http"
            };
            format!("{scheme}://{domain}/")
        });
        let source_url = NSURL::URLWithString(&NSString::from_str(&source_url))
            .expect("Hydrolysis WKWebView cookie source URL must be valid");
        let header_name = NSString::from_str("Set-Cookie");
        let header_value = NSString::from_str(&value);
        let headers = NSDictionary::from_slices(&[&*header_name], &[&*header_value]);
        let cookies = NSHTTPCookie::cookiesWithResponseHeaderFields_forURL(&headers, &source_url);
        assert!(
            cookies.count() == 1,
            "Hydrolysis WKWebView received an invalid Set-Cookie value: {value}"
        );
        cookies.objectAtIndex(0)
    }

    fn cookie_from_native(cookie: &NSHTTPCookie) -> Cookie<'static> {
        let mut builder = Cookie::build((cookie.name().to_string(), cookie.value().to_string()))
            .domain(cookie.domain().to_string())
            .path(cookie.path().to_string())
            .secure(cookie.isSecure())
            .http_only(cookie.isHTTPOnly());
        if let Some(expires) = cookie.expiresDate() {
            let seconds = expires.timeIntervalSince1970();
            if seconds.is_finite() {
                let timestamp = seconds as i64;
                let expires = OffsetDateTime::from_unix_timestamp(timestamp)
                    .expect("Hydrolysis WKWebView cookie expiration must fit OffsetDateTime");
                builder = builder.expires(expires);
            }
        }
        builder.build()
    }
}

impl WebViewHandle for MacSystemWebViewHandle {
    fn go_back(&self) {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner.web_view.goBack();
        }
    }

    fn go_forward(&self) {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner.web_view.goForward();
        }
    }

    fn go_to(&self, url: &Url) {
        let url = NSURL::URLWithString(&NSString::from_str(url.as_str()))
            .unwrap_or_else(|| panic!("WKWebView rejected the URL: {url}"));
        let request = NSURLRequest::requestWithURL(&url);
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner.web_view.loadRequest(&request);
        }
    }

    fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        self.inner.shared.scripts.borrow_mut().push(InjectedScript {
            source: script.to_owned(),
            time,
        });
        self.rebuild_user_scripts();
    }

    fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        assert!(
            !name.is_empty(),
            "Hydrolysis WKWebView handler name must not be empty"
        );
        // Registering the same name twice replaces the handler, matching every
        // other backend.
        self.inner
            .shared
            .handlers
            .borrow_mut()
            .insert(name.to_owned(), Rc::from(handler));
        self.ensure_transport_registered();
    }

    fn remove_handler(&self, name: &str) {
        // Removing a name that was never registered is a no-op, matching every
        // other backend.
        self.inner.shared.handlers.borrow_mut().remove(name);
    }


    fn stop(&self) {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner.web_view.stopLoading();
        }
    }

    fn refresh(&self) {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner.web_view.reload();
        }
    }

    fn set_user_agent(&self, user_agent: &str) {
        let user_agent = (!user_agent.trim().is_empty()).then(|| NSString::from_str(user_agent));
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner
                .web_view
                .setCustomUserAgent(user_agent.as_deref());
        }
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        self.inner
            .shared
            .redirects_enabled
            .replace(Computed::new(enabled));
    }

    fn watch(&self, watcher: impl Fn(WebViewEvent) + 'static) -> WatcherGuard {
        self.inner.shared.watchers.insert(watcher)
    }

    fn can_go_back(&self) -> bool {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe { self.inner.web_view.canGoBack() }
    }

    fn can_go_forward(&self) -> bool {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe { self.inner.web_view.canGoForward() }
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let cookie = Self::native_cookie(&cookie, &self.inner.web_view);
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        let store = unsafe {
            self.inner
                .web_view
                .configuration()
                .websiteDataStore()
                .httpCookieStore()
        };
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            store.setCookie_completionHandler(&cookie, None);
        }
    }

    async fn get_cookies(&self) -> Vec<Cookie<'static>> {
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        let store = unsafe {
            self.inner
                .web_view
                .configuration()
                .websiteDataStore()
                .httpCookieStore()
        };
        let (sender, receiver) = oneshot::channel();
        let sender = RefCell::new(Some(sender));
        let completion = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
            // SAFETY: WebKit hands the completion block a non-null cookie array that
            // lives for the duration of the call.
            let cookies = unsafe { cookies.as_ref() };
            let values = (0..cookies.count())
                .map(|index| {
                    let cookie = cookies.objectAtIndex(index);
                    Self::cookie_from_native(&cookie)
                })
                .collect();
            let sender = sender
                .borrow_mut()
                .take()
                .expect("Hydrolysis WKWebView cookie callback invoked twice");
            let _ = sender.send(values);
        });
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            store.getAllCookies(&completion);
        }
        receiver
            .await
            .expect("Hydrolysis WKWebView cookie query was cancelled")
    }

    async fn run_javascript(&self, script: &str) -> Result<Str, Str> {
        let (sender, receiver) = oneshot::channel();
        let sender = RefCell::new(Some(sender));
        let completion = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            // SAFETY: WebKit passes a nullable `NSError` to the completion block;
            // `as_ref` is the null check.
            let result = if let Some(error) = unsafe { error.as_ref() } {
                Err(Str::from(error.localizedDescription().to_string()))
            // SAFETY: WebKit passes a nullable result to the completion block;
            // `as_ref` is the null check.
            } else if let Some(value) = unsafe { value.as_ref() } {
                // SAFETY: `value` is the non-null result WebKit handed the
                // completion block, and `-description` is defined on every
                // `NSObject`, returning a retained string.
                let description: Retained<NSString> = unsafe { msg_send![value, description] };
                Ok(Str::from(description.to_string()))
            } else {
                Ok(Str::from(""))
            };
            let sender = sender
                .borrow_mut()
                .take()
                .expect("Hydrolysis WKWebView JavaScript callback invoked twice");
            let _ = sender.send(result);
        });
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note.
        unsafe {
            self.inner.web_view.evaluateJavaScript_completionHandler(
                &NSString::from_str(script),
                Some(&completion),
            );
        }
        receiver
            .await
            .expect("Hydrolysis WKWebView JavaScript evaluation was cancelled")
    }
}

#[derive(Debug, Clone, Copy)]
struct MacSystemWebViewController;

impl CustomWebViewController for MacSystemWebViewController {
    fn open(&self) -> impl WebViewHandle {
        MacSystemWebViewHandle::new()
    }
}

pub(crate) fn install(env: &mut Environment) {
    assert!(
        env.get::<WebViewController>().is_none(),
        "Hydrolysis macOS system WebView controller was installed twice"
    );
    env.insert(WebViewController::new(MacSystemWebViewController));
}
