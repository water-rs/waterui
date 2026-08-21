//! `WebView` component FFI bindings.
//!
//! This module provides FFI bindings for the `WebView` component, allowing native backends
//! to create and control web views.

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::cell::Cell;

use crate::closure::WuiFn;
use crate::reactive::WuiComputed;
use crate::{IntoFFI, IntoRust, WuiEnv, WuiStr};
use base64::Engine;
use cookie::Cookie;
use nami::{Signal, SignalExt};
use waterui_str::Str;
use waterui_webview::{
    BackendEvent, CustomWebViewController, JsReply, ScriptInjectionTime, Url, WatcherGuard,
    WatcherSet, WebView, WebViewController, WebViewError, WebViewEvent, WebViewHandle,
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
            Self::DocumentStart => WuiScriptInjectionTime::DocumentStart,
            Self::DocumentEnd => WuiScriptInjectionTime::DocumentEnd,
        }
    }
}

impl IntoRust for WuiScriptInjectionTime {
    type Rust = ScriptInjectionTime;
    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::DocumentStart => ScriptInjectionTime::DocumentStart,
            Self::DocumentEnd => ScriptInjectionTime::DocumentEnd,
        }
    }
}

// =============================================================================
// Event FFI Types
// =============================================================================

/// FFI representation of `WebView` event types.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiWebViewEventType {
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

/// FFI representation of a `WebView` event.
#[repr(C)]
#[derive(Debug)]
pub struct WuiWebViewEvent {
    /// The type of event.
    pub event_type: WuiWebViewEventType,
    /// URL associated with the event (for `WillNavigate`, `SslError`, Error, Redirect from).
    pub url: *mut WuiStr,
    /// Second URL (for Redirect to).
    pub url2: *mut WuiStr,
    /// Error/message string (for `SslError`, Error).
    pub message: *mut WuiStr,
    /// Loading progress (0.0 to 1.0, for Loading event).
    pub progress: f32,
    /// Whether can navigate back (for `StateChanged`).
    pub can_go_back: bool,
    /// Whether can navigate forward (for `StateChanged`).
    pub can_go_forward: bool,
}

/// Converts a URL that a backend reported alongside an event.
///
/// Backends report whatever the engine actually navigated to, which routinely
/// includes `about:blank`, `data:` documents, `blob:` URLs and `file://` paths.
/// This conversion is therefore total: it must not reject any of them, and it
/// must never abort the process — which it previously did, because `Url::parse`
/// accepts only web URLs and this unwrapped its `None`.
fn parse_url(s: &Str) -> Url {
    s.as_str().parse().unwrap_or_else(|error| {
        panic!(
            "WebView backend emitted an unparseable URL {:?}: {error}",
            s.as_str()
        )
    })
}

/// Takes ownership of a string field out of a `WuiWebViewEvent`.
///
/// # Safety
///
/// `value` must be an owning `WuiStr` the backend attached to the event, and may
/// be taken only once.
// SAFETY: the backend fills the string fields its event type defines with owning
// `WuiStr` handles, and each arm reads only its own fields, exactly once.
unsafe fn take_event_string(value: *mut WuiStr) -> Str {
    // SAFETY: the caller contract makes `value` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    let value = unsafe { Box::from_raw(value) };
    // SAFETY: the caller contract makes `value` a valid owning event pointer; it is
    // read once and converted here.
    unsafe { (*value).into_rust() }
}

impl IntoRust for WuiWebViewEvent {
    type Rust = BackendEvent;
    unsafe fn into_rust(self) -> Self::Rust {
        match self.event_type {
            WuiWebViewEventType::WillNavigate => BackendEvent::Event(WebViewEvent::WillNavigate {
                // SAFETY: the backend fills the string fields its event type defines
                // with owning `WuiStr` handles, and each arm reads only its
                // own fields, exactly once.
                url: parse_url(&unsafe { take_event_string(self.url) }),
            }),
            WuiWebViewEventType::Loading => BackendEvent::Event(WebViewEvent::Loading {
                progress: self.progress,
            }),
            WuiWebViewEventType::Loaded => BackendEvent::Event(WebViewEvent::Loaded),
            WuiWebViewEventType::Redirect => BackendEvent::Event(WebViewEvent::Redirect {
                // SAFETY: the backend fills the string fields its event type defines
                // with owning `WuiStr` handles, and each arm reads only its
                // own fields, exactly once.
                from: parse_url(&unsafe { take_event_string(self.url) }),
                // SAFETY: the backend fills the string fields its event type defines
                // with owning `WuiStr` handles, and each arm reads only its
                // own fields, exactly once.
                to: parse_url(&unsafe { take_event_string(self.url2) }),
            }),
            WuiWebViewEventType::SslError => {
                BackendEvent::Event(WebViewEvent::Error(WebViewError::Ssl {
                    // SAFETY: the backend fills the string fields its event type defines
                    // with owning `WuiStr` handles, and each arm reads only its
                    // own fields, exactly once.
                    url: parse_url(&unsafe { take_event_string(self.url) }),
                    // SAFETY: the backend fills the string fields its event type defines
                    // with owning `WuiStr` handles, and each arm reads only its
                    // own fields, exactly once.
                    message: unsafe { take_event_string(self.message) },
                }))
            }
            WuiWebViewEventType::Error => {
                BackendEvent::Event(WebViewEvent::Error(WebViewError::LoadFailed(unsafe {
                    // SAFETY: the backend fills the string fields its event type defines
                    // with owning `WuiStr` handles, and each arm reads only its
                    // own fields, exactly once.
                    take_event_string(self.message)
                })))
            }
            WuiWebViewEventType::StateChanged => BackendEvent::NavigationState {
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
#[derive(Debug)]
pub struct WuiJsCallback {
    /// Opaque callback state consumed by `call`.
    pub data: *mut (),
    /// One-shot completion. `success=true` means `result` is the value;
    /// `false` means it is the error. Native must invoke it exactly once.
    pub call: unsafe extern "C" fn(data: *mut (), success: bool, result: WuiStr),
}

/// One-shot completion callback for an owned string result.
#[repr(C)]
#[derive(Debug)]
pub struct WuiStringCallback {
    /// Opaque callback state consumed by `call`.
    pub data: *mut (),
    /// Completes the operation and transfers ownership of `result`.
    pub call: unsafe extern "C" fn(data: *mut (), result: WuiStr),
}

/// Which of the bridge's two channels a handler's reply crosses on.
///
/// The page sees the difference — a value it can use, or a `Uint8Array` — so the
/// answer has to survive the trip out. It used to be dropped here, and every
/// reply reached the page as base64 text.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiJsReplyKind {
    /// The bytes are a serialized JSON value; the page receives the value.
    Json = 0,
    /// The bytes are opaque; the page receives a `Uint8Array`.
    Bytes = 1,
}

/// One-shot reply to one bridge call.
///
/// Distinct from [`WuiJsCallback`], which completes a `run_javascript` call and
/// has no channel to choose.
#[repr(C)]
#[derive(Debug)]
pub struct WuiWebViewReply {
    /// Opaque callback state consumed by `call`.
    pub data: *mut (),
    /// One-shot completion. `success=true` means `result` is base64 of the
    /// handler's output and `kind` says how to read it; `false` means `result`
    /// is the error message and `kind` is ignored. Native must invoke it
    /// exactly once.
    pub call:
        unsafe extern "C" fn(data: *mut (), success: bool, kind: WuiJsReplyKind, result: WuiStr),
}

/// Message payload emitted from JavaScript to a native-registered handler.
///
/// `payload_base64` is base64-encoded bytes from JavaScript.
/// `reply` must be called exactly once for request/response semantics.
#[repr(C)]
#[derive(Debug)]
pub struct WuiWebViewMessage {
    /// Base64-encoded message bytes sent from JavaScript.
    pub payload_base64: WuiStr,
    /// One-shot callback used to send a base64-encoded reply back to JavaScript.
    pub reply: WuiWebViewReply,
}

/// FFI representation of a `WebView` handle with function pointers.
///
/// Native backends create this struct with function pointers to their implementation.
#[repr(C)]
#[derive(Debug)]
pub struct WuiWebViewHandle {
    /// Opaque pointer to native `WebView` wrapper.
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

    /// Takes ownership of a reactive redirect-following signal.
    pub set_redirects_enabled: unsafe extern "C" fn(*mut (), *mut WuiComputed<bool>),

    // Script injection
    /// Inject a script that runs on every page load.
    ///
    /// The first string is a key naming the script: injecting again under a key
    /// already in use replaces that script rather than adding a second copy.
    /// The mirrored-state seed relies on it, being re-rendered and replaced
    /// before every navigation.
    pub inject_script: unsafe extern "C" fn(*mut (), WuiStr, WuiStr, WuiScriptInjectionTime),

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
    /// Restricts which documents may reach the bridge.
    ///
    /// Receives newline-separated rule tokens: `*` for every origin, `file:` for
    /// any local file, and otherwise an exact `scheme://host[:port]` to compare
    /// the calling frame's origin against. **The empty string denies every
    /// document** — that is what the default policy resolves to when the view is
    /// opened at a URL with no origin — and a backend that treats it as `*`
    /// hands every registered handler to whatever the view navigates to.
    pub set_bridge_origins: Option<unsafe extern "C" fn(*mut (), WuiStr)>,

    // Cookies
    /// Sets a cookie for the web view. The string is a Set-Cookie header value.
    pub set_cookie: Option<unsafe extern "C" fn(*mut (), WuiStr)>,
    /// Gets cookies asynchronously as newline-separated Set-Cookie strings.
    pub get_cookies: Option<unsafe extern "C" fn(*const (), WuiStringCallback)>,

    // JavaScript
    /// Execute JavaScript on the currently loaded page and call callback with result.
    ///
    /// The raw path: the value comes back however the engine marshals it.
    pub run_javascript: unsafe extern "C" fn(*mut (), WuiStr, WuiJsCallback),

    /// Run the string as the body of an `async` function and **await** the
    /// promise it returns, then call the callback with the resolved value.
    ///
    /// Every typed evaluation goes through here, because the shared wrapper in
    /// `js/eval.js` is `async`: an engine API that does not await — WebKit's
    /// `evaluateJavaScript`, `webkit_web_view_evaluate_javascript`,
    /// `WebView.evaluateJavascript` — hands back the promise object instead of
    /// the JSON envelope, and every `eval!`/`exec!` fails while mirrored state
    /// silently stops reaching the page. Use `callAsyncJavaScript`,
    /// `webkit_web_view_call_async_javascript_function`, `Runtime.evaluate`
    /// with `awaitPromise`, or resolve the promise in JavaScript and report the
    /// result through the backend's own bridge.
    pub call_async_javascript: unsafe extern "C" fn(*mut (), WuiStr, WuiJsCallback),

    // Cleanup
    /// Release the native handle.
    pub drop: unsafe extern "C" fn(*mut ()),
}

/// Rust wrapper that implements `WebViewHandle` by delegating to FFI function pointers.
///
/// This struct is public so that the FFI layer can downcast `AnyWebViewHandle`
/// to extract the native webview pointer for rendering.
pub struct FfiWebViewHandle {
    ffi: WuiWebViewHandle,
    /// Rust-side watchers, so the single `WuiFn` trampoline installed on the
    /// native handle can fan out to all of them and each can be removed.
    watchers: WatcherSet<BackendEvent>,
    watcher_installed: Cell<bool>,
}

impl core::fmt::Debug for FfiWebViewHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FfiWebViewHandle")
            .field("ffi", &self.ffi)
            .field("watcher_installed", &self.watcher_installed)
            .finish_non_exhaustive()
    }
}

impl FfiWebViewHandle {
    pub(crate) fn new(ffi: WuiWebViewHandle) -> Self {
        Self {
            ffi,
            watchers: WatcherSet::new(),
            watcher_installed: Cell::new(false),
        }
    }

    /// Returns the raw pointer to the native `WebView` wrapper.
    ///
    /// This pointer points to the native `WebViewWrapper` (Swift/Kotlin)
    /// which contains the underlying `WKWebView` or Android `WebView`.
    pub const fn native_ptr(&self) -> *mut () {
        self.ffi.data
    }
}

impl Drop for FfiWebViewHandle {
    fn drop(&mut self) {
        unsafe {
            // SAFETY: `ffi.data` and this function pointer were registered together
            // by the backend for this controller, which is alive for as long
            // as `self` is.
            (self.ffi.drop)(self.ffi.data);
        }
    }
}

impl WebViewHandle for FfiWebViewHandle {
    fn go_back(&self) {
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.go_back)(self.ffi.data) }
    }

    fn go_forward(&self) {
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.go_forward)(self.ffi.data) }
    }

    fn go_to(&self, url: &Url) {
        let owned_url = Str::from(url.as_str().to_string());
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.go_to)(self.ffi.data, owned_url.into_ffi()) }
    }

    fn stop(&self) {
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.stop)(self.ffi.data) }
    }

    fn refresh(&self) {
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.refresh)(self.ffi.data) }
    }

    fn can_go_back(&self) -> bool {
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.can_go_back)(self.ffi.data) }
    }

    fn can_go_forward(&self) -> bool {
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.can_go_forward)(self.ffi.data) }
    }

    fn set_user_agent(&self, user_agent: &str) {
        let owned_ua = Str::from(user_agent.to_string());
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.set_user_agent)(self.ffi.data, owned_ua.into_ffi()) }
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        unsafe {
            // SAFETY: `ffi.data` and this function pointer were registered together
            // by the backend for this controller, which is alive for as long
            // as `self` is.
            (self.ffi.set_redirects_enabled)(self.ffi.data, enabled.computed().into_ffi());
        }
    }

    fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        let owned_key = Str::from(key.to_string());
        let owned_script = Str::from(script.to_string());
        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe {
            (self.ffi.inject_script)(
                self.ffi.data,
                owned_key.into_ffi(),
                owned_script.into_ffi(),
                time.into_ffi(),
            );
        }
    }

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        let guard = self.watchers.insert(f);

        if self.watcher_installed.replace(true) {
            return guard;
        }

        let watchers = self.watchers.clone();
        // Wrap a single Rust closure in a WuiFn that converts FFI events to Rust events
        // and fan-outs to all registered watchers.
        let callback = WuiFn::from(move |ffi_event: WuiWebViewEvent| {
            // SAFETY: the caller contract makes `ffi_event` an owning handle from
            // the matching FFI constructor; it is consumed here and not
            // observed again.
            let event = unsafe { ffi_event.into_rust() };
            watchers.emit(&event);
        });

        // SAFETY: `ffi.data` and this function pointer were registered together by
        // the backend for this controller, which is alive for as long as
        // `self` is.
        unsafe { (self.ffi.watch)(self.ffi.data, callback) };
        guard
    }

    fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        let add_handler = self
            .ffi
            .add_handler
            .expect("WebView backend must implement `add_handler`");

        let engine = base64::engine::general_purpose::STANDARD;
        let name = Str::from(name.to_string());
        let callback = WuiFn::from(move |msg: WuiWebViewMessage| {
            // SAFETY: the caller contract makes `payload_base64` an owning handle
            // from the matching FFI constructor; it is consumed here and not
            // observed again.
            let payload_b64: Str = unsafe { msg.payload_base64.into_rust() };
            let payload = match engine.decode(payload_b64.as_str()) {
                Ok(bytes) => bytes,
                Err(err) => {
                    let message = Str::from(err.to_string());
                    // SAFETY: the backend registered `reply.call` with `reply.data`
                    // for this message; the early return below makes this the only
                    // reply sent.
                    unsafe {
                        (msg.reply.call)(
                            msg.reply.data,
                            false,
                            WuiJsReplyKind::Json,
                            message.into_ffi(),
                        );
                    };
                    return;
                }
            };

            // The reply callback is one-shot and may be invoked later, which is
            // what lets a handler be asynchronous without the backend changing.
            let future = handler(&payload);
            let reply_call = msg.reply.call;
            let reply_data = msg.reply.data;
            executor_core::spawn_local(async move {
                let engine = base64::engine::general_purpose::STANDARD;
                let (success, kind, payload) = match future.await {
                    Ok(JsReply::Json(json)) => (true, WuiJsReplyKind::Json, engine.encode(json)),
                    Ok(JsReply::Bytes(bytes)) => {
                        (true, WuiJsReplyKind::Bytes, engine.encode(bytes))
                    }
                    Err(message) => (false, WuiJsReplyKind::Json, message),
                };
                // SAFETY: as above — the failure path returned early, so this is
                // the single reply for this message.
                unsafe { (reply_call)(reply_data, success, kind, Str::from(payload).into_ffi()) };
            })
            .detach();
        });

        // SAFETY: `add_handler` and `ffi.data` were registered together for this
        // controller, which outlives the call; the name and callback are passed by
        // value into backend ownership.
        unsafe { add_handler(self.ffi.data, name.into_ffi(), callback) }
    }

    fn set_bridge_origins(&self, policy: waterui_webview::OriginPolicy) {
        // Enforced natively by the backend: it is the only side that can
        // authenticate the frame a call arrived from. The policy is handed over
        // as its URI patterns, which is what the platform filters accept.
        let Some(set) = self.ffi.set_bridge_origins else {
            return;
        };
        // The wire form keeps deny-all (`""`) distinct from allow-all (`"*"`).
        // Collapsing the two is what made a view opened at a `file:` URL — whose
        // default policy admits nothing, because a local document has no origin
        // to compare — hand every handler to whatever it navigated to next.
        let patterns = policy.wire();
        // SAFETY: `set` and `ffi.data` come from the same registration.
        // SAFETY: as for `add_handler` — same controller, same registration.
        unsafe { set(self.ffi.data, patterns.into_ffi()) }
    }

    fn remove_handler(&self, name: &str) {
        let remove_handler = self
            .ffi
            .remove_handler
            .expect("WebView backend must implement `remove_handler`");

        let name = Str::from(name.to_string());
        // SAFETY: as for `add_handler` — same controller, same registration.
        unsafe { remove_handler(self.ffi.data, name.into_ffi()) }
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let set_cookie = self
            .ffi
            .set_cookie
            .expect("WebView backend must implement `set_cookie`");
        let cookie = Str::from(cookie.to_string());
        // SAFETY: as for `add_handler` — same controller, same registration.
        unsafe { set_cookie(self.ffi.data, cookie.into_ffi()) }
    }

    #[expect(
        clippy::future_not_send,
        reason = "WaterUI webview bridge futures resolve on the main-thread local executor and carry non-`Send` `Str` payloads by design"
    )]
    fn get_cookies(&self) -> impl core::future::Future<Output = Vec<Cookie<'static>>> {
        unsafe extern "C" fn cookies_callback(data: *mut (), result: WuiStr) {
            // SAFETY: `data` is the sender this callback was registered with, boxed
            // by the caller below; the backend invokes the callback once, so the box
            // is reclaimed once.
            let sender = unsafe { Box::from_raw(data.cast::<async_channel::Sender<Str>>()) };
            // SAFETY: the caller contract makes `result` an owning handle from the
            // matching FFI constructor; it is consumed here and not observed
            // again.
            let result = unsafe { result.into_rust() };
            let _ = sender.try_send(result);
        }

        let get_cookies = self
            .ffi
            .get_cookies
            .expect("WebView backend must implement `get_cookies`");
        let (sender, receiver) = async_channel::bounded::<Str>(1);
        let callback_data = Box::into_raw(Box::new(sender)).cast::<()>();

        // SAFETY: `get_cookies` and `ffi.data` come from the same registration, and
        // the callback below owns the boxed sender it will reclaim.
        unsafe {
            get_cookies(
                self.ffi.data.cast_const(),
                WuiStringCallback {
                    data: callback_data,
                    call: cookies_callback,
                },
            );
        }

        async move {
            // A web view torn down while the query was in flight drops the
            // callback without invoking it; that is a teardown race, not a
            // reason to suspend the caller's task forever.
            let Ok(text) = receiver.recv().await else {
                return Vec::new();
            };
            text.as_str()
                .lines()
                .filter_map(|line| match Cookie::parse(line.to_string()) {
                    Ok(cookie) => Some(cookie.into_owned()),
                    Err(error) => {
                        // The store holds whatever the pages put there, so one
                        // unparseable entry must not take down the app inside a
                        // getter.
                        tracing::warn!(%error, "skipping a cookie the web view could not parse");
                        None
                    }
                })
                .collect()
        }
    }

    #[expect(
        clippy::future_not_send,
        reason = "WaterUI webview bridge futures resolve on the main-thread local executor and carry non-`Send` `Str` payloads by design"
    )]
    fn run_javascript(&self, script: &str) -> impl core::future::Future<Output = Result<Str, Str>> {
        // SAFETY: the two come from the same registration; `evaluate_through`
        // documents what it needs of them.
        unsafe { self.evaluate_through(self.ffi.run_javascript, script) }
    }

    #[expect(
        clippy::future_not_send,
        reason = "WaterUI webview bridge futures resolve on the main-thread local executor and carry non-`Send` `Str` payloads by design"
    )]
    fn call_async_javascript(
        &self,
        body: &str,
    ) -> impl core::future::Future<Output = Result<Str, Str>> {
        // SAFETY: as above.
        unsafe { self.evaluate_through(self.ffi.call_async_javascript, body) }
    }
}

impl FfiWebViewHandle {
    /// Drives one of the two evaluation entry points, which differ only in
    /// whether the backend awaits the promise the source produces.
    ///
    /// # Safety
    ///
    /// `evaluate` must be a function pointer from the same registration as
    /// `self.ffi.data`.
    #[expect(
        clippy::future_not_send,
        reason = "WaterUI webview bridge futures resolve on the main-thread local executor and carry non-`Send` `Str` payloads by design"
    )]
    unsafe fn evaluate_through(
        &self,
        evaluate: unsafe extern "C" fn(*mut (), WuiStr, WuiJsCallback),
        source: &str,
    ) -> impl core::future::Future<Output = Result<Str, Str>> {
        unsafe extern "C" fn js_callback_trampoline(data: *mut (), success: bool, result: WuiStr) {
            let sender =
                // SAFETY: as for the cookie callback — `data` is the boxed sender
                // this callback was registered with, invoked once.
                unsafe { Box::from_raw(data.cast::<async_channel::Sender<Result<Str, Str>>>()) };
            // SAFETY: the caller contract makes `result` an owning handle from the
            // matching FFI constructor; it is consumed here and not observed
            // again.
            let result = unsafe { result.into_rust() };
            let _ = sender.try_send(if success { Ok(result) } else { Err(result) });
        }

        let (sender, receiver) = async_channel::bounded::<Result<Str, Str>>(1);
        let callback_data = Box::into_raw(Box::new(sender)).cast::<()>();

        let ffi_callback = WuiJsCallback {
            data: callback_data,
            call: js_callback_trampoline,
        };

        let owned_source = Str::from(source.to_string());
        unsafe {
            // SAFETY: the caller guarantees `evaluate` and `ffi.data` come from
            // the same registration, and that controller is alive for as long as
            // `self` is.
            evaluate(self.ffi.data, owned_source.into_ffi(), ffi_callback);
        }

        async move {
            // A backend that is torn down mid-evaluation drops the callback
            // without invoking it. Reporting that as an error keeps the caller's
            // future resolving: awaiting a channel that will never receive left
            // the task suspended forever, holding everything it had captured.
            receiver.recv().await.unwrap_or_else(|_| {
                Err(Str::from_static(
                    "the web view was torn down before the script completed",
                ))
            })
        }
    }
}

// =============================================================================
// WebView Raw View
// =============================================================================

// =============================================================================
// Shared JavaScript bridge
// =============================================================================

/// One parsed `waterui.invoke(...)` request.
#[repr(C)]
#[derive(Debug)]
pub struct WuiBridgeRequest {
    /// Whether the envelope parsed. When false every other field is empty and the
    /// call must be ignored.
    pub ok: bool,
    /// Correlates the reply with the page's pending promise.
    pub id: u64,
    /// The handler name the page asked for.
    pub name: WuiStr,
    /// The payload, base64-encoded to match `WuiWebViewMessage`.
    pub payload_base64: WuiStr,
}

/// Returns the bridge script every backend injects at document start.
///
/// Backends that route messages themselves — the Apple backend keeps its handler
/// table in Swift — use this together with
/// [`waterui_webview_parse_bridge_request`] and
/// [`waterui_webview_bridge_reply_script`], so the envelope format stays defined
/// only in `waterui_webview::bridge`.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_webview_bridge_script() -> WuiStr {
    Str::from_static(waterui_webview::DOCUMENT_START_SCRIPT).into_ffi()
}

/// Parses one envelope produced by the bridge script.
///
/// # Safety
///
/// `envelope` must be an owning `WuiStr`; it is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_webview_parse_bridge_request(
    envelope: WuiStr,
) -> WuiBridgeRequest {
    // SAFETY: the caller contract makes `envelope` an owning handle from the
    // matching FFI constructor; it is consumed here and not observed again.
    let envelope: Str = unsafe { envelope.into_rust() };
    match waterui_webview::bridge::Request::parse(envelope.as_str()) {
        Ok(request) => WuiBridgeRequest {
            ok: true,
            id: request.id,
            name: Str::from(request.name).into_ffi(),
            payload_base64: Str::from(
                base64::engine::general_purpose::STANDARD.encode(&request.payload),
            )
            .into_ffi(),
        },
        Err(error) => {
            tracing::warn!(%error, "page script sent a malformed WaterUI bridge request");
            WuiBridgeRequest {
                ok: false,
                id: 0,
                name: Str::from_static("").into_ffi(),
                payload_base64: Str::from_static("").into_ffi(),
            }
        }
    }
}

/// Renders the JavaScript that settles one bridge call.
///
/// # Safety
///
/// `payload_base64` must be an owning `WuiStr`; it is consumed. On success it is
/// base64 handler output and `kind` says whether those bytes are a JSON value or
/// opaque; otherwise it is an error message and `kind` is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_webview_bridge_reply_script(
    id: u64,
    success: bool,
    kind: WuiJsReplyKind,
    payload_base64: WuiStr,
) -> WuiStr {
    // SAFETY: the caller contract makes `payload_base64` an owning handle from the
    // matching FFI constructor; it is consumed here and not observed again.
    let payload: Str = unsafe { payload_base64.into_rust() };
    let reply = if success {
        match base64::engine::general_purpose::STANDARD.decode(payload.as_str()) {
            Ok(bytes) => match kind {
                WuiJsReplyKind::Json => waterui_webview::bridge::Reply::Json(bytes),
                WuiJsReplyKind::Bytes => waterui_webview::bridge::Reply::Bytes(bytes),
            },
            Err(error) => waterui_webview::bridge::Reply::failure(&error),
        }
    } else {
        waterui_webview::bridge::Reply::failure(&payload.as_str())
    };
    Str::from(reply.resolve_script(id)).into_ffi()
}

opaque!(WuiWebView, WebView);
ffi_view!(WebView, *mut WuiWebView, webview, any());

/// Gets the native handle pointer from a `WebView`.
///
/// Returns the opaque pointer to the native `WebView` wrapper (Swift/Kotlin).
/// This pointer can be used by native backends to access the underlying
/// `WKWebView` or Android `WebView`.
///
/// # Safety
///
/// - The caller must ensure that `webview` is a valid pointer to a `WuiWebView`.
/// - The `WebView` must have been created via the FFI `WebViewController` (i.e., the handle
///   must be an `FfiWebViewHandle`). This is guaranteed when the native backend properly
///   installed the `WebViewController` via `waterui_env_install_webview_controller`.
///
/// # Panics
///
/// Panics if the `WebView`'s handle was not created via the FFI `WebViewController`
/// (i.e., it does not downcast to `FfiWebViewHandle`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_webview_native_handle(webview: *mut WuiWebView) -> *mut () {
    // SAFETY: the caller contract requires `webview` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    unsafe {
        let webview = crate::borrow_ffi(webview);
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

/// Type for the native function that creates a new `WebView`.
pub type WuiCreateWebViewFn = unsafe extern "C" fn() -> WuiWebViewHandle;

/// FFI-compatible `WebViewController` implementation.
struct FfiWebViewController {
    create_fn: WuiCreateWebViewFn,
}

impl CustomWebViewController for FfiWebViewController {
    fn open(&self) -> impl WebViewHandle {
        // SAFETY: `create_fn` is the backend's constructor, registered on this
        // factory; it takes no arguments and hands back an owning handle.
        let handle = unsafe { (self.create_fn)() };
        FfiWebViewHandle::new(handle)
    }
}

/// Installs a `WebViewController` into the environment from a native factory function.
///
/// Native backends call this during initialization to register their `WebView` factory.
/// The factory creates blank `WebViews` that can be navigated with `go_to()`.
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
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };

    let controller = WebViewController::new(FfiWebViewController { create_fn });
    env.insert(controller);
}

/// Returns whether a `WebView` controller is already installed in the environment.
///
/// # Safety
///
/// `env` must be a valid pointer to a live `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_has_webview_controller(env: *const WuiEnv) -> bool {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    env.get::<WebViewController>().is_some()
}
