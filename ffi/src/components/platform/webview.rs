//! WebView component FFI bindings.
//!
//! This module provides FFI bindings for the WebView component, allowing native backends
//! to create and control web views.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::pin::Pin;

use crate::closure::WuiFn;
use crate::{IntoFFI, IntoRust, WuiEnv, WuiStr};
use base64::Engine;
use cookie::Cookie;
use waterui_str::Str;
use waterui_webview::{
    CustomWebViewController, ScriptInjectionTime, Url, WebView, WebViewController, WebViewError,
    WebViewEvent, WebViewHandle,
};

// =============================================================================
// Script Injection Time FFI
// =============================================================================

/// FFI representation of script injection timing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiScriptInjectionTime {
    /// Inject at the start of document loading, before the DOM is constructed.
    DocumentStart = 0,
    /// Inject after the document has finished loading.
    DocumentEnd = 1,
}

impl IntoFFI for ScriptInjectionTime {
    type FFI = WuiScriptInjectionTime;
    fn into_ffi(self) -> Self::FFI {
        match self {
            ScriptInjectionTime::DocumentStart => WuiScriptInjectionTime::DocumentStart,
            ScriptInjectionTime::DocumentEnd => WuiScriptInjectionTime::DocumentEnd,
        }
    }
}

impl IntoRust for WuiScriptInjectionTime {
    type Rust = ScriptInjectionTime;
    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            WuiScriptInjectionTime::DocumentStart => ScriptInjectionTime::DocumentStart,
            WuiScriptInjectionTime::DocumentEnd => ScriptInjectionTime::DocumentEnd,
        }
    }
}

// =============================================================================
// Event FFI Types
// =============================================================================

/// FFI representation of WebView event types.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiWebViewEventType {
    /// No event (initial state).
    None = 0,
    /// The web view is about to navigate to a new URL.
    WillNavigate = 1,
    /// The web view is loading content.
    Loading = 2,
    /// The web view has finished loading.
    Loaded = 3,
    /// A redirect occurred.
    Redirect = 4,
    /// An SSL error occurred.
    SslError = 5,
    /// A general error occurred.
    Error = 6,
    /// Navigation state changed.
    StateChanged = 7,
}

/// FFI representation of a WebView event.
#[repr(C)]
pub struct WuiWebViewEvent {
    /// The type of event.
    pub event_type: WuiWebViewEventType,
    /// URL associated with the event (for WillNavigate, SslError, Error, Redirect from).
    pub url: WuiStr,
    /// Second URL (for Redirect to).
    pub url2: WuiStr,
    /// Error/message string (for SslError, Error).
    pub message: WuiStr,
    /// Loading progress (0.0 to 1.0, for Loading event).
    pub progress: f32,
    /// Whether can navigate back (for StateChanged).
    pub can_go_back: bool,
    /// Whether can navigate forward (for StateChanged).
    pub can_go_forward: bool,
}

impl WuiWebViewEvent {
    /// Create an empty event.
    pub fn empty() -> Self {
        Self {
            event_type: WuiWebViewEventType::None,
            url: Str::from_static("").into_ffi(),
            url2: Str::from_static("").into_ffi(),
            message: Str::from_static("").into_ffi(),
            progress: 0.0,
            can_go_back: false,
            can_go_forward: false,
        }
    }
}

impl IntoFFI for WebViewEvent {
    type FFI = WuiWebViewEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            WebViewEvent::None => WuiWebViewEvent::empty(),
            WebViewEvent::WillNavigate { url } => WuiWebViewEvent {
                event_type: WuiWebViewEventType::WillNavigate,
                url: url.inner().into_ffi(),
                ..WuiWebViewEvent::empty()
            },
            WebViewEvent::Loading { progress } => WuiWebViewEvent {
                event_type: WuiWebViewEventType::Loading,
                progress,
                ..WuiWebViewEvent::empty()
            },
            WebViewEvent::Loaded => WuiWebViewEvent {
                event_type: WuiWebViewEventType::Loaded,
                ..WuiWebViewEvent::empty()
            },
            WebViewEvent::Redirect { from, to } => WuiWebViewEvent {
                event_type: WuiWebViewEventType::Redirect,
                url: from.inner().into_ffi(),
                url2: to.inner().into_ffi(),
                ..WuiWebViewEvent::empty()
            },
            WebViewEvent::Error(err) => match err {
                WebViewError::Ssl { url, message } => WuiWebViewEvent {
                    event_type: WuiWebViewEventType::SslError,
                    url: url.inner().into_ffi(),
                    message: message.into_ffi(),
                    ..WuiWebViewEvent::empty()
                },
                _ => WuiWebViewEvent {
                    event_type: WuiWebViewEventType::Error,
                    message: Str::from(err.to_string()).into_ffi(),
                    ..WuiWebViewEvent::empty()
                },
            },
            WebViewEvent::StateChanged {
                can_go_back,
                can_go_forward,
            } => WuiWebViewEvent {
                event_type: WuiWebViewEventType::StateChanged,
                can_go_back,
                can_go_forward,
                ..WuiWebViewEvent::empty()
            },
        }
    }
}

/// Parses a URL string, returning a fallback URL if parsing fails.
fn parse_url_or_blank(s: Str) -> Url {
    let text = s.as_str();
    if text.is_empty() {
        return Url::from("about:blank");
    }
    Url::parse(text).unwrap_or_else(|| Url::from(s))
}

impl IntoRust for WuiWebViewEvent {
    type Rust = WebViewEvent;
    unsafe fn into_rust(self) -> Self::Rust {
        match self.event_type {
            WuiWebViewEventType::None => WebViewEvent::None,
            WuiWebViewEventType::WillNavigate => WebViewEvent::WillNavigate {
                url: parse_url_or_blank(unsafe { self.url.into_rust() }),
            },
            WuiWebViewEventType::Loading => WebViewEvent::Loading {
                progress: self.progress,
            },
            WuiWebViewEventType::Loaded => WebViewEvent::Loaded,
            WuiWebViewEventType::Redirect => WebViewEvent::Redirect {
                from: parse_url_or_blank(unsafe { self.url.into_rust() }),
                to: parse_url_or_blank(unsafe { self.url2.into_rust() }),
            },
            WuiWebViewEventType::SslError => WebViewEvent::Error(WebViewError::Ssl {
                url: parse_url_or_blank(unsafe { self.url.into_rust() }),
                message: unsafe { self.message.into_rust() },
            }),
            WuiWebViewEventType::Error => {
                let msg: Str = unsafe { self.message.into_rust() };
                WebViewEvent::Error(WebViewError::LoadFailed(msg))
            }
            WuiWebViewEventType::StateChanged => WebViewEvent::StateChanged {
                can_go_back: self.can_go_back,
                can_go_forward: self.can_go_forward,
            },
        }
    }
}

// =============================================================================
// WebViewHandle FFI
// =============================================================================

/// Callback for JavaScript execution results.
#[repr(C)]
pub struct WuiJsCallback {
    /// Opaque pointer to callback data.
    pub data: *mut (),
    /// Function to call with result. success=true means result is the value, false means error.
    pub call: unsafe extern "C" fn(data: *mut (), success: bool, result: WuiStr),
}

/// Message payload emitted from JavaScript to a native-registered handler.
///
/// `payload_base64` is base64-encoded bytes from JavaScript.
/// `reply` must be called exactly once for request/response semantics.
#[repr(C)]
pub struct WuiWebViewMessage {
    pub payload_base64: WuiStr,
    pub reply: WuiJsCallback,
}

/// FFI representation of a WebView handle with function pointers.
///
/// Native backends create this struct with function pointers to their implementation.
#[repr(C)]
pub struct WuiWebViewHandle {
    /// Opaque pointer to native WebView wrapper.
    pub data: *mut (),

    // Navigation
    /// Navigate back in history.
    pub go_back: unsafe extern "C" fn(*mut ()),
    /// Navigate forward in history.
    pub go_forward: unsafe extern "C" fn(*mut ()),
    /// Navigate to URL.
    pub go_to: unsafe extern "C" fn(*mut (), WuiStr),
    /// Stop loading.
    pub stop: unsafe extern "C" fn(*mut ()),
    /// Refresh/reload page.
    pub refresh: unsafe extern "C" fn(*mut ()),

    // State queries
    /// Returns whether can go back.
    pub can_go_back: unsafe extern "C" fn(*const ()) -> bool,
    /// Returns whether can go forward.
    pub can_go_forward: unsafe extern "C" fn(*const ()) -> bool,

    // Configuration
    /// Set user agent string.
    pub set_user_agent: unsafe extern "C" fn(*mut (), WuiStr),

    /// Enable or disable following redirects.
    pub set_redirects_enabled: unsafe extern "C" fn(*mut (), bool),

    // Script injection
    /// Inject a script that runs on every page load.
    pub inject_script: unsafe extern "C" fn(*mut (), WuiStr, WuiScriptInjectionTime),

    // Event watching
    /// Set event callback. Native calls this when events occur.
    pub watch: unsafe extern "C" fn(*mut (), WuiFn<WuiWebViewEvent>),

    // JS-to-native messaging
    /// Register a named handler that can be called from JavaScript.
    ///
    /// Backends are expected to provide a Promise-based API where possible:
    /// JavaScript sends `payload_base64` and receives a base64 reply.
    pub add_handler: Option<unsafe extern "C" fn(*mut (), WuiStr, WuiFn<WuiWebViewMessage>)>,
    /// Removes a previously added handler.
    pub remove_handler: Option<unsafe extern "C" fn(*mut (), WuiStr)>,

    // Cookies
    /// Sets a cookie for the web view. The string is a Set-Cookie header value.
    pub set_cookie: Option<unsafe extern "C" fn(*mut (), WuiStr)>,
    /// Gets cookies as newline-separated Set-Cookie strings.
    pub get_cookies: Option<unsafe extern "C" fn(*const ()) -> WuiStr>,

    // JavaScript
    /// Execute JavaScript on the currently loaded page and call callback with result.
    pub run_javascript: unsafe extern "C" fn(*mut (), WuiStr, WuiJsCallback),

    // Cleanup
    /// Release the native handle.
    pub drop: unsafe extern "C" fn(*mut ()),
}

/// Rust wrapper that implements `WebViewHandle` by delegating to FFI function pointers.
///
/// This struct is public so that the FFI layer can downcast `AnyWebViewHandle`
/// to extract the native webview pointer for rendering.
type WebViewWatchers = Rc<RefCell<Vec<Rc<dyn Fn(WebViewEvent)>>>>;

pub struct FfiWebViewHandle {
    ffi: WuiWebViewHandle,
    watchers: WebViewWatchers,
    watcher_installed: Cell<bool>,
}

impl FfiWebViewHandle {
    fn new(ffi: WuiWebViewHandle) -> Self {
        Self {
            ffi,
            watchers: Rc::new(RefCell::new(Vec::new())),
            watcher_installed: Cell::new(false),
        }
    }

    /// Returns the raw pointer to the native WebView wrapper.
    ///
    /// This pointer points to the native `WebViewWrapper` (Swift/Kotlin)
    /// which contains the underlying WKWebView or Android WebView.
    pub fn native_ptr(&self) -> *mut () {
        self.ffi.data
    }
}

impl Drop for FfiWebViewHandle {
    fn drop(&mut self) {
        unsafe {
            (self.ffi.drop)(self.ffi.data);
        }
    }
}

impl WebViewHandle for FfiWebViewHandle {
    fn go_back(&self) {
        unsafe { (self.ffi.go_back)(self.ffi.data) }
    }

    fn go_forward(&self) {
        unsafe { (self.ffi.go_forward)(self.ffi.data) }
    }

    fn go_to(&self, url: &str) {
        let owned_url = Str::from(url.to_string());
        unsafe { (self.ffi.go_to)(self.ffi.data, owned_url.into_ffi()) }
    }

    fn stop(&self) {
        unsafe { (self.ffi.stop)(self.ffi.data) }
    }

    fn refresh(&self) {
        unsafe { (self.ffi.refresh)(self.ffi.data) }
    }

    fn can_go_back(&self) -> bool {
        unsafe { (self.ffi.can_go_back)(self.ffi.data) }
    }

    fn can_go_forward(&self) -> bool {
        unsafe { (self.ffi.can_go_forward)(self.ffi.data) }
    }

    fn set_user_agent(&self, user_agent: &str) {
        let owned_ua = Str::from(user_agent.to_string());
        unsafe { (self.ffi.set_user_agent)(self.ffi.data, owned_ua.into_ffi()) }
    }

    fn set_redirects_enabled(&self, enabled: bool) {
        unsafe { (self.ffi.set_redirects_enabled)(self.ffi.data, enabled) }
    }

    fn inject_script(&self, script: &str, time: ScriptInjectionTime) {
        let owned_script = Str::from(script.to_string());
        unsafe { (self.ffi.inject_script)(self.ffi.data, owned_script.into_ffi(), time.into_ffi()) }
    }

    fn watch(&self, f: impl Fn(WebViewEvent) + 'static) {
        self.watchers.borrow_mut().push(Rc::new(f));

        if self.watcher_installed.replace(true) {
            return;
        }

        let watchers = self.watchers.clone();
        // Wrap a single Rust closure in a WuiFn that converts FFI events to Rust events
        // and fan-outs to all registered watchers.
        let callback = WuiFn::from(move |ffi_event: WuiWebViewEvent| {
            let event = unsafe { ffi_event.into_rust() };
            let snapshot = watchers.borrow().clone();
            for watcher in snapshot {
                watcher(event.clone())
            }
        });

        unsafe { (self.ffi.watch)(self.ffi.data, callback) }
    }

    fn add_handler(&self, name: &str, handler: Box<dyn Fn(&[u8]) -> Vec<u8> + 'static>) {
        let add_handler = self
            .ffi
            .add_handler
            .expect("WebView backend must implement `add_handler`");

        let engine = base64::engine::general_purpose::STANDARD;
        let name = Str::from(name.to_string());
        let callback = WuiFn::from(move |msg: WuiWebViewMessage| {
            let payload_b64: Str = unsafe { msg.payload_base64.into_rust() };
            let payload = match engine.decode(payload_b64.as_str()) {
                Ok(bytes) => bytes,
                Err(err) => {
                    let message = Str::from(err.to_string());
                    unsafe { (msg.reply.call)(msg.reply.data, false, message.into_ffi()) };
                    return;
                }
            };

            let reply_bytes = handler(&payload);
            let reply_b64 = engine.encode(reply_bytes);
            let reply = Str::from(reply_b64);
            unsafe { (msg.reply.call)(msg.reply.data, true, reply.into_ffi()) };
        });

        unsafe { add_handler(self.ffi.data, name.into_ffi(), callback) }
    }

    fn remove_handler(&self, name: &str) {
        let remove_handler = self
            .ffi
            .remove_handler
            .expect("WebView backend must implement `remove_handler`");

        let name = Str::from(name.to_string());
        unsafe { remove_handler(self.ffi.data, name.into_ffi()) }
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let set_cookie = self
            .ffi
            .set_cookie
            .expect("WebView backend must implement `set_cookie`");
        let cookie = Str::from(cookie.to_string());
        unsafe { set_cookie(self.ffi.data, cookie.into_ffi()) }
    }

    fn get_cookies(&self) -> Vec<Cookie<'static>> {
        let get_cookies = self
            .ffi
            .get_cookies
            .expect("WebView backend must implement `get_cookies`");

        let raw = unsafe { get_cookies(self.ffi.data.cast_const()) };
        let text: Str = unsafe { raw.into_rust() };
        text.as_str()
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                Cookie::parse(trimmed.to_string())
                    .ok()
                    .map(Cookie::into_owned)
            })
            .collect()
    }

    fn run_javascript(&self, script: &str) -> impl core::future::Future<Output = Result<Str, Str>> {
        use alloc::rc::Rc;
        use core::cell::RefCell;
        use core::task::{Poll, Waker};

        // Create a shared state for the async result
        struct JsResultState {
            result: Option<Result<Str, Str>>,
            waker: Option<Waker>,
        }

        let state = Rc::new(RefCell::new(JsResultState {
            result: None,
            waker: None,
        }));

        // Create the callback
        let state_clone = state.clone();
        let callback_box: Box<Box<dyn FnOnce(bool, Str)>> =
            Box::new(Box::new(move |success: bool, result_str: Str| {
                let mut state = state_clone.borrow_mut();
                state.result = Some(if success {
                    Ok(result_str)
                } else {
                    Err(result_str)
                });
                if let Some(waker) = state.waker.take() {
                    waker.wake();
                }
            }));
        let callback_data = Box::into_raw(callback_box).cast::<()>();

        unsafe extern "C" fn js_callback_trampoline(data: *mut (), success: bool, result: WuiStr) {
            let callback = unsafe { Box::from_raw(data.cast::<Box<dyn FnOnce(bool, Str)>>()) };
            let result_str: Str = unsafe { result.into_rust() };
            callback(success, result_str);
        }

        let ffi_callback = WuiJsCallback {
            data: callback_data,
            call: js_callback_trampoline,
        };

        // Call the FFI function
        let owned_script = Str::from(script.to_string());
        unsafe {
            (self.ffi.run_javascript)(self.ffi.data, owned_script.into_ffi(), ffi_callback);
        }

        // Return a future that polls the state
        struct JsFuture {
            state: Rc<RefCell<JsResultState>>,
        }

        impl core::future::Future for JsFuture {
            type Output = Result<Str, Str>;

            fn poll(self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Self::Output> {
                let mut state = self.state.borrow_mut();
                if let Some(result) = state.result.take() {
                    Poll::Ready(result)
                } else {
                    state.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }

        JsFuture { state }
    }
}

// =============================================================================
// WebView Raw View
// =============================================================================

opaque!(WuiWebView, WebView);
ffi_view!(WebView, *mut WuiWebView, webview);

/// Gets the native handle pointer from a WebView.
///
/// Returns the opaque pointer to the native WebView wrapper (Swift/Kotlin).
/// This pointer can be used by native backends to access the underlying
/// WKWebView or Android WebView.
///
/// # Safety
///
/// - The caller must ensure that `webview` is a valid pointer to a `WuiWebView`.
/// - The WebView must have been created via the FFI WebViewController (i.e., the handle
///   must be an `FfiWebViewHandle`). This is guaranteed when the native backend properly
///   installed the WebViewController via `waterui_env_install_webview_controller`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_webview_native_handle(webview: *mut WuiWebView) -> *mut () {
    unsafe {
        let webview = crate::expect_non_null(webview, "waterui_webview_native_handle", "webview");
        let handle = webview.0.handle();
        let ffi_handle = handle.downcast_ref::<FfiWebViewHandle>().unwrap_or_else(|| {
            panic!(
                "waterui_webview_native_handle requires a WebView created by the backend-installed FFI WebViewController"
            )
        });
        ffi_handle.native_ptr()
    }
}

// =============================================================================
// WebViewController Installation
// =============================================================================

/// Type for the native function that creates a new WebView.
pub type WuiCreateWebViewFn = unsafe extern "C" fn() -> WuiWebViewHandle;

/// FFI-compatible WebViewController implementation.
struct FfiWebViewController {
    create_fn: WuiCreateWebViewFn,
}

impl CustomWebViewController for FfiWebViewController {
    fn open(&self) -> impl WebViewHandle {
        let handle = unsafe { (self.create_fn)() };
        FfiWebViewHandle::new(handle)
    }
}

/// Installs a WebViewController into the environment from a native factory function.
///
/// Native backends call this during initialization to register their WebView factory.
/// The factory creates blank WebViews that can be navigated with `go_to()`.
///
/// # Safety
///
/// The caller must ensure that:
/// - `env` is a valid pointer to a `WuiEnv`
/// - `create_fn` is a valid function pointer that returns a properly initialized `WuiWebViewHandle`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_webview_controller(
    env: *mut WuiEnv,
    create_fn: WuiCreateWebViewFn,
) {
    let env =
        unsafe { crate::expect_non_null_mut(env, "waterui_env_install_webview_controller", "env") };

    let controller = WebViewController::new(FfiWebViewController { create_fn });
    env.insert(controller);
}
