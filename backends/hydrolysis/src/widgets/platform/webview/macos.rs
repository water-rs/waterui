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

use std::cell::{Cell, RefCell};
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
    NSArray, NSDictionary, NSError, NSHTTPCookie, NSInteger, NSJSONSerialization,
    NSJSONWritingOptions, NSObject, NSRect, NSString, NSURL,
    NSURLErrorAppTransportSecurityRequiresSecureConnection, NSURLErrorCancelled,
    NSURLErrorCannotConnectToHost, NSURLErrorCannotFindHost, NSURLErrorClientCertificateRejected,
    NSURLErrorClientCertificateRequired, NSURLErrorDNSLookupFailed, NSURLErrorDomain,
    NSURLErrorNetworkConnectionLost, NSURLErrorNotConnectedToInternet,
    NSURLErrorSecureConnectionFailed, NSURLErrorServerCertificateHasBadDate,
    NSURLErrorServerCertificateHasUnknownRoot, NSURLErrorServerCertificateNotYetValid,
    NSURLErrorServerCertificateUntrusted, NSURLErrorTimedOut, NSURLRequest,
};
use objc2_web_kit::{
    WKContentWorld, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationDelegate, WKScriptMessage, WKScriptMessageHandler, WKSecurityOrigin,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration,
};
use waterui_core::{Computed, Environment, Str};
use waterui_webview::{
    BackendEvent, Cookie, CustomWebViewController, OriginPolicy, ScriptInjectionTime, Url,
    WatcherGuard, WatcherSet, WebViewController, WebViewError, WebViewEvent, WebViewHandle, bridge,
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

/// `NSURLErrorDomain` codes that mean the TLS handshake or a certificate was
/// the problem, so the failure is reported as [`WebViewError::Ssl`] rather than
/// being flattened into a generic load failure.
const TLS_ERROR_CODES: [NSInteger; 8] = [
    NSURLErrorSecureConnectionFailed,
    NSURLErrorServerCertificateHasBadDate,
    NSURLErrorServerCertificateUntrusted,
    NSURLErrorServerCertificateHasUnknownRoot,
    NSURLErrorServerCertificateNotYetValid,
    NSURLErrorClientCertificateRejected,
    NSURLErrorClientCertificateRequired,
    NSURLErrorAppTransportSecurityRequiresSecureConnection,
];

/// `NSURLErrorDomain` codes that mean the request never reached the server, so
/// the failure is reported as [`WebViewError::Network`].
const TRANSPORT_ERROR_CODES: [NSInteger; 6] = [
    NSURLErrorTimedOut,
    NSURLErrorCannotFindHost,
    NSURLErrorCannotConnectToHost,
    NSURLErrorNetworkConnectionLost,
    NSURLErrorDNSLookupFailed,
    NSURLErrorNotConnectedToInternet,
];

#[derive(Clone)]
struct InjectedScript {
    /// The key the script was injected under. Re-injecting under a key already
    /// in use replaces that script instead of stacking another copy in front of
    /// it, which is what the mirrored-state seed relies on.
    key: String,
    source: String,
    time: ScriptInjectionTime,
}

struct SharedState {
    watchers: WatcherSet<BackendEvent>,
    redirects_enabled: RefCell<Computed<bool>>,
    handlers: RefCell<HashMap<String, JavaScriptHandler>>,
    /// Keyed, in injection order. A `Vec` rather than a map because the order
    /// scripts run in is part of what was asked for.
    scripts: RefCell<Vec<InjectedScript>>,
    last_navigation_url: RefCell<Option<String>>,
    /// Which documents may reach the bridge; checked on every message.
    bridge_origins: RefCell<Option<OriginPolicy>>,
    /// Whether the document being loaded is one the policy admits.
    ///
    /// `WKUserContentController` has no per-origin injection filter, so the
    /// filter is the injection itself: the scripts are rebuilt for every main
    /// frame navigation and installed only when its destination is admitted.
    bridge_admitted: Cell<bool>,
    transport_registered: Cell<bool>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            watchers: WatcherSet::new(),
            redirects_enabled: RefCell::new(Computed::new(true)),
            handlers: RefCell::new(HashMap::new()),
            scripts: RefCell::new(Vec::new()),
            last_navigation_url: RefCell::new(None),
            bridge_origins: RefCell::new(None),
            bridge_admitted: Cell::new(false),
            transport_registered: Cell::new(false),
        }
    }
}

impl SharedState {
    fn emit(&self, event: impl Into<BackendEvent>) {
        self.watchers.emit(&event.into());
    }

    /// Whether a document at `url` may be given the bridge.
    ///
    /// No policy means no bridge: a handle whose origins were never chosen has
    /// nothing to authenticate a page against, and the seed script carries the
    /// live value of every exposed binding.
    fn admits(&self, url: Option<&str>) -> bool {
        let policy = self.bridge_origins.borrow();
        let (Some(policy), Some(url)) = (policy.as_ref(), url) else {
            return false;
        };
        Url::parse(url).is_some_and(|url| policy.allows(&url))
    }

    /// Re-decides whether the document at `url` gets the bridge, and rebuilds
    /// the injected scripts to match.
    fn admit_document(&self, url: Option<&str>, controller: &WKUserContentController) {
        self.bridge_admitted.set(self.admits(url));
        install_user_scripts(controller, self);
    }

    /// Emits the failure of a navigation, or nothing when it was cancelled.
    ///
    /// A cancellation is not a failure: `stop()` and starting a second
    /// navigation while the first is in flight both report `NSURLErrorCancelled`
    /// here, and reporting those as load errors made every replaced navigation
    /// look broken.
    fn emit_navigation_error(&self, error: &NSError) {
        let is_url_error = {
            // SAFETY: reading an Objective-C string constant the framework owns.
            let domain = unsafe { NSURLErrorDomain };
            *error.domain() == *domain
        };
        let code = error.code();
        if is_url_error && code == NSURLErrorCancelled {
            return;
        }
        let message = Str::from(error.localizedDescription().to_string());
        let error = if !is_url_error {
            WebViewError::LoadFailed(message)
        } else if TLS_ERROR_CODES.contains(&code) {
            WebViewError::Ssl {
                // The URL that failed is the one the navigation started at;
                // `WKWebView::URL` still points at the document being replaced.
                url: Self::parse_url(
                    self.last_navigation_url
                        .borrow()
                        .clone()
                        .unwrap_or_default(),
                ),
                message,
            }
        } else if TRANSPORT_ERROR_CODES.contains(&code) {
            WebViewError::Network(message)
        } else {
            WebViewError::LoadFailed(message)
        };
        self.emit(WebViewEvent::Error(error));
    }

    fn emit_navigation_state(&self, web_view: &WKWebView) {
        self.emit(BackendEvent::NavigationState {
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

    /// Converts a URL `WebKit` reported into the event payload type.
    ///
    /// Parsed through `FromStr` rather than [`Url::parse`], which keeps only web
    /// URLs: a web view legitimately navigates to `about:blank`, and that is not
    /// an error. An absolute string `WKWebView` emits that does not parse at all
    /// is a contract break worth crashing on, matching the `WebKitGTK` and WPE
    /// bridges; the `Url::from(String)` fallback this replaces used to
    /// manufacture a bogus `Url` and hand it to the application instead.
    fn parse_url(raw: String) -> Url {
        raw.parse()
            .unwrap_or_else(|error| panic!("WebKit emitted an invalid URL {raw:?}: {error}"))
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
            web_view: &WKWebView,
            navigation_action: &WKNavigationAction,
            decision_handler: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // Document-start scripts have to be chosen before the load begins,
            // and this is the last callback that runs first. The bridge, its
            // evaluation wrapper and the mirrored-state seed are all injected
            // here or not at all: the seed contains the current value of every
            // exposed `Binding`, so handing it to a page the policy does not
            // admit would publish that state to it.
            //
            // Sub-frame navigations are ignored: the scripts are main-frame
            // only, and letting an iframe decide would let it revoke the
            // main frame's bridge.
            // SAFETY: main-thread message sends to objects WebKit retains for
            // this call; see the module safety note.
            let targets_main_frame = unsafe {
                navigation_action
                    .targetFrame()
                    .is_some_and(|frame| frame.isMainFrame())
            };
            if targets_main_frame {
                // SAFETY: main-thread message sends to objects WebKit retains
                // for this call; see the module safety note.
                let url = unsafe {
                    navigation_action
                        .request()
                        .URL()
                        .and_then(|url| url.absoluteString())
                }
                .map(|url| url.to_string());
                // SAFETY: main-thread message send to the web view WebKit hands
                // this callback; see the module safety note.
                let controller = unsafe { web_view.configuration().userContentController() };
                self.ivars()
                    .shared
                    .admit_document(url.as_deref(), &controller);
            }
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
            self.ivars().shared.emit_navigation_error(error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFailNavigation_withError(
            &self,
            _web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.ivars().shared.emit_navigation_error(error);
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
            // WebKit reports the frame each message came from, so the origin is
            // authenticated by the engine rather than claimed by the page.
            let allowed = {
                let policy = self.ivars().shared.bridge_origins.borrow().clone();
                // SAFETY: main-thread message send to an object WebKit retains for
                // this call; see the module safety note.
                let frame = unsafe { message.frameInfo() };
                // SAFETY: main-thread message sends to objects WebKit retains for
                // this call; see the module safety note.
                let is_main_frame = unsafe { frame.isMainFrame() };
                // SAFETY: main-thread message send to an object WebKit retains for
                // this call; see the module safety note.
                let origin = unsafe { frame.securityOrigin() };
                // SAFETY: main-thread message sends to an object WebKit retains
                // for this call; see the module safety note.
                let origin = unsafe { security_origin_string(&origin) };
                is_main_frame && policy.is_some_and(|policy| policy.allows_origin(&origin))
            };
            if !allowed {
                tracing::warn!(
                    "a document outside the bridge origin policy tried to call a WaterUI handler"
                );
                return;
            }

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
                let reply =
                    bridge::Reply::failure(&format!("no WaterUI handler named `{}`", request.name));
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
            let web_view =
                unsafe { message.webView() }.expect("a bridge message must have a source web view");
            executor_core::spawn_local(async move {
                let reply = match future.await {
                    Ok(reply) => bridge::Reply::from(reply),
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

/// The `scheme://host[:port]` string [`OriginPolicy::allows_origin`] matches.
///
/// `WKSecurityOrigin` reports a scheme's default port as `0`; every other port
/// belongs to the origin. Dropping it refused every call from a development
/// server on `http://localhost:3000`, and admitted `https://app.example:8443`
/// for a policy that named only `https://app.example`.
///
/// # Safety
///
/// `origin` must be a live `WKSecurityOrigin` messaged on the main thread.
unsafe fn security_origin_string(origin: &WKSecurityOrigin) -> String {
    // SAFETY: the caller guarantees a live receiver on the main thread.
    let (protocol, host, port) = unsafe {
        (
            origin.protocol().to_string(),
            origin.host().to_string(),
            origin.port(),
        )
    };
    if port == 0 {
        format!("{protocol}://{host}")
    } else {
        format!("{protocol}://{host}:{port}")
    }
}

fn add_user_script(controller: &WKUserContentController, source: &str, time: ScriptInjectionTime) {
    let source = NSString::from_str(source);
    let injection_time = match time {
        ScriptInjectionTime::DocumentStart => WKUserScriptInjectionTime::AtDocumentStart,
        ScriptInjectionTime::DocumentEnd => WKUserScriptInjectionTime::AtDocumentEnd,
    };
    let mtm = MainThreadMarker::new()
        .expect("Hydrolysis WKWebView scripts must be installed on the macOS main thread");
    // SAFETY: main-thread message send to an object this wrapper retains; see
    // the module safety note. The `forMainFrameOnly` argument is `true`: the
    // bridge is documented as reaching the main frame only, and the message
    // handler refuses sub-frame calls, so injecting into sub-frames would only
    // publish the mirrored state to documents that cannot use it.
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            WKUserScript::alloc(mtm),
            &source,
            injection_time,
            true,
        )
    };
    // SAFETY: main-thread message send to an object this wrapper retains; see
    // the module safety note.
    unsafe {
        controller.addUserScript(&script);
    }
}

/// Installs the document-start scripts for the document currently being loaded.
///
/// Everything is rebuilt from scratch, because `WKUserContentController` can
/// only remove all of its user scripts at once — which is also what makes
/// replacing a keyed script work.
fn install_user_scripts(controller: &WKUserContentController, shared: &SharedState) {
    // SAFETY: main-thread message send to an object this wrapper retains; see
    // the module safety note.
    unsafe {
        controller.removeAllUserScripts();
    }
    if !shared.bridge_admitted.get() {
        // A document outside the policy gets no bridge, no evaluation wrapper
        // and no mirrored-state seed. The seed is the reason this is not merely
        // tidy: it declares the current value of every exposed `Binding`, so
        // injecting it into whatever the view navigated to would hand that page
        // the app's state.
        return;
    }
    // Transport first: the shared script calls `__wateruiSend`, so the adapter
    // has to be defined before it. Handlers no longer need a script each --
    // the page reaches all of them through `waterui.invoke`.
    add_user_script(
        controller,
        TRANSPORT_SCRIPT,
        ScriptInjectionTime::DocumentStart,
    );
    add_user_script(
        controller,
        waterui_webview::DOCUMENT_START_SCRIPT,
        ScriptInjectionTime::DocumentStart,
    );
    for script in shared.scripts.borrow().iter() {
        add_user_script(controller, &script.source, script.time);
    }
}

/// Turns the value WebKit hands a completion block into the text every backend
/// returns for it: a string as itself, anything else as JSON.
///
/// `-description` used to stand in for this, so an object came back in
/// Objective-C property-list syntax (`{a = 1;}`) where every other backend
/// returns JSON (`{"a":1}`).
fn marshal_javascript_result(value: *mut AnyObject) -> Result<Str, Str> {
    // SAFETY: WebKit passes a nullable result to the completion block; `as_ref`
    // is the null check.
    let Some(value) = (unsafe { value.as_ref() }) else {
        // `undefined`: no value at all, which is not the JSON `null`.
        return Ok(Str::from_static(""));
    };
    if let Some(string) = value.downcast_ref::<NSString>() {
        return Ok(Str::from(string.to_string()));
    }
    // SAFETY: `value` is the live result object WebKit handed the completion
    // block, and the serializer only reads it.
    let data = unsafe {
        NSJSONSerialization::dataWithJSONObject_options_error(
            value,
            NSJSONWritingOptions::FragmentsAllowed,
        )
    }
    .map_err(|error| Str::from(error.localizedDescription().to_string()))?;
    String::from_utf8(data.to_vec())
        .map(Str::from)
        .map_err(|error| Str::from(error.to_string()))
}

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
        install_user_scripts(&self.user_content_controller(), &self.inner.shared);
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

    fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        {
            let mut scripts = self.inner.shared.scripts.borrow_mut();
            let entry = InjectedScript {
                key: key.to_owned(),
                source: script.to_owned(),
                time,
            };
            // Replacing in place keeps the script's position, so the order the
            // page runs them in does not change when one is re-rendered.
            match scripts.iter_mut().find(|script| script.key == key) {
                Some(existing) => *existing = entry,
                None => scripts.push(entry),
            }
        }
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

    fn set_bridge_origins(&self, policy: OriginPolicy) {
        self.inner.shared.bridge_origins.replace(Some(policy));
        // The policy decides what is injected as well as what is answered, so
        // the document already loaded is re-judged against the new one.
        let url = SharedState::current_url(&self.inner.web_view);
        self.inner
            .shared
            .admit_document(url.as_deref(), &self.user_content_controller());
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

    fn watch(&self, watcher: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
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
        let completion = Self::javascript_completion(sender);
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

    async fn call_async_javascript(&self, body: &str) -> Result<Str, Str> {
        let mtm = MainThreadMarker::new()
            .expect("Hydrolysis WKWebView JavaScript must be evaluated on the macOS main thread");
        let (sender, receiver) = oneshot::channel();
        let completion = Self::javascript_completion(sender);
        // `callAsyncJavaScript` runs the body as an async function and settles
        // once the promise it returns resolves. `evaluateJavaScript` does not
        // await, so it answered the shared wrapper's promise object with
        // `WKErrorJavaScriptResultTypeUnsupported`, which is what made every
        // `eval`/`exec` and every mirrored-state push fail on this backend.
        //
        // The page world is where the injected scripts live, so it is where
        // `__wateruiEval` is defined.
        // SAFETY: main-thread message send to an object this wrapper retains; see
        // the module safety note. `arguments` is nil because the wrapper carries
        // its own, and a nil frame means the main frame.
        unsafe {
            self.inner
                .web_view
                .callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(body),
                    None,
                    None,
                    &WKContentWorld::pageWorld(mtm),
                    Some(&completion),
                );
        }
        receiver
            .await
            .expect("Hydrolysis WKWebView JavaScript evaluation was cancelled")
    }
}

impl MacSystemWebViewHandle {
    /// The completion block both evaluation paths hand WebKit.
    fn javascript_completion(
        sender: oneshot::Sender<Result<Str, Str>>,
    ) -> RcBlock<dyn Fn(*mut AnyObject, *mut NSError)> {
        let sender = RefCell::new(Some(sender));
        RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            // SAFETY: WebKit passes a nullable `NSError` to the completion block;
            // `as_ref` is the null check.
            let result = if let Some(error) = unsafe { error.as_ref() } {
                Err(Str::from(error.localizedDescription().to_string()))
            } else {
                marshal_javascript_result(value)
            };
            let sender = sender
                .borrow_mut()
                .take()
                .expect("Hydrolysis WKWebView JavaScript callback invoked twice");
            let _ = sender.send(result);
        })
    }
}

/// The controller behind this backend's system web views.
///
/// Public so the real-engine test suite can drive the exact handle
/// [`Self::open`] hands the renderer — a genuine `WKWebView`, not a double —
/// through the shared `WebViewHandle` contract. Applications never need it:
/// the backend installs it as the default controller on macOS.
#[derive(Debug, Clone, Copy)]
pub struct MacSystemWebViewController;

impl CustomWebViewController for MacSystemWebViewController {
    fn open(&self) -> impl WebViewHandle {
        MacSystemWebViewHandle::new()
    }
}

pub(crate) fn install(env: &mut Environment) {
    // The backend supplies the *default* controller, so an application or test
    // that installed its own keeps it. This used to assert instead, which turned
    // a deliberate `WebViewController` in the environment into a crash; the WPE
    // path had the mirror-image bug and silently overwrote one.
    if env.get::<WebViewController>().is_some() {
        return;
    }
    env.insert(WebViewController::new(MacSystemWebViewController));
}
