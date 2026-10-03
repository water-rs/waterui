//! A typed `WKWebView` wrapper.
//!
//! The surface covers construction with a configuration, the navigation and
//! UI delegate object, the script-message handler object, the URL scheme
//! handler object, the cookie store and the JavaScript evaluation entry
//! points. `objc2-web-kit` 0.3 only generates `WKWebView` and its delegate
//! method lists for macOS, so this module declares the class itself
//! (splitting the superclass per platform) and declares the delegate methods
//! on its own `NSObject` subclasses. Every other `WebKit` type the module
//! uses is generated for both platforms.
//!
//! The surface is imperative: delegate decisions and notifications arrive as
//! closure calls on the main thread. No reactive types appear here; any
//! binding or signal watching lives in the consumer.
//!
//! # Safety
//!
//! `unsafe` in this file covers three things: declaring `WKWebView` and the
//! protocol methods the generated bindings only emit for macOS, sending
//! Objective-C messages whose generated signatures are not available
//! (`serverTrust`, `credentialForTrust:`, `allowsInlineMediaPlayback`), and
//! Objective-C to Rust callbacks where the calling convention is fixed by
//! `WebKit`. Every callback is funneled through [`crate::callback::guarded`]
//! so a panic unwinds as an abort instead of crossing the Objective-C
//! boundary.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use block2::RcBlock;
use objc2::DefinedClass;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, extern_class, extern_methods,
    msg_send,
};
use objc2_core_foundation::CFError;
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSHTTPCookie, NSHTTPURLResponse, NSInteger,
    NSJSONSerialization, NSJSONWritingOptions, NSObject, NSRect, NSString, NSURL,
    NSURLAuthenticationChallenge, NSURLCredential, NSURLRequest,
    NSURLSessionAuthChallengeDisposition,
};
use objc2_security::SecTrust;
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationDelegate, WKNavigationResponse, WKNavigationResponsePolicy, WKScriptMessage,
    WKScriptMessageHandler, WKUIDelegate, WKURLSchemeHandler, WKURLSchemeTask, WKUserScript,
    WKUserScriptInjectionTime, WKWebViewConfiguration, WKWindowFeatures,
};

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSResponder, NSView};
#[cfg(target_os = "ios")]
use objc2_ui_kit::{UIResponder, UIView};
#[cfg(target_os = "ios")]
use objc2_web_kit::WKAudiovisualMediaTypes;

#[cfg(target_os = "macos")]
extern_class!(
    /// `WKWebView`, declared here because `objc2-web-kit` only generates it
    /// for macOS.
    ///
    /// # Safety
    ///
    /// `WKWebView` is an `NSView` subclass on macOS.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[name = "WKWebView"]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    /// The shared `WebKit` web view class.
    pub struct WebView;
);

#[cfg(target_os = "ios")]
extern_class!(
    /// `WKWebView`, declared here because `objc2-web-kit` does not generate
    /// it for iOS.
    ///
    /// # Safety
    ///
    /// `WKWebView` is a `UIView` subclass on iOS.
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[name = "WKWebView"]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    /// The shared `WebKit` web view class.
    pub struct WebView;
);

impl WebView {
    extern_methods!(
        /// The configuration the view was created with.
        #[unsafe(method(configuration))]
        #[unsafe(method_family = none)]
        fn configuration(&self) -> Retained<WKWebViewConfiguration>;

        /// Loads `request`, returning the navigation it starts.
        #[unsafe(method(loadRequest:))]
        #[unsafe(method_family = none)]
        fn load_request(&self, request: &NSURLRequest) -> Option<Retained<WKNavigation>>;

        /// The active committed URL.
        #[unsafe(method(URL))]
        #[unsafe(method_family = none)]
        fn url(&self) -> Option<Retained<NSURL>>;

        /// The navigation delegate; weakly held by `WebKit`, so the owner
        /// retains the object it installs.
        #[unsafe(method(setNavigationDelegate:))]
        #[unsafe(method_family = none)]
        unsafe fn set_navigation_delegate(
            &self,
            delegate: Option<&ProtocolObject<dyn WKNavigationDelegate>>,
        );

        /// The UI delegate; weakly held by `WebKit`.
        #[unsafe(method(setUIDelegate:))]
        #[unsafe(method_family = none)]
        unsafe fn set_ui_delegate(&self, delegate: Option<&ProtocolObject<dyn WKUIDelegate>>);

        #[unsafe(method(canGoBack))]
        #[unsafe(method_family = none)]
        fn can_go_back(&self) -> bool;

        #[unsafe(method(canGoForward))]
        #[unsafe(method_family = none)]
        fn can_go_forward(&self) -> bool;

        #[unsafe(method(goBack))]
        #[unsafe(method_family = none)]
        fn go_back(&self) -> Option<Retained<WKNavigation>>;

        #[unsafe(method(goForward))]
        #[unsafe(method_family = none)]
        fn go_forward(&self) -> Option<Retained<WKNavigation>>;

        #[unsafe(method(reload))]
        #[unsafe(method_family = none)]
        fn reload(&self) -> Option<Retained<WKNavigation>>;

        #[unsafe(method(stopLoading))]
        #[unsafe(method_family = none)]
        fn stop_loading(&self);

        /// The page's estimated loading progress, 0.0 through 1.0.
        #[unsafe(method(estimatedProgress))]
        #[unsafe(method_family = none)]
        fn estimated_progress(&self) -> f64;

        /// `customUserAgent`.
        #[unsafe(method(setCustomUserAgent:))]
        #[unsafe(method_family = none)]
        fn set_custom_user_agent(&self, user_agent: Option<&NSString>);

        /// Runs `script` in the default world and reports the result.
        ///
        /// # Safety
        ///
        /// `completion` is copied and invoked by `WebKit` on the main thread.
        #[unsafe(method(evaluateJavaScript:completionHandler:))]
        #[unsafe(method_family = none)]
        unsafe fn evaluate_javascript(
            &self,
            script: &NSString,
            completion: Option<&block2::DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>>,
        );

        /// Calls an async JavaScript function body in `world`.
        ///
        /// # Safety
        ///
        /// `completion` is copied and invoked by `WebKit` on the main thread.
        #[unsafe(method(callAsyncJavaScript:arguments:inFrame:inContentWorld:completionHandler:))]
        #[unsafe(method_family = none)]
        unsafe fn call_async_javascript(
            &self,
            function_body: &NSString,
            arguments: Option<&NSDictionary<NSString, AnyObject>>,
            frame: Option<&WKFrameInfo>,
            world: &WKContentWorld,
            completion: Option<&block2::DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>>,
        );
    );
}

/// A navigation the delegate reports as a policy decision.
#[derive(Debug)]
pub struct ActionInfo {
    /// The request `WebKit` wants to perform.
    pub request: Retained<NSURLRequest>,
    /// What triggered the navigation.
    pub navigation_type: NavigationType,
    /// Whether the action carried a target frame.
    pub has_target_frame: bool,
    /// Whether the target frame is the main frame.
    pub target_is_main: bool,
}

impl ActionInfo {
    /// The request's absolute URL string, when it can be read.
    #[must_use]
    pub fn url_string(&self) -> Option<String> {
        self.request
            .URL()
            .and_then(|url| url.absoluteString())
            .map(|text| text.to_string())
    }
}

/// The `WKNavigationType` bucket the action came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationType {
    /// A link the user activated.
    LinkActivated,
    /// A submitted form.
    FormSubmitted,
    /// A back/forward item traversal.
    BackForward,
    /// A reload.
    Reload,
    /// A resubmitted form.
    FormResubmitted,
    /// Anything else, including programmatic loads.
    Other,
}

/// `allow` or `cancel` for a navigation decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Proceed with the navigation.
    Allow,
    /// Abort it.
    Cancel,
}

/// What the delegate learns about the response to decide on.
#[derive(Debug)]
pub struct ResponseInfo {
    /// Whether the response targets the main frame.
    pub is_main_frame: bool,
    /// The HTTP status code when the response is an `NSHTTPURLResponse`.
    pub status_code: Option<i64>,
    /// The response's absolute URL string.
    pub url: String,
    /// The `Location` header for redirects.
    pub redirect_location: Option<String>,
}

/// The authentication challenge a delegate answers.
#[derive(Debug)]
pub struct ChallengeInfo {
    /// `protectionSpace.authenticationMethod`.
    pub method: String,
    /// The server trust object for `NSURLAuthenticationMethodServerTrust`.
    pub server_trust: Option<Retained<SecTrust>>,
    /// The current document URL, for error reporting.
    pub current_url: String,
}

/// How a challenge is answered.
#[derive(Debug, Clone, Copy)]
pub enum ChallengeDecision {
    /// Present `serverTrust`'s credential — only valid for a server-trust
    /// challenge.
    UseCredential,
    /// Cancel the navigation.
    Cancel,
    /// Let `WebKit` perform its default handling.
    PerformDefault,
}

/// Notifications the delegate emits; decisions are the closures on
/// [`Handlers`], not events.
#[derive(Debug)]
pub enum Event {
    /// `didStartProvisionalNavigation`.
    StartedProvisional,
    /// `didReceiveServerRedirectForProvisionalNavigation`.
    ServerRedirect,
    /// `didFinishNavigation`.
    Finished,
    /// `didFailNavigation` with the error's localized description.
    Failed(String),
    /// `didFailProvisionalNavigation` with the error's localized description.
    ProvisionalFailed(String),
    /// `estimatedProgress` changed.
    Progress(f64),
    /// `createWebViewWith` arrived with no target frame: the caller loads
    /// the request itself.
    OpenRequest(Retained<NSURLRequest>),
}

/// The navigation-action decision closure.
pub type DecideAction = Box<dyn Fn(&WebView, &ActionInfo) -> Policy>;

/// The navigation-response decision closure.
pub type DecideResponse = Box<dyn Fn(&WebView, &ResponseInfo) -> Policy>;

/// The authentication-challenge decision closure.
pub type DecideAuthentication = Box<dyn Fn(&WebView, &ChallengeInfo) -> ChallengeDecision>;

/// The notification closure every delegate event arrives on.
pub type OnEvent = Box<dyn Fn(&WebView, Event)>;

/// The closures a [`WebViewController`] invokes.
///
/// An unset decision closure allows the navigation; an unset authentication
/// closure falls back to `WebKit`'s default handling.
#[derive(Default)]
pub struct Handlers {
    /// `decidePolicyForNavigationAction`.
    pub decide_action: Option<DecideAction>,
    /// `decidePolicyForNavigationResponse`.
    pub decide_response: Option<DecideResponse>,
    /// `didReceiveAuthenticationChallenge`.
    pub authentication: Option<DecideAuthentication>,
    /// Every notification the navigation delegate emits.
    pub event: Option<OnEvent>,
}

impl std::fmt::Debug for Handlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handlers")
            .field("decide_action", &self.decide_action.is_some())
            .field("decide_response", &self.decide_response.is_some())
            .field("authentication", &self.authentication.is_some())
            .field("event", &self.event.is_some())
            .finish()
    }
}

/// A user script's injection time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionTime {
    /// At document start.
    DocumentStart,
    /// At document end.
    DocumentEnd,
}

/// A script message delivered by `webkit.messageHandlers`.
#[derive(Debug)]
pub struct ScriptMessage {
    /// The message body, decoded as a string when `WebKit` hands one over.
    pub body: Option<String>,
    /// Whether the frame that sent it is the main frame.
    pub frame_is_main: bool,
    /// `securityOrigin.protocol`.
    pub frame_scheme: String,
    /// `securityOrigin.host`.
    pub frame_host: String,
    /// `securityOrigin.port`.
    pub frame_port: i64,
}

/// A request a [`SchemeHandler`] answers.
#[derive(Debug)]
pub struct SchemeRequest {
    /// The request method as delivered.
    pub method: String,
    /// The request path.
    pub path: String,
    /// The query string without the leading `?`.
    pub query: Option<String>,
}

/// A response a [`SchemeHandler`] produces.
#[derive(Debug)]
pub struct SchemeResponse {
    /// The HTTP status code.
    pub status: u16,
    /// Header fields.
    pub headers: Vec<(String, String)>,
    /// The body bytes.
    pub body: Vec<u8>,
}

/// A `WKURLSchemeHandler` backed by a closure.
///
/// Only requests whose URL host equals `host` reach the closure; others get
/// a 404. Requests for a different scheme never reach the handler at all.
#[derive(Clone)]
pub struct SchemeHandler {
    /// The URL scheme this handler answers, e.g. `waterui`.
    pub scheme: String,
    /// The host requests must carry, e.g. `localhost`.
    pub host: String,
    /// The responder. `WebKit` may invoke it off the main thread.
    pub respond: Arc<dyn Fn(&SchemeRequest) -> SchemeResponse + Send + Sync>,
}

impl std::fmt::Debug for SchemeHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemeHandler")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// A cookie read back from the web view's cookie store.
#[derive(Debug, Clone)]
pub struct CookieRecord {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// `Domain` attribute.
    pub domain: String,
    /// `Path` attribute.
    pub path: String,
    /// `Expires` as seconds since the Unix epoch.
    pub expires: Option<f64>,
    /// `Secure` flag.
    pub secure: bool,
    /// `HttpOnly` flag.
    pub http_only: bool,
    /// `SameSite` raw attribute value `WebKit` reported.
    pub same_site: Option<String>,
}

/// The error [`WebViewController::set_cookie_header`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieError {
    /// The header was empty after trimming.
    Empty,
    /// The cookie needs a `Domain` attribute and has no current URL to fall
    /// back on.
    MissingDomain,
    /// `HTTPCookie.cookiesWithResponseHeaderFields` rejected the header.
    Malformed,
    /// The `Domain` attribute was not a usable host.
    BadDomain,
}

impl core::fmt::Display for CookieError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::Empty => "cookie value is empty",
            Self::MissingDomain => "cookie is missing a Domain attribute",
            Self::Malformed => "cookie could not be parsed",
            Self::BadDomain => "cookie domain is not a usable host",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for CookieError {}

/// The `WebView`'s `estimatedProgress` key path.
fn estimated_progress_key() -> Retained<NSString> {
    NSString::from_str("estimatedProgress")
}

/// Ivars for the `ProgressObserver` class.
struct ProgressIvars {
    handler: Rc<dyn Fn(f64)>,
}

impl std::fmt::Debug for ProgressIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressIvars").finish()
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUIWebViewProgressObserver"]
    #[ivars = ProgressIvars]
    #[derive(Debug)]
    /// KVO target for `estimatedProgress`.
    struct ProgressObserver;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for ProgressObserver {}

    impl ProgressObserver {
        // SAFETY: overrides `NSObject`'s `observeValueForKeyPath`, whose
        // signature the KVO contract fixes.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary>,
            _context: *mut std::ffi::c_void,
        ) {
            if !key_path.is_some_and(|key| key.isEqualToString(&estimated_progress_key())) {
                return;
            }
            let Some(view) = object.and_then(AnyObject::downcast_ref::<WebView>) else {
                return;
            };
            let progress = view.estimated_progress();
            let handler = self.ivars().handler.clone();
            crate::callback::guarded("web view progress observer", || handler(progress));
        }
    }
);

impl ProgressObserver {
    fn new(mtm: MainThreadMarker, handler: Rc<dyn Fn(f64)>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ProgressIvars { handler });
        // SAFETY: `this` is a live, allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// A progress observer registration; dropping it removes the KVO pair.
#[derive(Debug)]
struct ProgressToken {
    view: Retained<WebView>,
    observer: Retained<ProgressObserver>,
}

impl ProgressToken {
    /// Registers `handler` for the view's `estimatedProgress`.
    fn register(view: &Retained<WebView>, handler: Rc<dyn Fn(f64)>) -> Self {
        use objc2_foundation::{
            NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration,
        };
        let mtm = MainThreadMarker::from(&**view);
        let observer = ProgressObserver::new(mtm, handler);
        // SAFETY: `observer` answers `observeValueForKeyPath`; the token
        // removes the registration on drop while both parties are retained.
        // SAFETY: see the module safety note.
        unsafe {
            view.addObserver_forKeyPath_options_context(
                &observer,
                &estimated_progress_key(),
                NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            );
        }
        Self {
            view: view.clone(),
            observer,
        }
    }
}

impl Drop for ProgressToken {
    fn drop(&mut self) {
        use objc2_foundation::NSObjectNSKeyValueObserverRegistration;
        // SAFETY: paired with the `addObserver` in `register`; both parties
        // are still retained here.
        // SAFETY: see the module safety note.
        unsafe {
            self.view
                .removeObserver_forKeyPath(&self.observer, &estimated_progress_key());
        }
    }
}

/// Ivars for the `ScriptMessageHandler` class.
struct ScriptHandlerIvars {
    handler: Rc<dyn Fn(ScriptMessage)>,
}

impl std::fmt::Debug for ScriptHandlerIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptHandlerIvars").finish()
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUIWebViewScriptHandler"]
    #[ivars = ScriptHandlerIvars]
    #[derive(Debug)]
    /// A `WKScriptMessageHandler` that forwards to a closure.
    struct ScriptMessageHandler;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for ScriptMessageHandler {}

    // SAFETY: `userContentController:didReceiveScriptMessage:` is the whole
    // `WKScriptMessageHandler` contract; the generated trait does not ship
    // it for iOS, but declaring the protocol method directly is valid.
    unsafe impl WKScriptMessageHandler for ScriptMessageHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive_script_message(
            &self,
            _controller: &objc2_web_kit::WKUserContentController,
            message: &WKScriptMessage,
        ) {
            // SAFETY: see the module safety note.
            let body = unsafe { message.body() }
                .downcast_ref::<NSString>()
                .map(ToString::to_string);
            // SAFETY: see the module safety note.
            let frame = unsafe { message.frameInfo() };
            // SAFETY: see the module safety note.
            let security_origin = unsafe { frame.securityOrigin() };
            let delivered = ScriptMessage {
                body,
                // SAFETY: see the module safety note.
                frame_is_main: unsafe { frame.isMainFrame() },
                // SAFETY: see the module safety note.
                frame_scheme: unsafe { security_origin.protocol() }.to_string(),
                // SAFETY: see the module safety note.
                frame_host: unsafe { security_origin.host() }.to_string(),
                // SAFETY: see the module safety note.
                frame_port: unsafe { security_origin.port() } as i64,
            };
            let handler = self.ivars().handler.clone();
            crate::callback::guarded("web view script message handler", || handler(delivered));
        }
    }
);

impl ScriptMessageHandler {
    fn new(mtm: MainThreadMarker, handler: Rc<dyn Fn(ScriptMessage)>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ScriptHandlerIvars { handler });
        // SAFETY: `this` is a live, allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// Ivars for the `SchemeTaskHandler` class.
struct SchemeIvars {
    handler: SchemeHandler,
}

impl std::fmt::Debug for SchemeIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemeIvars")
            .field("scheme", &self.handler.scheme)
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUIWebViewSchemeHandler"]
    #[ivars = SchemeIvars]
    #[derive(Debug)]
    /// A `WKURLSchemeHandler` that forwards to a closure.
    struct SchemeTaskHandler;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for SchemeTaskHandler {}

    // SAFETY: `webView:startURLSchemeTask:` and
    // `webView:stopURLSchemeTask:` are the complete `WKURLSchemeHandler`
    // contract; the generated trait only ships them for macOS, but declaring
    // them directly is valid on iOS too. The callbacks may run on a WebKit
    // background queue, so `SchemeHandler::respond` is `Send + Sync`.
    unsafe impl WKURLSchemeHandler for SchemeTaskHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start_scheme_task(
            &self,
            _web_view: &WebView,
            url_scheme_task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            let handler = self.ivars().handler.clone();
            crate::callback::guarded("web view scheme handler", || {
                respond_to_scheme_task(url_scheme_task, &handler);
            });
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop_scheme_task(
            &self,
            _web_view: &WebView,
            _url_scheme_task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
        }
    }
);

impl SchemeTaskHandler {
    fn new(mtm: MainThreadMarker, handler: SchemeHandler) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SchemeIvars { handler });
        // SAFETY: `this` is a live, allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// Serves one `WKURLSchemeTask` from `handler`.
fn respond_to_scheme_task(task: &ProtocolObject<dyn WKURLSchemeTask>, handler: &SchemeHandler) {
    // SAFETY: see the module safety note.
    let request = unsafe { task.request() };
    let url = request.URL();
    // SAFETY: see the module safety note.
    let method: Option<Retained<NSString>> = unsafe {
        // SAFETY: `request` is a live `NSURLRequest`; `HTTPMethod` is a
        // read-only property the generated bindings only attach to
        // `NSMutableURLRequest`.
        msg_send![&request, HTTPMethod]
    };
    let method = method.map(|method| method.to_string()).unwrap_or_default();

    let served = url
        .as_ref()
        .and_then(|url| url.host())
        .is_some_and(|host| host.to_string() == handler.host);
    let response = if served {
        let request = SchemeRequest {
            method,
            // `percentEncodedPath`/`percentEncodedQuery`: the dispatcher
            // wants the raw URI components, not `path`'s decoded form.
            // `objc2-foundation` does not bind the percent-encoded getters.
            path: url.as_ref().map_or_else(
                || String::from("/"),
                |url| {
                    // SAFETY: documented above — no binding for percentEncodedPath.
                    let path: Option<Retained<NSString>> =
                        unsafe { msg_send![&**url, percentEncodedPath] };
                    path.map_or_else(|| String::from("/"), |path| path.to_string())
                },
            ),
            query: url.as_ref().and_then(|url| {
                // SAFETY: documented above — no binding for percentEncodedQuery.
                let query: Option<Retained<NSString>> =
                    unsafe { msg_send![&**url, percentEncodedQuery] };
                query.map(|query| query.to_string())
            }),
        };
        (handler.respond)(&request)
    } else {
        SchemeResponse {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        }
    };

    let pairs: Vec<(Retained<NSString>, Retained<NSString>)> = response
        .headers
        .iter()
        .map(|(name, value)| (NSString::from_str(name), NSString::from_str(value)))
        .collect();
    let names: Vec<&NSString> = pairs.iter().map(|(name, _)| &**name).collect();
    let values: Vec<&NSString> = pairs.iter().map(|(_, value)| &**value).collect();
    let header_fields =
        NSDictionary::<NSString, NSString>::from_slices(names.as_slice(), values.as_slice());

    let Some(url) = url else {
        return;
    };
    let response_object = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        &url,
        NSInteger::try_from(response.status).unwrap_or_default(),
        Some(&NSString::from_str("HTTP/1.1")),
        Some(&header_fields),
    );
    let Some(response_object) = response_object else {
        return;
    };

    let body = NSData::from_vec(response.body);
    // SAFETY: the task is live for the duration of `startURLSchemeTask`,
    // and the response/data objects are retained locals.
    // SAFETY: see the module safety note.
    unsafe {
        task.didReceiveResponse(&response_object);
        if !body.is_empty() {
            task.didReceiveData(&body);
        }
        task.didFinish();
    }
}

/// Ivars for the `Delegate` class.
struct DelegateIvars {
    handlers: Rc<Handlers>,
}

impl std::fmt::Debug for DelegateIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelegateIvars").finish()
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CocoaUIWebViewDelegate"]
    #[ivars = DelegateIvars]
    #[derive(Debug)]
    /// The navigation and UI delegate for `WebView`.
    struct Delegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: each method below is declared with its protocol selector; the
    // generated `WKNavigationDelegate`/`WKUIDelegate` trait methods are
    // macOS-only, but declaring them on the class is valid on iOS too.
    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            web_view: &WebView,
            action: &WKNavigationAction,
            decision_handler: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: see the module safety note.
            let target_frame = unsafe { action.targetFrame() };
            let info = ActionInfo {
                // SAFETY: see the module safety note.
                request: unsafe { action.request() },
                // SAFETY: see the module safety note.
                navigation_type: map_navigation_type(unsafe { action.navigationType() }),
                has_target_frame: target_frame.is_some(),
                target_is_main: target_frame
                    // SAFETY: see the module safety note.
                    .is_some_and(|frame| unsafe { frame.isMainFrame() }),
            };
            let handlers = self.ivars().handlers.clone();
            let policy = crate::callback::guarded("web view navigation action", || {
                handlers
                    .decide_action
                    .as_ref()
                    .map_or(Policy::Allow, |decide| decide(web_view, &info))
            });
            decision_handler.call((match policy {
                Policy::Allow => WKNavigationActionPolicy::Allow,
                Policy::Cancel => WKNavigationActionPolicy::Cancel,
            },));
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            web_view: &WebView,
            response: &WKNavigationResponse,
            decision_handler: &block2::DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            // SAFETY: see the module safety note.
            let raw = unsafe { response.response() };
            let http = raw.downcast_ref::<NSHTTPURLResponse>();
            let info = ResponseInfo {
                // SAFETY: see the module safety note.
                is_main_frame: unsafe { response.isForMainFrame() },
                status_code: http.map(|http| http.statusCode() as i64),
                url: raw
                    .URL()
                    .and_then(|url| url.absoluteString())
                    .map(|text| text.to_string())
                    .unwrap_or_default(),
                redirect_location: http.and_then(|http| {
                    http.valueForHTTPHeaderField(&NSString::from_str("Location"))
                        .map(|value| value.to_string())
                }),
            };
            let handlers = self.ivars().handlers.clone();
            let policy = crate::callback::guarded("web view navigation response", || {
                handlers
                    .decide_response
                    .as_ref()
                    .map_or(Policy::Allow, |decide| decide(web_view, &info))
            });
            decision_handler.call((match policy {
                Policy::Allow => WKNavigationResponsePolicy::Allow,
                Policy::Cancel => WKNavigationResponsePolicy::Cancel,
            },));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start_provisional(&self, web_view: &WebView, _navigation: &WKNavigation) {
            self.emit(web_view, Event::StartedProvisional);
        }

        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        fn did_receive_server_redirect(&self, web_view: &WebView, _navigation: &WKNavigation) {
            self.emit(web_view, Event::ServerRedirect);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WebView, _navigation: &WKNavigation) {
            self.emit(web_view, Event::Finished);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, web_view: &WebView, _navigation: &WKNavigation, error: &NSError) {
            self.emit(
                web_view,
                Event::Failed(error.localizedDescription().to_string()),
            );
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(
            &self,
            web_view: &WebView,
            _navigation: &WKNavigation,
            error: &NSError,
        ) {
            self.emit(
                web_view,
                Event::ProvisionalFailed(error.localizedDescription().to_string()),
            );
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn did_receive_authentication_challenge(
            &self,
            web_view: &WebView,
            challenge: &NSURLAuthenticationChallenge,
            completion_handler: &block2::DynBlock<
                dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential),
            >,
        ) {
            let protection_space = challenge.protectionSpace();
            // SAFETY: see the module safety note.
            let server_trust: Option<Retained<SecTrust>> = unsafe {
                // SAFETY: `protection_space` is live; `serverTrust` returns
                // a retained `SecTrust` reference for server-trust
                // challenges and nil otherwise.
                msg_send![&*protection_space, serverTrust]
            };
            let info = ChallengeInfo {
                method: protection_space.authenticationMethod().to_string(),
                server_trust,
                current_url: web_view
                    .url()
                    .and_then(|url| url.absoluteString())
                    .map(|text| text.to_string())
                    .unwrap_or_default(),
            };
            let handlers = self.ivars().handlers.clone();
            let decision = crate::callback::guarded("web view authentication challenge", || {
                handlers
                    .authentication
                    .as_ref()
                    .map_or(ChallengeDecision::PerformDefault, |decide| {
                        decide(web_view, &info)
                    })
            });
            match decision {
                ChallengeDecision::UseCredential => {
                    let credential: Option<Retained<NSURLCredential>> =
                        // SAFETY: see the module safety note.
                        info.server_trust.as_ref().and_then(|trust| unsafe {
                            // SAFETY: `credentialForTrust:` is a class
                            // method returning a retained credential for a
                            // live `SecTrust`.
                            msg_send![
                                objc2::class!(NSURLCredential),
                                credentialForTrust: &**trust
                            ]
                        });
                    let credential = credential.map_or(std::ptr::null_mut(), |credential| {
                        Retained::into_raw(credential).cast()
                    });
                    completion_handler.call((
                        NSURLSessionAuthChallengeDisposition::UseCredential,
                        credential,
                    ));
                }
                ChallengeDecision::Cancel => {
                    completion_handler.call((
                        NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge,
                        std::ptr::null_mut(),
                    ));
                }
                ChallengeDecision::PerformDefault => {
                    completion_handler.call((
                        NSURLSessionAuthChallengeDisposition::PerformDefaultHandling,
                        std::ptr::null_mut(),
                    ));
                }
            }
        }
    }

    unsafe impl WKUIDelegate for Delegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            web_view: &WebView,
            _configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _window_features: &WKWindowFeatures,
        ) -> Option<Retained<WebView>> {
            // SAFETY: see the module safety note.
            if unsafe { action.targetFrame() }.is_none() {
                // SAFETY: see the module safety note.
                self.emit(web_view, Event::OpenRequest(unsafe { action.request() }));
            }
            None
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, handlers: Rc<Handlers>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { handlers });
        // SAFETY: `this` is a live, allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }

    /// Runs the event closure, when one is set.
    fn emit(&self, web_view: &WebView, event: Event) {
        let handlers = self.ivars().handlers.clone();
        if let Some(handler) = handlers.event.as_ref() {
            crate::callback::guarded("web view event", || handler(web_view, event));
        }
    }
}

/// Maps a `WKNavigationType` constant to [`NavigationType`].
fn map_navigation_type(navigation_type: objc2_web_kit::WKNavigationType) -> NavigationType {
    use objc2_web_kit::WKNavigationType as Raw;
    match navigation_type {
        value if value == Raw::LinkActivated => NavigationType::LinkActivated,
        value if value == Raw::FormSubmitted => NavigationType::FormSubmitted,
        value if value == Raw::BackForward => NavigationType::BackForward,
        value if value == Raw::Reload => NavigationType::Reload,
        value if value == Raw::FormResubmitted => NavigationType::FormResubmitted,
        _ => NavigationType::Other,
    }
}

/// Owns a `WKWebView` and its weakly-held helper objects.
///
/// The helpers are the delegate pair, the progress observer and the
/// script-message handlers. Dropping a `WebViewController` unwinds in the
/// same order the Swift `WebViewWrapper` did: script handlers off the user
/// content controller first, then `stopLoading`, delegates nilled, user
/// scripts removed, and the progress observer, delegate and scheme handler
/// released by field teardown.
#[derive(Debug)]
pub struct WebViewController {
    view: Retained<WebView>,
    #[allow(dead_code)]
    delegate: Retained<Delegate>,
    #[allow(dead_code)]
    progress: ProgressToken,
    #[allow(dead_code)]
    scheme: Option<Retained<SchemeTaskHandler>>,
    script_handlers: RefCell<Vec<(String, Retained<ScriptMessageHandler>)>>,
}

impl WebViewController {
    /// Builds a web view with a default configuration plus `scheme`.
    pub fn new(mtm: MainThreadMarker, handlers: Handlers, scheme: Option<SchemeHandler>) -> Self {
        // SAFETY: see the module safety note.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };

        #[cfg(target_os = "ios")]
        // SAFETY: see the module safety note.
        unsafe {
            // SAFETY: `allowsInlineMediaPlayback` is not in the generated
            // bindings; it is a plain BOOL property on
            // `WKWebViewConfiguration`.
            let _: () = msg_send![&*configuration, setAllowsInlineMediaPlayback: true];
            configuration
                .setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
        }

        let scheme_handler = scheme.map(|scheme| {
            let handler = SchemeTaskHandler::new(mtm, scheme.clone());
            // SAFETY: `handler` is a live `WKURLSchemeHandler`.
            unsafe {
                configuration.setURLSchemeHandler_forURLScheme(
                    Some(ProtocolObject::from_ref(&*handler)),
                    &NSString::from_str(&scheme.scheme),
                );
            }
            handler
        });

        let frame = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(0.0, 0.0),
        );
        // SAFETY: see the module safety note.
        let view: Retained<WebView> = unsafe {
            // SAFETY: `initWithFrame:configuration:` is the designated
            // initializer; `configuration` is a live object.
            msg_send![
                WebView::alloc(mtm),
                initWithFrame: frame,
                configuration: &*configuration
            ]
        };
        let handlers = Rc::new(handlers);
        let delegate = Delegate::new(mtm, handlers.clone());
        // SAFETY: both are live protocol objects; the controller retains
        // the delegate so WebKit's weak references stay valid.
        // SAFETY: see the module safety note.
        unsafe {
            view.set_navigation_delegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.set_ui_delegate(Some(ProtocolObject::from_ref(&*delegate)));
        }

        let weak_view = Weak::from_retained(&view);
        let progress = ProgressToken::register(
            &view,
            Rc::new(move |progress| {
                let Some(view) = weak_view.load() else {
                    return;
                };
                if let Some(event) = handlers.event.as_ref() {
                    crate::callback::guarded("web view progress", || {
                        event(&view, Event::Progress(progress));
                    });
                }
            }),
        );

        Self {
            view,
            delegate,
            progress,
            scheme: scheme_handler,
            script_handlers: RefCell::new(Vec::new()),
        }
    }

    /// The `WKWebView` itself, for embedding.
    #[must_use]
    pub const fn view(&self) -> &Retained<WebView> {
        &self.view
    }

    /// Navigates to `url`.
    ///
    /// # Panics
    ///
    /// When `url` is not a URL Foundation can parse.
    pub fn load_url(&self, url: &str) {
        let url = NSURL::URLWithString(&NSString::from_str(url))
            .expect("WebView received an invalid URL");
        // SAFETY: `url` is a live `NSURL`.
        let request = NSURLRequest::requestWithURL(&url);
        self.load_request(&request);
    }

    /// Loads `request`.
    pub fn load_request(&self, request: &NSURLRequest) {
        self.view.load_request(request);
    }

    /// The active committed URL string.
    #[must_use]
    pub fn url_string(&self) -> Option<String> {
        self.view
            .url()
            .and_then(|url| url.absoluteString())
            .map(|text| text.to_string())
    }

    /// Whether the back-forward list has a previous entry.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.view.can_go_back()
    }

    /// Whether the back-forward list has a next entry.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.view.can_go_forward()
    }

    /// Navigates to the previous entry.
    pub fn go_back(&self) {
        self.view.go_back();
    }

    /// Navigates to the next entry.
    pub fn go_forward(&self) {
        self.view.go_forward();
    }

    /// Reloads the current document.
    pub fn reload(&self) {
        self.view.reload();
    }

    /// Stops an in-flight load.
    pub fn stop_loading(&self) {
        self.view.stop_loading();
    }

    /// `customUserAgent`; `None` restores `WebKit`'s default.
    pub fn set_custom_user_agent(&self, user_agent: Option<&str>) {
        self.view
            .set_custom_user_agent(user_agent.map(NSString::from_str).as_deref());
    }

    /// Runs `script` in the default world; `done` receives the result
    /// JSON-serialized (top-level non-JSON values fall back to their
    /// `description`) or the error's localized description.
    pub fn evaluate_javascript(
        &self,
        script: &str,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let done = RefCell::new(Some(done));
        let completion = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            if let Some(done) = done.borrow_mut().take() {
                done(script_result(result, error));
            }
        });
        // SAFETY: `completion` is copied by WebKit and invoked on the main
        // thread.
        // SAFETY: see the module safety note.
        unsafe {
            self.view
                .evaluate_javascript(&NSString::from_str(script), Some(&*completion));
        }
    }

    /// Calls `function_body` in the page content world — the entry point
    /// `callAsyncJavaScript` is required for, since `evaluateJavaScript`
    /// returns the promise object itself.
    pub fn call_async_javascript(
        &self,
        function_body: &str,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let done = RefCell::new(Some(done));
        let completion = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            if let Some(done) = done.borrow_mut().take() {
                done(script_result(result, error));
            }
        });
        let arguments = NSDictionary::<NSString, AnyObject>::new();
        // SAFETY: see the module safety note.
        let world = unsafe { WKContentWorld::pageWorld(MainThreadMarker::from(&*self.view)) };
        // SAFETY: `completion` is copied by WebKit; `frame` nil means the
        // main frame.
        // SAFETY: see the module safety note.
        unsafe {
            self.view.call_async_javascript(
                &NSString::from_str(function_body),
                Some(&arguments),
                None,
                &world,
                Some(&*completion),
            );
        }
    }

    /// Adds a user script.
    pub fn add_user_script(
        &self,
        source: &str,
        injection_time: InjectionTime,
        main_frame_only: bool,
    ) {
        let mtm = MainThreadMarker::from(&*self.view);
        let time = match injection_time {
            InjectionTime::DocumentStart => WKUserScriptInjectionTime::AtDocumentStart,
            InjectionTime::DocumentEnd => WKUserScriptInjectionTime::AtDocumentEnd,
        };
        // SAFETY: see the module safety note.
        let script = unsafe {
            // SAFETY: `initWithSource:injectionTime:forMainFrameOnly:` is
            // the designated initializer; the source string is a live
            // `NSString`.
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(source),
                time,
                main_frame_only,
            )
        };
        // SAFETY: see the module safety note.
        let user_content = unsafe { self.view.configuration().userContentController() };
        // SAFETY: `script` is a live `WKUserScript`.
        unsafe {
            user_content.addUserScript(&script);
        }
    }

    /// Removes every user script.
    pub fn remove_all_user_scripts(&self) {
        // SAFETY: see the module safety note.
        let user_content = unsafe { self.view.configuration().userContentController() };
        // SAFETY: see the module safety note.
        unsafe {
            user_content.removeAllUserScripts();
        }
    }

    /// Registers `handler` for `name` under `webkit.messageHandlers`. The
    /// controller retains the handler object; re-adding a name first removes
    /// the previous handler.
    pub fn add_script_message_handler(&self, name: &str, handler: Rc<dyn Fn(ScriptMessage)>) {
        self.remove_script_message_handler(name);
        let mtm = MainThreadMarker::from(&*self.view);
        let object = ScriptMessageHandler::new(mtm, handler);
        // SAFETY: see the module safety note.
        let user_content = unsafe { self.view.configuration().userContentController() };
        // SAFETY: `object` is a live `WKScriptMessageHandler`.
        unsafe {
            user_content.addScriptMessageHandler_name(
                ProtocolObject::from_ref(&*object),
                &NSString::from_str(name),
            );
        }
        self.script_handlers
            .borrow_mut()
            .push((name.to_string(), object));
    }

    /// Removes the handler registered under `name`, if any.
    pub fn remove_script_message_handler(&self, name: &str) {
        let mut handlers = self.script_handlers.borrow_mut();
        if handlers.iter().any(|(registered, _)| registered == name) {
            handlers.retain(|(registered, _)| registered != name);
            drop(handlers);
            // SAFETY: see the module safety note.
            let user_content = unsafe { self.view.configuration().userContentController() };
            // SAFETY: see the module safety note.
            unsafe {
                user_content.removeScriptMessageHandlerForName(&NSString::from_str(name));
            }
        }
    }

    /// Stores a cookie from a `Set-Cookie` header value.
    ///
    /// Without a current URL the cookie must carry a `Domain` attribute, the
    /// same precondition `HTTPCookie.cookiesWithResponseHeaderFields`
    /// imposes through the fallback origin URL.
    ///
    /// # Errors
    ///
    /// Returns [`CookieError`] for empty input, a missing `Domain`, and a
    /// value Foundation could not parse.
    pub fn set_cookie_header(&self, header: &str) -> Result<(), CookieError> {
        let trimmed = header.trim();
        if trimmed.is_empty() {
            return Err(CookieError::Empty);
        }
        let url = self.view.url().or_else(|| cookie_origin_url(trimmed));
        let Some(url) = url else {
            return Err(CookieError::MissingDomain);
        };
        let set_cookie = NSString::from_str("Set-Cookie");
        let value = NSString::from_str(trimmed);
        let header_fields =
            NSDictionary::<NSString, NSString>::from_slices(&[&*set_cookie], &[&*value]);
        let cookies = NSHTTPCookie::cookiesWithResponseHeaderFields_forURL(&header_fields, &url);
        let Some(cookie) = cookies.firstObject() else {
            return Err(CookieError::Malformed);
        };
        // SAFETY: see the module safety note.
        let store = unsafe {
            self.view
                .configuration()
                .websiteDataStore()
                .httpCookieStore()
        };
        // SAFETY: `cookie` is a live `NSHTTPCookie`.
        unsafe {
            store.setCookie_completionHandler(&cookie, None);
        }
        Ok(())
    }

    /// Reads every cookie in the store; `done` receives them on the main
    /// thread.
    pub fn all_cookies(&self, done: impl FnOnce(Vec<CookieRecord>) + 'static) {
        // SAFETY: see the module safety note.
        let store = unsafe {
            self.view
                .configuration()
                .websiteDataStore()
                .httpCookieStore()
        };
        let done = RefCell::new(Some(done));
        let completion = RcBlock::new(move |cookies: std::ptr::NonNull<NSArray<NSHTTPCookie>>| {
            // SAFETY: WebKit hands a non-null, live array here.
            let records = unsafe { cookies.as_ref() }
                .iter()
                .map(|cookie| cookie_record(&cookie))
                .collect();
            if let Some(done) = done.borrow_mut().take() {
                done(records);
            }
        });
        // SAFETY: `completion` is copied by WebKit and invoked once.
        unsafe {
            store.getAllCookies(&completion);
        }
    }
}

impl Drop for WebViewController {
    fn drop(&mut self) {
        // SAFETY: see the module safety note.
        let user_content = unsafe { self.view.configuration().userContentController() };
        for (name, _) in self.script_handlers.get_mut().drain(..) {
            // SAFETY: see the module safety note.
            unsafe {
                user_content.removeScriptMessageHandlerForName(&NSString::from_str(&name));
            }
        }
        // SAFETY: see the module safety note.
        unsafe {
            user_content.removeAllUserScripts();
            // SAFETY: the view is live; nil'ing the delegates detaches
            // WebKit's weak references before the delegate object releases.
            self.view.stop_loading();
            self.view.set_navigation_delegate(None);
            self.view.set_ui_delegate(None);
        }
    }
}

/// Resolves a `Location` header value against the response URL it arrived with.
///
/// `URL(string:relativeTo:)` semantics, so a relative header is rooted at the
/// response and an unparseable one passes through unchanged.
#[must_use]
pub fn resolve_redirect(base: &str, location: &str) -> String {
    let base = NSURL::URLWithString(&NSString::from_str(base));
    NSURL::URLWithString_relativeToURL(&NSString::from_str(location), base.as_deref())
        .and_then(|url| url.absoluteString())
        .map_or_else(|| location.to_string(), |url| url.to_string())
}

/// Evaluates a server trust reference — `SecTrustEvaluateWithError`.
///
/// # Errors
///
/// The failure's `CFError` description, for the `Ssl` event payload.
pub fn evaluate_server_trust(trust: &SecTrust) -> Result<(), String> {
    let mut error: *mut CFError = std::ptr::null_mut();
    // SAFETY: `trust` is a live `SecTrust`; `error` is a valid out-pointer
    // that yields a retained `CFError` on failure.
    // SAFETY: see the module safety note.
    let valid = unsafe { trust.evaluate_with_error(&raw mut error) };
    if valid {
        return Ok(());
    }
    // SAFETY: the API returns a `CFError` with +1 semantics on failure.
    let error = unsafe { Retained::from_raw(error) };
    Err(error.and_then(|error| error.description()).map_or_else(
        || String::from("the server trust evaluation failed"),
        |description| description.to_string(),
    ))
}

/// Turns an evaluated result into the reply string the bridge expects.
fn script_result(result: *mut AnyObject, error: *mut NSError) -> Result<String, String> {
    if !error.is_null() {
        // SAFETY: a non-null `NSError` from WebKit is a live object.
        let error = unsafe { &*error };
        return Err(error.localizedDescription().to_string());
    }
    if result.is_null() {
        return Ok(String::from("null"));
    }
    // SAFETY: a non-null `AnyObject` from WebKit is a live object.
    let result = unsafe { &*result };
    Ok(json_string(result))
}

/// JSON-serializes `object`, falling back to `description` when it is not a
/// valid JSON object — the shape `evaluateJavaScript` replies with.
fn json_string(object: &AnyObject) -> String {
    // SAFETY: `object` is a live object.
    if unsafe { NSJSONSerialization::isValidJSONObject(object) } {
        // SAFETY: `object` passed `isValidJSONObject`.
        if let Ok(data) = unsafe {
            NSJSONSerialization::dataWithJSONObject_options_error(object, NSJSONWritingOptions(0))
        } && let Ok(text) = String::from_utf8(data.to_vec())
        {
            return text;
        }
    }
    // SAFETY: `object` is a live object; `description` is `NSObject`.
    let description: Option<Retained<NSString>> = unsafe { msg_send![object, description] };
    description.map_or_else(
        || String::from("null"),
        |description| description.to_string(),
    )
}

/// The `Set-Cookie` `Domain` attribute as an `https://` origin URL, the
/// fallback `cookiesWithResponseHeaderFields` needs when the page has no
/// URL yet.
fn cookie_origin_url(header: &str) -> Option<Retained<NSURL>> {
    let domain = header.split(';').find_map(|attribute| {
        let (name, value) = attribute.trim().split_once('=')?;
        name.eq_ignore_ascii_case("domain")
            .then(|| value.trim().to_string())
    })?;
    let host = domain.strip_prefix('.').unwrap_or(&domain);
    if host.is_empty() {
        return None;
    }
    NSURL::URLWithString(&NSString::from_str(&format!("https://{host}/")))
}

/// One `NSHTTPCookie` as a [`CookieRecord`].
fn cookie_record(cookie: &NSHTTPCookie) -> CookieRecord {
    CookieRecord {
        name: cookie.name().to_string(),
        value: cookie.value().to_string(),
        domain: cookie.domain().to_string(),
        path: cookie.path().to_string(),
        expires: cookie
            .expiresDate()
            .map(|date| date.timeIntervalSince1970()),
        secure: cookie.isSecure(),
        http_only: cookie.isHTTPOnly(),
        same_site: cookie.sameSitePolicy().map(|policy| policy.to_string()),
    }
}
