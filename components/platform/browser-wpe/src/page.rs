//! # Safety
//!
//! Every `unsafe` in this module is a call through the WPE bridge ABI, and they
//! share one justification, so the per-site comments state only what is specific
//! to each.
//!
//! The function pointers come from `RuntimeApi`, resolved once from the bridge
//! library, which an `Arc` keeps mapped for as long as any page exists. The page
//! pointer passed to them is the one this type owns and frees exactly once in its
//! `Drop`. The bridge marshals the underlying WPE object operations onto the
//! runtime's `GMainContext`, so these calls carry no thread affinity of their own.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use futures::channel::oneshot;
use num_traits::ToPrimitive as _;
use waterui_str::Str;
use waterui_url::Url;
use waterui_webview::{BackendEvent, WatcherSet, WebViewError, WebViewEvent, bridge};
use wgpu_external_frame::dma_buf::DmaBufFrame;

use crate::abi::{WaterWpeBytes, WaterWpeFrame, WaterWpePage};
use crate::frame::frame_from_abi;
use crate::runtime::{RuntimeApi, WpeRuntime, take_error};

const EVENT_NAVIGATION_STARTED: u32 = 1;
const EVENT_LOADING: u32 = 2;
const EVENT_LOADED: u32 = 3;
const EVENT_REDIRECT: u32 = 4;
const EVENT_LOAD_FAILED: u32 = 5;
const EVENT_TLS_FAILED: u32 = 6;
const EVENT_HISTORY_CHANGED: u32 = 7;

type MessageHandler = Rc<waterui_webview::ScriptMessageHandler>;

struct PageState {
    api: std::sync::Arc<RuntimeApi>,
    watchers: WatcherSet<BackendEvent>,
    handlers: RefCell<HashMap<String, MessageHandler>>,
    frame: RefCell<Option<DmaBufFrame>>,
    frame_waker: RefCell<Option<Rc<dyn Fn()>>>,
    size: Cell<(u32, u32, f64)>,
    /// Which documents may reach the bridge; checked on every message.
    bridge_origins: RefCell<Option<waterui_webview::OriginPolicy>>,
}

impl PageState {
    fn emit(&self, event: impl Into<BackendEvent>) {
        self.watchers.emit(&event.into());
    }
}

struct ClientContext {
    state: Weak<PageState>,
    /// The page this context belongs to, so an asynchronous handler can settle
    /// the page's promise after the transport has already returned.
    ///
    /// Weak, and upgraded rather than dereferenced: a handler may await for as
    /// long as it likes, and the page can be dropped — and its native page freed
    /// — while it does. Holding the raw pointer across the await called into
    /// freed memory; holding the strong reference the upgrade produces keeps the
    /// page alive for exactly the length of the reply.
    page: RefCell<Weak<PageInner>>,
}

struct PageInner {
    runtime: WpeRuntime,
    raw: NonNull<WaterWpePage>,
    state: Rc<PageState>,
}

impl Drop for PageInner {
    fn drop(&mut self) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.runtime.api().api.page_free)(self.raw.as_ptr()) };
    }
}

/// Pointer button understood by `WPEPlatform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    /// Primary or left button.
    Primary = 1,
    /// Middle button.
    Middle = 2,
    /// Secondary or right button.
    Secondary = 3,
    /// Back navigation button.
    Back = 4,
    /// Forward navigation button.
    Forward = 5,
}

/// One offscreen WPE `WebKit` page.
#[derive(Clone)]
pub struct WpePage {
    inner: Rc<PageInner>,
}

impl core::fmt::Debug for WpePage {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("WpePage").finish_non_exhaustive()
    }
}

impl WpePage {
    /// Opens a blank page in `runtime`.
    ///
    /// # Panics
    ///
    /// Panics when the runtime cannot create a page.
    #[must_use]
    pub fn new(runtime: WpeRuntime) -> Self {
        let state = Rc::new(PageState {
            api: std::sync::Arc::clone(runtime.api()),
            watchers: WatcherSet::new(),
            handlers: RefCell::new(HashMap::new()),
            frame: RefCell::new(None),
            frame_waker: RefCell::new(None),
            size: Cell::new((1, 1, 1.0)),
            bridge_origins: RefCell::new(None),
        });
        // The page is filled in below, once it exists.
        let context = Box::new(ClientContext {
            state: Rc::downgrade(&state),
            page: RefCell::new(Weak::new()),
        });
        let context_ptr = Box::into_raw(context);
        let mut error = std::ptr::null_mut::<c_char>();
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        let raw = unsafe {
            (runtime.api().api.page_new)(
                runtime.raw().as_ptr(),
                event_callback,
                frame_callback,
                message_callback,
                context_ptr.cast(),
                destroy_client_context,
                &raw mut error,
            )
        };
        let raw = NonNull::new(raw).unwrap_or_else(|| {
            let message = take_error(runtime.api(), error);
            panic!("failed to create bundled WPE page: {message}")
        });
        assert!(
            error.is_null(),
            "WPE page creation succeeded while also returning an error"
        );
        let page = Self {
            inner: Rc::new(PageInner {
                runtime,
                raw,
                state,
            }),
        };
        // The client context now knows its page, so an asynchronous handler can
        // settle the page's promise after the transport has returned — and can
        // tell whether the page is still there when it does.
        // SAFETY: `page_new` kept the context pointer and has not freed it; this
        // is the only write, before any callback can run.
        unsafe {
            *(*context_ptr).page.borrow_mut() = Rc::downgrade(&page.inner);
        }
        page
    }

    fn string(value: &str, context: &str) -> CString {
        CString::new(value).unwrap_or_else(|_| panic!("{context} contains an interior NUL byte"))
    }

    /// Drains WPE's ready main-loop work.
    pub fn pump(&self) {
        while self.inner.runtime.iteration() {}
    }

    /// Loads `url`.
    ///
    /// # Panics
    ///
    /// Panics when `url` is invalid or contains an interior NUL byte.
    pub fn load_uri(&self, url: &str) {
        let _: Url = url
            .parse()
            .unwrap_or_else(|error| panic!("WPE WebView received an invalid URL: {error}"));
        let url = Self::string(url, "WPE URL");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_load_uri)(self.inner.raw.as_ptr(), url.as_ptr()) };
    }

    /// Navigates backward.
    pub fn go_back(&self) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_go_back)(self.inner.raw.as_ptr()) };
    }

    /// Navigates forward.
    pub fn go_forward(&self) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_go_forward)(self.inner.raw.as_ptr()) };
    }

    /// Stops loading.
    pub fn stop(&self) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_stop)(self.inner.raw.as_ptr()) };
    }

    /// Reloads the current page.
    pub fn reload(&self) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_reload)(self.inner.raw.as_ptr()) };
    }

    /// Returns whether history has an earlier item.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_can_go_back)(self.inner.raw.as_ptr()) }
    }

    /// Returns whether history has a later item.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_can_go_forward)(self.inner.raw.as_ptr()) }
    }

    /// Enables or rejects redirects.
    pub fn set_redirects_enabled(&self, enabled: bool) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_set_redirects_enabled)(self.inner.raw.as_ptr(), enabled);
        };
    }

    /// Overrides the user agent.
    pub fn set_user_agent(&self, user_agent: &str) {
        let user_agent = Self::string(user_agent, "WPE user agent");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_set_user_agent)(
                self.inner.raw.as_ptr(),
                user_agent.as_ptr(),
            );
        };
    }

    /// Updates the WPE viewport in physical pixels.
    ///
    /// # Panics
    ///
    /// Panics when the dimensions are zero or `scale` is not positive and
    /// finite.
    pub fn resize(&self, width: u32, height: u32, scale: f64) {
        assert!(width > 0 && height > 0, "WPE viewport must be non-zero");
        assert!(
            scale.is_finite() && scale > 0.0,
            "WPE scale must be positive and finite"
        );
        if self.inner.state.size.replace((width, height, scale)) == (width, height, scale) {
            return;
        }
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_resize)(self.inner.raw.as_ptr(), width, height, scale);
        };
    }

    /// Updates keyboard focus.
    pub fn set_focus(&self, focused: bool) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { (self.inner.state.api.api.page_set_focus)(self.inner.raw.as_ptr(), focused) };
    }

    /// Sends a pointer button event.
    pub fn pointer_button(
        &self,
        pressed: bool,
        button: PointerButton,
        x: f64,
        y: f64,
        modifiers: u32,
        time_ms: u32,
    ) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_pointer_button)(
                self.inner.raw.as_ptr(),
                pressed,
                button as u32,
                x,
                y,
                modifiers,
                time_ms,
            );
        };
    }

    /// Sends a pointer movement event.
    pub fn pointer_move(
        &self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        modifiers: u32,
        time_ms: u32,
    ) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_pointer_move)(
                self.inner.raw.as_ptr(),
                x,
                y,
                delta_x,
                delta_y,
                modifiers,
                time_ms,
            );
        };
    }

    /// Sends a scroll event.
    #[expect(
        clippy::too_many_arguments,
        reason = "WPE scroll event payload is atomic"
    )]
    pub fn scroll(
        &self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        precise: bool,
        stopped: bool,
        modifiers: u32,
        time_ms: u32,
    ) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_scroll)(
                self.inner.raw.as_ptr(),
                x,
                y,
                delta_x,
                delta_y,
                precise,
                stopped,
                modifiers,
                time_ms,
            );
        };
    }

    /// Sends a keyboard event with Linux evdev keycode and XKB keysym.
    pub fn key(&self, pressed: bool, keycode: u32, keyval: u32, modifiers: u32, time_ms: u32) {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_key)(
                self.inner.raw.as_ptr(),
                pressed,
                keycode,
                keyval,
                modifiers,
                time_ms,
            );
        };
    }

    /// Watches semantic `WebView` events.
    pub fn watch(&self, watcher: impl Fn(BackendEvent) + 'static) -> waterui_webview::WatcherGuard {
        self.inner.state.watchers.insert(watcher)
    }

    /// Installs a callback used to wake the host renderer when WPE submits a frame.
    pub fn set_frame_waker(&self, waker: impl Fn() + 'static) {
        self.inner.state.frame_waker.replace(Some(Rc::new(waker)));
    }

    /// Takes the newest frame. Superseded frames are returned to WPE immediately.
    #[must_use]
    pub fn take_frame(&self) -> Option<DmaBufFrame> {
        self.inner.state.frame.borrow_mut().take()
    }

    /// Installs a document script under `key`, replacing whatever script that
    /// key already names.
    ///
    /// # Panics
    ///
    /// Panics when `key` or `script` contains an interior NUL byte.
    pub fn add_script(&self, key: &str, script: &str, at_document_end: bool) {
        let key = Self::string(key, "WPE script key");
        let script = Self::string(script, "WPE script");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_add_script)(
                self.inner.raw.as_ptr(),
                key.as_ptr(),
                script.as_ptr(),
                u32::from(at_document_end),
            );
        };
    }

    /// Chooses which documents may reach the bridge.
    pub fn set_bridge_origins(&self, policy: waterui_webview::OriginPolicy) {
        self.inner.state.bridge_origins.replace(Some(policy));
    }

    /// Registers a JavaScript message handler.
    ///
    /// # Panics
    ///
    /// Panics when `name` is empty, contains an interior NUL byte, or is
    /// already registered.
    pub fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        assert!(!name.is_empty(), "WPE handler name must not be empty");
        // One transport serves every handler name, so registration is bookkeeping
        // only. Re-registering replaces, matching every other backend.
        self.inner
            .state
            .handlers
            .borrow_mut()
            .insert(name.to_owned(), Rc::from(handler));
    }

    /// Removes a JavaScript message handler.
    ///
    /// # Panics
    ///
    /// Panics when `name` is not registered or contains an interior NUL byte.
    pub fn remove_handler(&self, name: &str) {
        // Removing a name that was never registered is a no-op, matching every
        // other backend.
        self.inner.state.handlers.borrow_mut().remove(name);
    }

    /// Adds a serialized Set-Cookie header for the current origin.
    pub fn set_cookie(&self, cookie: &str) {
        let cookie = Self::string(cookie, "WPE cookie");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe {
            (self.inner.state.api.api.page_set_cookie)(self.inner.raw.as_ptr(), cookie.as_ptr());
        };
    }

    /// Retrieves the runtime's JSON cookie array.
    ///
    /// # Panics
    ///
    /// Panics when the runtime cancels or rejects the cookie query.
    #[expect(
        clippy::future_not_send,
        reason = "WPE WebKit pages are confined to the UI thread"
    )]
    pub async fn cookies_json(&self) -> String {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        self.string_result(|callback, context| unsafe {
            (self.inner.state.api.api.page_get_cookies)(self.inner.raw.as_ptr(), callback, context);
        })
        .await
        .unwrap_or_else(|error| panic!("WPE cookie query failed: {error}"))
    }

    /// Evaluates JavaScript and returns its serialized result.
    ///
    /// # Errors
    ///
    /// Returns the serialized JavaScript exception when evaluation fails.
    #[expect(
        clippy::future_not_send,
        reason = "WPE WebKit pages are confined to the UI thread"
    )]
    pub async fn run_javascript(&self, script: &str) -> Result<Str, Str> {
        let script = Self::string(script, "WPE JavaScript");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        self.string_result(|callback, context| unsafe {
            (self.inner.state.api.api.page_run_javascript)(
                self.inner.raw.as_ptr(),
                script.as_ptr(),
                callback,
                context,
            );
        })
        .await
        .map(Str::from)
        .map_err(Str::from)
    }

    /// Runs `body` as the body of an async function and resolves with the value
    /// its promise settles on.
    ///
    /// # Errors
    ///
    /// Returns the serialized JavaScript exception when the promise rejects or
    /// the body throws.
    #[expect(
        clippy::future_not_send,
        reason = "WPE WebKit pages are confined to the UI thread"
    )]
    pub async fn call_async_javascript(&self, body: &str) -> Result<Str, Str> {
        let body = Self::string(body, "WPE async JavaScript body");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        self.string_result(|callback, context| unsafe {
            (self.inner.state.api.api.page_call_async_javascript)(
                self.inner.raw.as_ptr(),
                body.as_ptr(),
                callback,
                context,
            );
        })
        .await
        .map(Str::from)
        .map_err(Str::from)
    }

    #[expect(
        clippy::future_not_send,
        reason = "WPE WebKit pages and their callbacks are confined to the UI thread"
    )]
    async fn string_result(
        &self,
        start: impl FnOnce(crate::abi::ResultCallback, *mut c_void),
    ) -> Result<String, String> {
        let (sender, receiver) = oneshot::channel();
        start(
            string_result_callback,
            Box::into_raw(Box::new(sender)).cast(),
        );
        receiver
            .await
            .expect("bundled WPE result callback was cancelled")
    }
}

unsafe extern "C" fn destroy_client_context(context: *mut c_void) {
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    unsafe { drop(Box::from_raw(context.cast::<ClientContext>())) };
}

unsafe extern "C" fn event_callback(
    context: *mut c_void,
    kind: u32,
    first: *const c_char,
    second: *const c_char,
    number: f64,
) {
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    let context = unsafe { &*context.cast::<ClientContext>() };
    let Some(state) = context.state.upgrade() else {
        return;
    };
    let text = |value: *const c_char| {
        assert!(!value.is_null(), "WPE event string is null");
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    };
    match kind {
        EVENT_NAVIGATION_STARTED => state.emit(WebViewEvent::WillNavigate {
            url: parse_url(&text(first)),
        }),
        EVENT_LOADING => state.emit(WebViewEvent::Loading {
            progress: number
                .to_f32()
                .expect("WPE loading progress does not fit f32"),
        }),
        EVENT_LOADED => state.emit(WebViewEvent::Loaded),
        EVENT_REDIRECT => state.emit(WebViewEvent::Redirect {
            from: parse_url(&text(first)),
            to: parse_url(&text(second)),
        }),
        EVENT_LOAD_FAILED => state.emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
            text(first),
        )))),
        EVENT_TLS_FAILED => state.emit(WebViewEvent::Error(WebViewError::Ssl {
            url: parse_url(&text(first)),
            message: Str::from(text(second)),
        })),
        EVENT_HISTORY_CHANGED => {
            let history = number.to_u32().expect("WPE history flags do not fit u32");
            state.emit(BackendEvent::NavigationState {
                can_go_back: history & 1 != 0,
                can_go_forward: history & 2 != 0,
            });
        }
        _ => panic!("bundled WPE returned unknown event kind {kind}"),
    }
}

unsafe extern "C" fn frame_callback(context: *mut c_void, frame: *const WaterWpeFrame) {
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    let context = unsafe { &*context.cast::<ClientContext>() };
    let Some(state) = context.state.upgrade() else {
        return;
    };
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    let frame = unsafe {
        frame_from_abi(
            std::sync::Arc::clone(&state.api),
            frame.as_ref().expect("WPE frame callback received null"),
        )
    };
    state.frame.replace(Some(frame));
    if let Some(waker) = state.frame_waker.borrow().as_ref() {
        waker();
    }
}

struct ResponseBytes {
    bytes: Vec<u8>,
}

unsafe extern "C" fn destroy_response(context: *mut c_void) {
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    unsafe { drop(Box::from_raw(context.cast::<ResponseBytes>())) };
}

unsafe extern "C" fn message_callback(
    context: *mut c_void,
    origin: *const c_char,
    envelope: *const c_char,
) -> WaterWpeBytes {
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    let context = unsafe { &*context.cast::<ClientContext>() };
    let state = context
        .state
        .upgrade()
        .expect("WPE invoked a message after page destruction");
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    let envelope = unsafe { CStr::from_ptr(envelope) };
    // SAFETY: bridge ABI call on the page this type owns; see the module safety
    // note.
    let origin = unsafe { CStr::from_ptr(origin) };

    // Page script reaches this transport directly, so a malformed envelope or an
    // unknown handler name is rejected back to JavaScript rather than being fatal.
    let script = match envelope
        .to_str()
        .map_err(|_| String::from("envelope is not UTF-8"))
        .and_then(|envelope| bridge::Request::parse(envelope).map_err(|error| error.to_string()))
    {
        Ok(request) => {
            // Every registered handler is a capability, and `webkit.messageHandlers`
            // is reachable from any frame of the page, so the document that sent
            // this has to be authenticated before anything is dispatched. No policy
            // means no handler has been registered either, and denying is what the
            // other backends do.
            let allowed = origin.to_str().is_ok_and(|origin| {
                state
                    .bridge_origins
                    .borrow()
                    .as_ref()
                    .is_some_and(|policy| policy.allows_origin(origin))
            });
            if !allowed {
                tracing::warn!(
                    origin = %origin.to_string_lossy(),
                    handler = %request.name,
                    "a document outside the bridge origin policy tried to call a WaterUI handler"
                );
                // Reject rather than drop: the page is awaiting a promise, and one
                // that never settles is indistinguishable from a handler that
                // hangs. Like every reply, it is evaluated in the top frame, which
                // is where the bridge is injected and therefore where the pending
                // promise lives.
                let reply = bridge::Reply::failure(
                    "this document is not allowed to use the WaterUI bridge",
                );
                return respond(reply.resolve_script(request.id));
            }

            // Release the borrow before invoking: a handler may register or remove
            // handlers on the same page.
            let handler = state.handlers.borrow().get(&request.name).map(Rc::clone);
            let Some(handler) = handler else {
                tracing::warn!(
                    handler = %request.name,
                    "page script called a WaterUI handler that is not registered"
                );
                let reply =
                    bridge::Reply::failure(&format!("no WaterUI handler named `{}`", request.name));
                return respond(reply.resolve_script(request.id));
            };

            // Handlers are asynchronous, so nothing is returned now; the promise
            // settles when the future completes.
            let future = handler(&request.payload);
            let page = context.page.borrow().clone();
            executor_core::spawn_local(async move {
                let reply = match future.await {
                    Ok(reply) => bridge::Reply::from(reply),
                    Err(message) => bridge::Reply::Failure(message),
                };
                // The await may have outlived the web view. Upgrading answers
                // that, and the strong reference it yields keeps the native page
                // alive for the call: nothing can free it while this holds one.
                let Some(page) = page.upgrade() else {
                    return;
                };
                let script = CString::new(reply.resolve_script(request.id))
                    .expect("a reply script never contains NUL");
                // SAFETY: `page` is a live `PageInner`, which owns this pointer and
                // frees it only in its own `Drop`; see the module safety note.
                unsafe {
                    (page.state.api.api.page_evaluate)(page.raw.as_ptr(), script.as_ptr());
                }
            })
            .detach();
            String::new()
        }
        Err(error) => {
            tracing::warn!(%error, "page script sent a malformed WaterUI bridge request");
            // Without a usable id there is no pending promise to settle.
            String::new()
        }
    };

    respond(script)
}

/// Hands one script back across the C ABI, transferring ownership of its buffer.
fn respond(script: String) -> WaterWpeBytes {
    let mut response = Box::new(ResponseBytes {
        bytes: script.into_bytes(),
    });
    let payload = WaterWpeBytes {
        data: response.bytes.as_ptr(),
        len: response.bytes.len(),
        user_data: (&raw mut *response).cast(),
        destroy: Some(destroy_response),
    };
    let _ = Box::into_raw(response);
    payload
}

unsafe extern "C" fn string_result_callback(
    context: *mut c_void,
    success: bool,
    data: *const c_char,
    len: usize,
) {
    let sender =
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { Box::from_raw(context.cast::<oneshot::Sender<Result<String, String>>>()) };
    assert!(
        !data.is_null() || len == 0,
        "WPE result data is null with non-zero length"
    );
    let bytes = if len == 0 {
        &[][..]
    } else {
        // SAFETY: bridge ABI call on the page this type owns; see the module safety
        // note.
        unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) }
    };
    let value =
        String::from_utf8(bytes.to_vec()).expect("WPE result callback returned invalid UTF-8");
    let _ = sender.send(if success { Ok(value) } else { Err(value) });
}

fn parse_url(value: &str) -> Url {
    value
        .parse()
        .unwrap_or_else(|error| panic!("WPE emitted an invalid URL: {error}"))
}
