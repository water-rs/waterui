#![cfg_attr(
    not(all(
        feature = "webkitgtk",
        gtk_webkitgtk_link_available,
        unix,
        not(target_os = "macos")
    )),
    allow(dead_code, unused_imports)
)]

//! System `WebKitGTK` implementation selected by `webview-system`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Computed, Environment, Signal, Str};
use waterui_webview::{
    BackendEvent, Cookie, CustomWebViewController, ScriptInjectionTime, Url, WatcherGuard,
    WatcherSet, WebViewController, WebViewError, WebViewEvent, WebViewHandle, bridge,
};

type JsHandler = Rc<waterui_webview::ScriptMessageHandler>;

/// What the handle's `impl Future` methods return.
///
/// Boxed because the configuration with no linkable WebKitGTK produces no future
/// at all — it fast-fails — and the two arms still have to name one type.
type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T>>>;

const WEBKIT_FEATURE_MSG: &str = "WebView requires waterui-gtk feature `webkitgtk` and linkable WebKitGTK 6 libraries on Linux (fast-fail: no placeholder backend)";

/// Adapts the shared bridge's one-function transport onto `WebKitGTK`'s message handler.
const TRANSPORT_SCRIPT: &str = concat!(
    "globalThis.__wateruiSend = function (envelope) {",
    "window.webkit.messageHandlers.__wateruiSend.postMessage(envelope);",
    "};"
);

struct SharedState {
    watchers: WatcherSet<BackendEvent>,
    redirects_enabled: RefCell<Computed<bool>>,
    handler_callbacks: RefCell<HashMap<String, JsHandler>>,
    /// Which documents may reach the bridge; checked on every message.
    bridge_origins: RefCell<Option<waterui_webview::OriginPolicy>>,
}

impl SharedState {
    fn emit(&self, event: impl Into<BackendEvent>) {
        self.watchers.emit(&event.into());
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            watchers: WatcherSet::new(),
            redirects_enabled: RefCell::new(Computed::new(true)),
            handler_callbacks: RefCell::new(HashMap::new()),
            bridge_origins: RefCell::new(None),
        }
    }
}

/// One script injected through [`WebViewHandle::inject_script`], kept with the
/// key it was injected under so re-injecting replaces it in place instead of
/// stacking another, staler copy behind it.
#[derive(Debug, Clone)]
struct InjectedScript {
    key: String,
    source: String,
    time: ScriptInjectionTime,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
mod webkitgtk {
    use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
    use std::os::raw::c_ulong;
    use std::ptr::NonNull;

    use gtk4::Widget;
    use gtk4::ffi::GtkWidget;
    use gtk4::gio;
    use gtk4::glib;
    use gtk4::glib::translate::{ToGlibPtr, from_glib_none};
    use waterui_core::Str;

    use super::ScriptInjectionTime;

    #[repr(C)]
    pub struct WebKitWebView {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitUserContentManager {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitUserScript {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitPolicyDecision {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitResponsePolicyDecision {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitURIResponse {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitWebsiteDataManager {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitCookieManager {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct SoupCookie {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct SoupMessageHeaders {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct JSCValue {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct WebKitSecurityOrigin {
        _private: [u8; 0],
    }

    pub type WebKitPolicyDecisionType = c_int;
    pub const WEBKIT_POLICY_DECISION_TYPE_RESPONSE: WebKitPolicyDecisionType = 2;

    pub type WebKitUserContentInjectedFrames = c_int;
    pub const WEBKIT_USER_CONTENT_INJECT_TOP_FRAME: WebKitUserContentInjectedFrames = 0;

    pub type WebKitUserScriptInjectionTime = c_int;
    pub const WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START: WebKitUserScriptInjectionTime = 0;
    pub const WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_END: WebKitUserScriptInjectionTime = 1;

    #[link(name = "webkitgtk-6.0")]
    unsafe extern "C" {
        fn webkit_web_view_new() -> *mut WebKitWebView;
        fn webkit_web_view_load_uri(web_view: *mut WebKitWebView, uri: *const c_char);
        fn webkit_web_view_go_back(web_view: *mut WebKitWebView);
        fn webkit_web_view_go_forward(web_view: *mut WebKitWebView);
        fn webkit_web_view_stop_loading(web_view: *mut WebKitWebView);
        fn webkit_web_view_reload(web_view: *mut WebKitWebView);
        fn webkit_web_view_can_go_back(web_view: *mut WebKitWebView) -> glib::ffi::gboolean;
        fn webkit_web_view_can_go_forward(web_view: *mut WebKitWebView) -> glib::ffi::gboolean;
        fn webkit_web_view_get_user_content_manager(
            web_view: *mut WebKitWebView,
        ) -> *mut WebKitUserContentManager;
        fn webkit_web_view_set_custom_user_agent(
            web_view: *mut WebKitWebView,
            user_agent: *const c_char,
        );
        fn webkit_web_view_get_uri(web_view: *mut WebKitWebView) -> *const c_char;
        fn webkit_web_view_get_website_data_manager(
            web_view: *mut WebKitWebView,
        ) -> *mut WebKitWebsiteDataManager;
        fn webkit_web_view_evaluate_javascript(
            web_view: *mut WebKitWebView,
            script: *const c_char,
            length: isize,
            world_name: *const c_char,
            source_uri: *const c_char,
            cancellable: *mut gio::ffi::GCancellable,
            callback: gio::ffi::GAsyncReadyCallback,
            user_data: *mut c_void,
        );
        fn webkit_web_view_evaluate_javascript_finish(
            web_view: *mut WebKitWebView,
            result: *mut gio::ffi::GAsyncResult,
            error: *mut *mut glib::ffi::GError,
        ) -> *mut JSCValue;
        fn webkit_web_view_call_async_javascript_function(
            web_view: *mut WebKitWebView,
            body: *const c_char,
            length: isize,
            arguments: *mut glib::ffi::GVariant,
            world_name: *const c_char,
            source_uri: *const c_char,
            cancellable: *mut gio::ffi::GCancellable,
            callback: gio::ffi::GAsyncReadyCallback,
            user_data: *mut c_void,
        );
        fn webkit_web_view_call_async_javascript_function_finish(
            web_view: *mut WebKitWebView,
            result: *mut gio::ffi::GAsyncResult,
            error: *mut *mut glib::ffi::GError,
        ) -> *mut JSCValue;
        fn webkit_security_origin_new_for_uri(uri: *const c_char) -> *mut WebKitSecurityOrigin;
        fn webkit_security_origin_to_string(origin: *mut WebKitSecurityOrigin) -> *mut c_char;
        fn webkit_security_origin_unref(origin: *mut WebKitSecurityOrigin);
        fn webkit_response_policy_decision_get_response(
            decision: *mut WebKitResponsePolicyDecision,
        ) -> *mut WebKitURIResponse;
        fn webkit_uri_response_get_status_code(response: *mut WebKitURIResponse) -> c_uint;
        fn webkit_uri_response_get_uri(response: *mut WebKitURIResponse) -> *const c_char;
        fn webkit_uri_response_get_http_headers(
            response: *mut WebKitURIResponse,
        ) -> *mut SoupMessageHeaders;
        fn webkit_policy_decision_ignore(decision: *mut WebKitPolicyDecision);
    }

    #[link(name = "webkitgtk-6.0")]
    unsafe extern "C" {
        fn webkit_user_content_manager_add_script(
            manager: *mut WebKitUserContentManager,
            script: *mut WebKitUserScript,
        );
        fn webkit_user_content_manager_remove_all_scripts(manager: *mut WebKitUserContentManager);
        fn webkit_user_content_manager_register_script_message_handler(
            manager: *mut WebKitUserContentManager,
            name: *const c_char,
            world_name: *const c_char,
        ) -> glib::ffi::gboolean;
        fn webkit_user_content_manager_unregister_script_message_handler(
            manager: *mut WebKitUserContentManager,
            name: *const c_char,
            world_name: *const c_char,
        );
        fn webkit_user_script_new(
            source: *const c_char,
            injected_frames: WebKitUserContentInjectedFrames,
            injection_time: WebKitUserScriptInjectionTime,
            allow_list: *const *const c_char,
            block_list: *const *const c_char,
        ) -> *mut WebKitUserScript;
        fn webkit_user_script_unref(script: *mut WebKitUserScript);
    }

    #[link(name = "webkitgtk-6.0")]
    unsafe extern "C" {
        fn webkit_website_data_manager_get_cookie_manager(
            data_manager: *mut WebKitWebsiteDataManager,
        ) -> *mut WebKitCookieManager;
        fn webkit_cookie_manager_add_cookie(
            manager: *mut WebKitCookieManager,
            cookie: *mut SoupCookie,
            cancellable: *mut gio::ffi::GCancellable,
            callback: gio::ffi::GAsyncReadyCallback,
            user_data: *mut c_void,
        );
        fn webkit_cookie_manager_add_cookie_finish(
            manager: *mut WebKitCookieManager,
            result: *mut gio::ffi::GAsyncResult,
            error: *mut *mut glib::ffi::GError,
        ) -> glib::ffi::gboolean;
        fn webkit_cookie_manager_get_cookies(
            manager: *mut WebKitCookieManager,
            uri: *const c_char,
            cancellable: *mut gio::ffi::GCancellable,
            callback: gio::ffi::GAsyncReadyCallback,
            user_data: *mut c_void,
        );
        fn webkit_cookie_manager_get_cookies_finish(
            manager: *mut WebKitCookieManager,
            result: *mut gio::ffi::GAsyncResult,
            error: *mut *mut glib::ffi::GError,
        ) -> *mut glib::ffi::GList;
    }

    #[link(name = "javascriptcoregtk-6.0")]
    unsafe extern "C" {
        fn jsc_value_is_string(value: *mut JSCValue) -> glib::ffi::gboolean;
        fn jsc_value_is_null(value: *mut JSCValue) -> glib::ffi::gboolean;
        fn jsc_value_is_undefined(value: *mut JSCValue) -> glib::ffi::gboolean;
        fn jsc_value_to_string(value: *mut JSCValue) -> *mut c_char;
        fn jsc_value_to_json(value: *mut JSCValue, indent: c_uint) -> *mut c_char;
    }

    #[link(name = "soup-3.0")]
    unsafe extern "C" {
        fn soup_cookie_parse(
            header: *const c_char,
            origin: *mut glib::ffi::GUri,
        ) -> *mut SoupCookie;
        fn soup_cookie_free(cookie: *mut SoupCookie);
        fn soup_cookie_to_set_cookie_header(cookie: *mut SoupCookie) -> *mut c_char;
        fn soup_message_headers_get_one(
            hdrs: *mut SoupMessageHeaders,
            name: *const c_char,
        ) -> *const c_char;
    }

    pub(super) struct WebViewParts {
        pub widget: Widget,
        pub ptr: NonNull<WebKitWebView>,
        pub manager: NonNull<WebKitUserContentManager>,
        pub cookie_manager: NonNull<WebKitCookieManager>,
    }

    pub(super) fn create_webview() -> WebViewParts {
        let ptr = NonNull::new(unsafe { webkit_web_view_new() })
            .expect("webkit_web_view_new returned null (fast-fail)");

        let manager =
            NonNull::new(unsafe { webkit_web_view_get_user_content_manager(ptr.as_ptr()) })
                .expect("webkit_web_view_get_user_content_manager returned null (fast-fail)");

        let data_manager =
            NonNull::new(unsafe { webkit_web_view_get_website_data_manager(ptr.as_ptr()) })
                .expect("webkit_web_view_get_website_data_manager returned null (fast-fail)");
        let cookie_manager = NonNull::new(unsafe {
            webkit_website_data_manager_get_cookie_manager(data_manager.as_ptr())
        })
        .expect("webkit_website_data_manager_get_cookie_manager returned null (fast-fail)");

        // `webkit_web_view_new` returns a *floating* `GInitiallyUnowned`
        // reference. `from_glib_none` is the constructor that sinks it — as every
        // gtk-rs widget constructor does — so the wrapper owns a real reference.
        // `from_glib_full` does not sink, so the first `gtk_widget_set_parent`
        // consumed the wrapper's only reference and the final unparent freed the
        // object while `NativeState` still pointed at it.
        //
        // SAFETY: `ptr` is a live `WebKitWebView`, which is a `GtkWidget`, and the
        // floating reference is handed over to the wrapper here.
        let widget: Widget = unsafe { from_glib_none(ptr.as_ptr().cast::<GtkWidget>()) };
        WebViewParts {
            widget,
            ptr,
            manager,
            cookie_manager,
        }
    }

    pub(super) fn cstring(value: &str) -> Option<CString> {
        CString::new(value).ok()
    }

    pub(super) fn cstr_to_string(ptr: *const c_char) -> String {
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }
    }

    pub(super) fn load_uri(ptr: NonNull<WebKitWebView>, uri: &str) {
        if let Some(cstr) = cstring(uri) {
            unsafe { webkit_web_view_load_uri(ptr.as_ptr(), cstr.as_ptr()) };
        }
    }

    pub(super) fn go_back(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_go_back(ptr.as_ptr()) };
    }

    pub(super) fn go_forward(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_go_forward(ptr.as_ptr()) };
    }

    pub(super) fn stop(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_stop_loading(ptr.as_ptr()) };
    }

    pub(super) fn reload(ptr: NonNull<WebKitWebView>) {
        unsafe { webkit_web_view_reload(ptr.as_ptr()) };
    }

    pub(super) fn can_go_back(ptr: NonNull<WebKitWebView>) -> bool {
        unsafe { webkit_web_view_can_go_back(ptr.as_ptr()) != 0 }
    }

    pub(super) fn can_go_forward(ptr: NonNull<WebKitWebView>) -> bool {
        unsafe { webkit_web_view_can_go_forward(ptr.as_ptr()) != 0 }
    }

    pub(super) fn set_user_agent(ptr: NonNull<WebKitWebView>, user_agent: &str) {
        if let Some(cstr) = cstring(user_agent) {
            unsafe { webkit_web_view_set_custom_user_agent(ptr.as_ptr(), cstr.as_ptr()) };
        }
    }

    pub(super) fn current_uri(ptr: NonNull<WebKitWebView>) -> String {
        let raw = unsafe { webkit_web_view_get_uri(ptr.as_ptr()) };
        cstr_to_string(raw)
    }

    /// Injects `source`, restricted to the documents `allow_list` describes.
    ///
    /// `None` is `WebKit`'s "every document"; a list restricts injection to the
    /// URI patterns in it. An *empty* list is never passed: `WebKit` reads it as
    /// no restriction, so the caller must not inject at all in that case.
    pub(super) fn add_user_script(
        manager: NonNull<WebKitUserContentManager>,
        source: &str,
        time: ScriptInjectionTime,
        allow_list: Option<&[Str]>,
    ) {
        let Some(source) = cstring(source) else {
            return;
        };
        let injection_time = match time {
            ScriptInjectionTime::DocumentStart => WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START,
            ScriptInjectionTime::DocumentEnd => WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_END,
        };
        let patterns: Option<Vec<CString>> = allow_list.map(|patterns| {
            assert!(
                !patterns.is_empty(),
                "an empty WebKit allow list injects everywhere; do not inject instead (fast-fail)"
            );
            patterns
                .iter()
                .map(|pattern| {
                    cstring(pattern.as_str()).expect("an origin pattern must not contain NUL")
                })
                .collect()
        });
        // Kept alive for the whole call: WebKit copies the strings out of it.
        let allow_pointers: Option<Vec<*const c_char>> = patterns.as_ref().map(|patterns| {
            patterns
                .iter()
                .map(|pattern| pattern.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect()
        });
        let allow_list = allow_pointers
            .as_ref()
            .map_or(std::ptr::null(), |pointers| pointers.as_ptr());
        let script = unsafe {
            webkit_user_script_new(
                source.as_ptr(),
                WEBKIT_USER_CONTENT_INJECT_TOP_FRAME,
                injection_time,
                allow_list,
                std::ptr::null(),
            )
        };
        if script.is_null() {
            return;
        }
        unsafe {
            webkit_user_content_manager_add_script(manager.as_ptr(), script);
            webkit_user_script_unref(script);
        }
    }

    pub(super) fn remove_all_scripts(manager: NonNull<WebKitUserContentManager>) {
        unsafe { webkit_user_content_manager_remove_all_scripts(manager.as_ptr()) };
    }

    pub(super) fn register_script_message_handler(
        manager: NonNull<WebKitUserContentManager>,
        name: &str,
    ) -> bool {
        let Some(name) = cstring(name) else {
            return false;
        };
        unsafe {
            webkit_user_content_manager_register_script_message_handler(
                manager.as_ptr(),
                name.as_ptr(),
                std::ptr::null(),
            ) != 0
        }
    }

    pub(super) fn unregister_script_message_handler(
        manager: NonNull<WebKitUserContentManager>,
        name: &str,
    ) {
        let Some(name) = cstring(name) else {
            return;
        };
        unsafe {
            webkit_user_content_manager_unregister_script_message_handler(
                manager.as_ptr(),
                name.as_ptr(),
                std::ptr::null(),
            );
        }
    }

    pub(super) fn evaluate_javascript(
        ptr: NonNull<WebKitWebView>,
        script: &str,
        callback: gio::ffi::GAsyncReadyCallback,
        user_data: *mut c_void,
    ) -> Result<(), String> {
        let Some(script) = cstring(script) else {
            return Err(String::from("JavaScript contains interior NUL byte"));
        };
        unsafe {
            webkit_web_view_evaluate_javascript(
                ptr.as_ptr(),
                script.as_ptr(),
                -1,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                callback,
                user_data,
            );
        }
        Ok(())
    }

    pub(super) fn evaluate_javascript_finish(
        ptr: NonNull<WebKitWebView>,
        result: *mut gio::ffi::GAsyncResult,
    ) -> Result<*mut JSCValue, String> {
        let mut error: *mut glib::ffi::GError = std::ptr::null_mut();
        let value =
            unsafe { webkit_web_view_evaluate_javascript_finish(ptr.as_ptr(), result, &mut error) };
        finished(value, error, "JavaScript evaluation failed")
    }

    /// Runs `body` as the body of an `async` function and awaits the promise it
    /// returns.
    ///
    /// The shared evaluation wrapper is `async`, so
    /// `webkit_web_view_evaluate_javascript` hands back the unresolved `Promise`
    /// rather than the JSON envelope. This is the entry point that awaits it.
    pub(super) fn call_async_javascript_function(
        ptr: NonNull<WebKitWebView>,
        body: &str,
        callback: gio::ffi::GAsyncReadyCallback,
        user_data: *mut c_void,
    ) -> Result<(), String> {
        let Some(body) = cstring(body) else {
            return Err(String::from("JavaScript contains interior NUL byte"));
        };
        unsafe {
            webkit_web_view_call_async_javascript_function(
                ptr.as_ptr(),
                body.as_ptr(),
                -1,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                callback,
                user_data,
            );
        }
        Ok(())
    }

    pub(super) fn call_async_javascript_function_finish(
        ptr: NonNull<WebKitWebView>,
        result: *mut gio::ffi::GAsyncResult,
    ) -> Result<*mut JSCValue, String> {
        let mut error: *mut glib::ffi::GError = std::ptr::null_mut();
        let value = unsafe {
            webkit_web_view_call_async_javascript_function_finish(ptr.as_ptr(), result, &mut error)
        };
        finished(value, error, "JavaScript function call failed")
    }

    /// Turns one `GError`-or-value pair from a `_finish` call into a `Result`.
    fn finished(
        value: *mut JSCValue,
        error: *mut glib::ffi::GError,
        failure: &str,
    ) -> Result<*mut JSCValue, String> {
        if !error.is_null() {
            let message = cstr_to_string(unsafe { (*error).message });
            unsafe { glib::ffi::g_error_free(error) };
            return Err(if message.is_empty() {
                failure.to_owned()
            } else {
                message
            });
        }
        Ok(value)
    }

    /// The strictest injection pattern `WebKit` can express for `rule`.
    ///
    /// [`OriginRule::injection_pattern`] renders the rule exactly, port and all,
    /// but `WebKit`'s `UserContentURLPattern` rejects a pattern whose host
    /// contains `:` and an invalid pattern matches nothing — so passing
    /// `http://localhost:3000/*` through would inject nowhere and leave a dev
    /// server with no bridge at all. `WebKit`'s filter is host-granular, so a
    /// ported origin becomes the pattern for its host, and the port is enforced
    /// exactly where it can be: the origin check on every bridge message.
    ///
    /// The URI is taken apart by GLib rather than by string surgery here.
    pub(super) fn injection_pattern(rule: &waterui_webview::OriginRule) -> Str {
        let waterui_webview::OriginRule::Exact(origin) = rule else {
            return rule.injection_pattern();
        };
        let parsed = glib::Uri::parse(origin, glib::UriFlags::NONE)
            .unwrap_or_else(|error| panic!("`{origin}` is not a usable origin: {error}"));
        if parsed.port() < 0 {
            return rule.injection_pattern();
        }
        let scheme = parsed.scheme();
        let host = parsed
            .host()
            .unwrap_or_else(|| panic!("`{origin}` names a port but no host"));
        Str::from(format!("{scheme}://{host}/*"))
    }

    /// The origin `WebKit` itself reports for a document at `uri`, in the
    /// `scheme://host[:port]` form the shared origin policy matches against.
    ///
    /// `None` for an opaque origin — `about:blank`, `data:`, a sandboxed
    /// document — which is exactly the case that cannot be authenticated.
    pub(super) fn security_origin(uri: &str) -> Option<Str> {
        let uri = cstring(uri)?;
        let origin = unsafe { webkit_security_origin_new_for_uri(uri.as_ptr()) };
        if origin.is_null() {
            return None;
        }
        let raw = unsafe { webkit_security_origin_to_string(origin) };
        unsafe { webkit_security_origin_unref(origin) };
        if raw.is_null() {
            return None;
        }
        let text = cstr_to_string(raw);
        unsafe { glib::ffi::g_free(raw.cast()) };
        Some(Str::from(text))
    }

    pub(super) fn jsc_value_to_rust(value: *mut JSCValue) -> Str {
        if value.is_null() {
            return Str::from_static("null");
        }
        if unsafe { jsc_value_is_null(value) != 0 || jsc_value_is_undefined(value) != 0 } {
            return Str::from_static("null");
        }
        if unsafe { jsc_value_is_string(value) != 0 } {
            let raw = unsafe { jsc_value_to_string(value) };
            let text = cstr_to_string(raw);
            if !raw.is_null() {
                unsafe { glib::ffi::g_free(raw.cast()) };
            }
            return Str::from(text);
        }
        let raw_json = unsafe { jsc_value_to_json(value, 0) };
        if !raw_json.is_null() {
            let text = cstr_to_string(raw_json);
            unsafe { glib::ffi::g_free(raw_json.cast()) };
            return Str::from(text);
        }
        let raw = unsafe { jsc_value_to_string(value) };
        let text = cstr_to_string(raw);
        if !raw.is_null() {
            unsafe { glib::ffi::g_free(raw.cast()) };
        }
        Str::from(text)
    }

    pub(super) fn unref_jsc_value(value: *mut JSCValue) {
        if value.is_null() {
            return;
        }
        unsafe { glib::gobject_ffi::g_object_unref(value.cast()) };
    }

    pub(super) fn policy_ignore(decision: *mut WebKitPolicyDecision) {
        unsafe { webkit_policy_decision_ignore(decision) };
    }

    pub(super) fn response_from_decision(
        decision: *mut WebKitPolicyDecision,
    ) -> *mut WebKitURIResponse {
        unsafe { webkit_response_policy_decision_get_response(decision.cast()) }
    }

    pub(super) fn response_status(response: *mut WebKitURIResponse) -> u32 {
        unsafe { webkit_uri_response_get_status_code(response) }
    }

    pub(super) fn response_uri(response: *mut WebKitURIResponse) -> String {
        let uri = unsafe { webkit_uri_response_get_uri(response) };
        cstr_to_string(uri)
    }

    pub(super) fn response_redirect_target(response: *mut WebKitURIResponse) -> Option<String> {
        let headers = unsafe { webkit_uri_response_get_http_headers(response) };
        if headers.is_null() {
            return None;
        }
        let location = unsafe { soup_message_headers_get_one(headers, c"Location".as_ptr()) };
        if location.is_null() {
            None
        } else {
            Some(cstr_to_string(location))
        }
    }

    /// Parses a `Set-Cookie` header as if it had arrived from `origin`.
    ///
    /// The origin is what gives a cookie without an explicit `Domain` its
    /// domain. Parsing against `NULL` left that domain unset, and
    /// `webkit_cookie_manager_add_cookie` then refused the cookie.
    pub(super) fn parse_cookie(header: &str, origin: &str) -> *mut SoupCookie {
        let Some(header) = cstring(header) else {
            return std::ptr::null_mut();
        };
        let origin = glib::Uri::parse(origin, glib::UriFlags::NONE)
            .unwrap_or_else(|error| panic!("WebKit reported an unparseable document URI: {error}"));
        unsafe { soup_cookie_parse(header.as_ptr(), origin.to_glib_none().0) }
    }

    pub(super) fn add_cookie(
        manager: NonNull<WebKitCookieManager>,
        cookie: *mut SoupCookie,
        callback: gio::ffi::GAsyncReadyCallback,
        user_data: *mut c_void,
    ) {
        unsafe {
            webkit_cookie_manager_add_cookie(
                manager.as_ptr(),
                cookie,
                std::ptr::null_mut(),
                callback,
                user_data,
            );
        }
    }

    pub(super) fn free_cookie(cookie: *mut SoupCookie) {
        if cookie.is_null() {
            return;
        }
        unsafe { soup_cookie_free(cookie) };
    }

    pub(super) fn add_cookie_finish(
        manager: NonNull<WebKitCookieManager>,
        result: *mut gio::ffi::GAsyncResult,
    ) -> Result<(), String> {
        let mut error: *mut glib::ffi::GError = std::ptr::null_mut();
        let ok = unsafe {
            webkit_cookie_manager_add_cookie_finish(manager.as_ptr(), result, &mut error)
        };
        if !error.is_null() {
            let message = cstr_to_string(unsafe { (*error).message });
            unsafe { glib::ffi::g_error_free(error) };
            return Err(if message.is_empty() {
                String::from("cookie add failed")
            } else {
                message
            });
        }
        if ok == 0 {
            return Err(String::from("cookie add failed"));
        }
        Ok(())
    }

    pub(super) fn get_cookies(
        manager: NonNull<WebKitCookieManager>,
        uri: &str,
        callback: gio::ffi::GAsyncReadyCallback,
        user_data: *mut c_void,
    ) {
        let Some(uri) = cstring(uri) else {
            return;
        };
        unsafe {
            webkit_cookie_manager_get_cookies(
                manager.as_ptr(),
                uri.as_ptr(),
                std::ptr::null_mut(),
                callback,
                user_data,
            );
        }
    }

    pub(super) fn get_cookies_finish(
        manager: NonNull<WebKitCookieManager>,
        result: *mut gio::ffi::GAsyncResult,
    ) -> Result<*mut glib::ffi::GList, String> {
        let mut error: *mut glib::ffi::GError = std::ptr::null_mut();
        let list = unsafe {
            webkit_cookie_manager_get_cookies_finish(manager.as_ptr(), result, &mut error)
        };
        if !error.is_null() {
            let message = cstr_to_string(unsafe { (*error).message });
            unsafe { glib::ffi::g_error_free(error) };
            return Err(if message.is_empty() {
                String::from("cookie query failed")
            } else {
                message
            });
        }
        Ok(list)
    }

    pub(super) fn cookie_to_set_cookie_header(cookie: *mut SoupCookie) -> Option<String> {
        let raw = unsafe { soup_cookie_to_set_cookie_header(cookie) };
        if raw.is_null() {
            return None;
        }
        let text = cstr_to_string(raw);
        unsafe { glib::ffi::g_free(raw.cast()) };
        Some(text)
    }

    pub(super) fn free_cookie_list(list: *mut glib::ffi::GList) {
        if list.is_null() {
            return;
        }
        let free_cookie = Some(unsafe {
            std::mem::transmute::<
                unsafe extern "C" fn(*mut SoupCookie),
                unsafe extern "C" fn(*mut c_void),
            >(soup_cookie_free)
        });
        unsafe { glib::ffi::g_list_free_full(list, free_cookie) };
    }

    unsafe extern "C" fn destroy_boxed<T>(
        data: *mut c_void,
        _closure: *mut glib::gobject_ffi::GClosure,
    ) {
        unsafe { drop(Box::from_raw(data.cast::<T>())) };
    }

    pub(super) unsafe fn connect_signal<T>(
        instance: *mut glib::gobject_ffi::GObject,
        detailed_signal: &CStr,
        callback: glib::gobject_ffi::GCallback,
        data: T,
    ) -> c_ulong {
        unsafe {
            glib::gobject_ffi::g_signal_connect_data(
                instance,
                detailed_signal.as_ptr(),
                callback,
                Box::into_raw(Box::new(data)).cast(),
                Some(destroy_boxed::<T>),
                0,
            )
        }
    }

    pub(super) fn disconnect_signal(instance: *mut glib::gobject_ffi::GObject, signal_id: c_ulong) {
        unsafe { glib::gobject_ffi::g_signal_handler_disconnect(instance, signal_id) };
    }
}

/// Installs the system `WebKitGTK` controller unless one is already present.
pub fn ensure_webview_controller(env: &mut Environment) {
    if env.get::<WebViewController>().is_some() {
        return;
    }
    env.insert(WebViewController::new(GtkWebViewController));
}

#[derive(Debug, Default, Clone)]
pub(crate) struct GtkWebViewController;

impl CustomWebViewController for GtkWebViewController {
    fn open(&self) -> impl WebViewHandle {
        GtkWebViewHandle::new()
    }
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct NativeState {
    ptr: std::ptr::NonNull<webkitgtk::WebKitWebView>,
    manager: std::ptr::NonNull<webkitgtk::WebKitUserContentManager>,
    cookie_manager: std::ptr::NonNull<webkitgtk::WebKitCookieManager>,
    custom_scripts: RefCell<Vec<InjectedScript>>,
    handler_signal_ids: RefCell<HashMap<String, std::os::raw::c_ulong>>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
impl Drop for NativeState {
    fn drop(&mut self) {
        let mut ids = self.handler_signal_ids.borrow_mut();
        for (name, id) in ids.drain() {
            webkitgtk::disconnect_signal(self.manager.as_ptr().cast(), id);
            webkitgtk::unregister_script_message_handler(self.manager, &name);
        }
    }
}

#[derive(Clone)]
pub(crate) struct GtkWebViewHandle {
    widget: Widget,
    shared: Rc<SharedState>,
    #[cfg(all(
        feature = "webkitgtk",
        gtk_webkitgtk_link_available,
        unix,
        not(target_os = "macos")
    ))]
    native: Rc<NativeState>,
}

impl core::fmt::Debug for GtkWebViewHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GtkWebViewHandle")
    }
}

impl GtkWebViewHandle {
    pub(crate) fn new() -> Self {
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");

        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            let shared = Rc::new(SharedState::default());
            let parts = webkitgtk::create_webview();
            let native = Rc::new(NativeState {
                ptr: parts.ptr,
                manager: parts.manager,
                cookie_manager: parts.cookie_manager,
                custom_scripts: RefCell::new(Vec::new()),
                handler_signal_ids: RefCell::new(HashMap::new()),
            });

            let this = Self {
                widget: parts.widget,
                shared,
                native,
            };
            this.install_observers();
            this.rebuild_user_scripts();
            this
        }
    }

    pub(crate) fn widget(&self) -> Widget {
        self.widget.clone()
    }

    /// Converts a URI `WebKitGTK` reported into the event payload type.
    ///
    /// Parsed through `FromStr` rather than [`Url::parse`], which keeps only
    /// web URLs: an engine legitimately navigates to `about:blank`, and that is
    /// not an error. A URI the engine emits that does not parse at all is a
    /// contract break worth crashing on, matching the WPE bridge's
    /// `parse_url`; the removed `Url::from(String)` fallback used to
    /// manufacture a bogus `Url` and hand it to the application instead.
    fn parse_url(raw: &str) -> Url {
        raw.parse()
            .unwrap_or_else(|error| panic!("WebKitGTK emitted an invalid URL {raw:?}: {error}"))
    }

    #[cfg(all(
        feature = "webkitgtk",
        gtk_webkitgtk_link_available,
        unix,
        not(target_os = "macos")
    ))]
    fn current_uri_or_default(&self) -> String {
        let uri = webkitgtk::current_uri(self.native.ptr);
        if uri.is_empty() {
            String::from("https://localhost/")
        } else {
            uri
        }
    }

    #[cfg(all(
        feature = "webkitgtk",
        gtk_webkitgtk_link_available,
        unix,
        not(target_os = "macos")
    ))]
    fn install_observers(&self) {
        assert!(
            self.widget.find_property("uri").is_some(),
            "GTK WebView missing `uri` property"
        );
        assert!(
            self.widget
                .find_property("estimated-load-progress")
                .is_some(),
            "GTK WebView missing `estimated-load-progress` property"
        );
        assert!(
            self.widget.find_property("can-go-back").is_some(),
            "GTK WebView missing `can-go-back` property"
        );
        assert!(
            self.widget.find_property("can-go-forward").is_some(),
            "GTK WebView missing `can-go-forward` property"
        );

        // Every closure below captures `shared` alone. Capturing the handle — which
        // owns `widget` — closed a cycle through the widget the closure is attached
        // to, so `NativeState::drop` never ran: the script message handler stayed
        // registered and the web view, its web process and every registered handler
        // outlived the view that showed them.
        let shared = self.shared.clone();
        self.widget
            .connect_notify_local(Some("uri"), move |obj, _| {
                let uri = obj.property::<String>("uri");
                if uri.is_empty() {
                    return;
                }
                shared.emit(WebViewEvent::WillNavigate {
                    url: Self::parse_url(&uri),
                });
            });

        let shared = self.shared.clone();
        self.widget
            .connect_notify_local(Some("estimated-load-progress"), move |obj, _| {
                let progress = obj.property::<f64>("estimated-load-progress") as f32;
                shared.emit(WebViewEvent::Loading { progress });
                if progress >= 1.0 {
                    shared.emit(WebViewEvent::Loaded);
                }
            });

        let shared = self.shared.clone();
        self.widget
            .connect_notify_local(Some("can-go-back"), move |obj, _| {
                let back = obj.property::<bool>("can-go-back");
                let forward = obj.property::<bool>("can-go-forward");
                shared.emit(BackendEvent::NavigationState {
                    can_go_back: back,
                    can_go_forward: forward,
                });
            });

        let shared = self.shared.clone();
        self.widget
            .connect_notify_local(Some("can-go-forward"), move |obj, _| {
                let back = obj.property::<bool>("can-go-back");
                let forward = obj.property::<bool>("can-go-forward");
                shared.emit(BackendEvent::NavigationState {
                    can_go_back: back,
                    can_go_forward: forward,
                });
            });

        let webview_obj = self
            .native
            .ptr
            .as_ptr()
            .cast::<gtk4::glib::gobject_ffi::GObject>();

        let decide_policy_signal =
            std::ffi::CString::new("decide-policy").expect("valid signal name");
        let decide_policy_callback = Some(unsafe {
            std::mem::transmute::<
                unsafe extern "C" fn(
                    *mut webkitgtk::WebKitWebView,
                    *mut webkitgtk::WebKitPolicyDecision,
                    i32,
                    *mut std::ffi::c_void,
                ) -> gtk4::glib::ffi::gboolean,
                unsafe extern "C" fn(),
            >(on_decide_policy)
        });
        let decide_policy_data = DecidePolicyData {
            shared: self.shared.clone(),
        };
        unsafe {
            webkitgtk::connect_signal(
                webview_obj,
                &decide_policy_signal,
                decide_policy_callback,
                decide_policy_data,
            );
        }

        let load_failed_signal = std::ffi::CString::new("load-failed").expect("valid signal name");
        let load_failed_callback = Some(unsafe {
            std::mem::transmute::<
                unsafe extern "C" fn(
                    *mut webkitgtk::WebKitWebView,
                    i32,
                    *const std::ffi::c_char,
                    *mut gtk4::glib::ffi::GError,
                    *mut std::ffi::c_void,
                ) -> gtk4::glib::ffi::gboolean,
                unsafe extern "C" fn(),
            >(on_load_failed)
        });
        let load_failed_data = LoadFailedData {
            shared: self.shared.clone(),
        };
        unsafe {
            webkitgtk::connect_signal(
                webview_obj,
                &load_failed_signal,
                load_failed_callback,
                load_failed_data,
            );
        }

        let tls_signal =
            std::ffi::CString::new("load-failed-with-tls-errors").expect("valid signal name");
        let tls_callback = Some(unsafe {
            std::mem::transmute::<
                unsafe extern "C" fn(
                    *mut webkitgtk::WebKitWebView,
                    *const std::ffi::c_char,
                    *mut std::ffi::c_void,
                    u32,
                    *mut std::ffi::c_void,
                ) -> gtk4::glib::ffi::gboolean,
                unsafe extern "C" fn(),
            >(on_load_failed_with_tls_errors)
        });
        let tls_data = TlsFailedData {
            shared: self.shared.clone(),
        };
        unsafe {
            webkitgtk::connect_signal(webview_obj, &tls_signal, tls_callback, tls_data);
        }
    }

    /// Reinstalls every user script, restricted to the documents the bridge
    /// origin policy admits.
    ///
    /// `WebKit` has no "replace this script" call, so the whole set is rebuilt;
    /// that is also what makes an [`inject_script`](WebViewHandle::inject_script)
    /// under an existing key replace rather than stack.
    ///
    /// Nothing is injected while no policy admits anything. The scripts carry the
    /// bridge and the mirrored-state seed — the seed being the current *values*
    /// of the exposed state — so injecting them into a document that may not use
    /// the bridge hands that document state it is not allowed to read.
    #[cfg(all(
        feature = "webkitgtk",
        gtk_webkitgtk_link_available,
        unix,
        not(target_os = "macos")
    ))]
    fn rebuild_user_scripts(&self) {
        webkitgtk::remove_all_scripts(self.native.manager);
        let policy = self.shared.bridge_origins.borrow().clone();
        let Some(rules) = policy.as_ref().map(waterui_webview::OriginPolicy::rules) else {
            // No policy installed yet; one always arrives before the first
            // handler is registered.
            return;
        };
        let allow_list: Option<Vec<Str>> = match rules.as_slice() {
            // Deny-all: no document may reach the bridge, so nothing is injected.
            // WebKit reads an *empty* allow list as "no restriction", which is why
            // this case cannot be expressed as one.
            [] => return,
            // WebKit's own spelling of "every document" is a null allow list.
            [waterui_webview::OriginRule::Any] => None,
            rules => Some(rules.iter().map(webkitgtk::injection_pattern).collect()),
        };
        let allow_list = allow_list.as_deref();

        // Transport first: the shared script calls `__wateruiSend`, so the adapter
        // onto WebKitGTK's message handler has to exist before it runs.
        webkitgtk::add_user_script(
            self.native.manager,
            TRANSPORT_SCRIPT,
            ScriptInjectionTime::DocumentStart,
            allow_list,
        );
        webkitgtk::add_user_script(
            self.native.manager,
            waterui_webview::DOCUMENT_START_SCRIPT,
            ScriptInjectionTime::DocumentStart,
            allow_list,
        );
        let custom = self.native.custom_scripts.borrow().clone();
        for script in custom {
            webkitgtk::add_user_script(
                self.native.manager,
                &script.source,
                script.time,
                allow_list,
            );
        }
    }

    /// Starts one evaluation and returns the future that settles with its result.
    ///
    /// `start` chooses which of `WebKit`'s two evaluation entry points runs it:
    /// the raw one, or the one that awaits the promise the script returns.
    #[cfg(all(
        feature = "webkitgtk",
        gtk_webkitgtk_link_available,
        unix,
        not(target_os = "macos")
    ))]
    fn evaluate(&self, script: &str, start: JsStart, finish: JsFinish) -> JsFuture {
        start_javascript(&self.widget, script, start, finish)
    }
}

/// Starts one evaluation on `view`, keeping the view alive until it completes.
///
/// The strong `Widget` reference in the callback's data is the liveness
/// guarantee: `_finish` dereferences the web view, so a pending evaluation must
/// not be able to outlive it.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
fn start_javascript(view: &Widget, script: &str, start: JsStart, finish: JsFinish) -> JsFuture {
    let state = Rc::new(RefCell::new(PendingState::new()));
    let data = Box::into_raw(Box::new(JsEvalData {
        state: Rc::clone(&state),
        view: view.clone(),
        finish,
    }));
    if let Err(error) = start(
        webview_ptr(view),
        script,
        Some(on_javascript_evaluated),
        data.cast(),
    ) {
        // SAFETY: WebKit refused the call before taking the callback data, so this
        // box is still ours and nothing else will ever see it.
        unsafe { drop(Box::from_raw(data)) };
        settle(&state, Err(Str::from(error)));
    }
    JsFuture { state }
}

/// The `WebKitWebView` behind a widget this backend created.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
fn webview_ptr(view: &Widget) -> std::ptr::NonNull<webkitgtk::WebKitWebView> {
    std::ptr::NonNull::new(view.as_ptr().cast::<webkitgtk::WebKitWebView>())
        .expect("a live GTK widget never has a null instance pointer")
}

/// Evaluates one bridge reply in the page and reports a delivery failure.
///
/// A reply that never arrives leaves the page's promise pending forever, so the
/// failure is logged rather than dropped.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
fn deliver_reply(view: &Widget, reply: &bridge::Reply, id: u64) {
    let script = reply.resolve_script(id);
    let pending = start_javascript(
        view,
        &script,
        webkitgtk::evaluate_javascript,
        webkitgtk::evaluate_javascript_finish,
    );
    executor_core::spawn_local(async move {
        if let Err(error) = pending.await {
            tracing::warn!(%error, "failed to deliver a WaterUI bridge reply to the page");
        }
    })
    .detach();
}

impl WebViewHandle for GtkWebViewHandle {
    fn go_back(&self) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            webkitgtk::go_back(self.native.ptr);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");
    }

    fn go_forward(&self) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            webkitgtk::go_forward(self.native.ptr);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");
    }

    fn go_to(&self, url: &Url) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            webkitgtk::load_uri(self.native.ptr, url);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        {
            let _ = url;
            panic!("{WEBKIT_FEATURE_MSG}");
        }
    }

    fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            let injected = InjectedScript {
                key: key.to_owned(),
                source: script.to_owned(),
                time,
            };
            {
                let mut scripts = self.native.custom_scripts.borrow_mut();
                // Replacing in place keeps the injection order stable, which the
                // mirrored-state seed depends on: it is re-rendered and replaced
                // before every navigation, and a seed that moved behind a later
                // script would define stale values over fresh ones.
                match scripts.iter_mut().find(|existing| existing.key == key) {
                    Some(existing) => *existing = injected,
                    None => scripts.push(injected),
                }
            }
            self.rebuild_user_scripts();
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        {
            let _ = (key, script, time);
            panic!("{WEBKIT_FEATURE_MSG}");
        }
    }

    fn add_handler(&self, name: &str, handler: Box<waterui_webview::ScriptMessageHandler>) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            let handler: JsHandler = Rc::from(handler);
            // Registering the same name twice replaces the handler, matching every
            // other backend.
            self.shared
                .handler_callbacks
                .borrow_mut()
                .insert(name.to_owned(), handler);

            // One WebKit message handler serves every WaterUI handler name, so the
            // transport is registered once rather than per name.
            if self
                .native
                .handler_signal_ids
                .borrow()
                .contains_key(bridge::SEND_FUNCTION)
            {
                return;
            }

            let registered = webkitgtk::register_script_message_handler(
                self.native.manager,
                bridge::SEND_FUNCTION,
            );
            assert!(
                registered,
                "failed to register the WaterUI bridge script message handler (fast-fail)"
            );

            let signal_name = std::ffi::CString::new(format!(
                "script-message-received::{}",
                bridge::SEND_FUNCTION
            ))
            .expect("valid detailed signal");
            let callback = Some(unsafe {
                std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut webkitgtk::WebKitUserContentManager,
                        *mut webkitgtk::JSCValue,
                        *mut std::ffi::c_void,
                    ),
                    unsafe extern "C" fn(),
                >(on_script_message_received)
            });

            let data = ScriptMessageData {
                shared: self.shared.clone(),
                view: self.widget.downgrade(),
            };

            let signal_id = unsafe {
                webkitgtk::connect_signal(
                    self.native.manager.as_ptr().cast(),
                    &signal_name,
                    callback,
                    data,
                )
            };
            self.native
                .handler_signal_ids
                .borrow_mut()
                .insert(bridge::SEND_FUNCTION.to_owned(), signal_id);
            self.rebuild_user_scripts();
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        {
            let _ = (name, handler);
            panic!("{WEBKIT_FEATURE_MSG}");
        }
    }

    fn set_bridge_origins(&self, policy: waterui_webview::OriginPolicy) {
        self.shared.bridge_origins.replace(Some(policy));
        // The policy decides where the bridge and the mirrored-state seed are
        // injected, so the installed scripts are reinstalled under the new one.
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        self.rebuild_user_scripts();
    }

    fn remove_handler(&self, name: &str) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            // Removing a name that was never registered is a no-op, matching every
            // other backend. The transport stays registered for the page's life.
            self.shared.handler_callbacks.borrow_mut().remove(name);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        {
            let _ = name;
            panic!("{WEBKIT_FEATURE_MSG}");
        }
    }

    fn stop(&self) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            webkitgtk::stop(self.native.ptr);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");
    }

    fn refresh(&self) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            webkitgtk::reload(self.native.ptr);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");
    }

    fn set_user_agent(&self, user_agent: &str) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            webkitgtk::set_user_agent(self.native.ptr, user_agent);
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        {
            let _ = user_agent;
            panic!("{WEBKIT_FEATURE_MSG}");
        }
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        self.shared
            .redirects_enabled
            .replace(Computed::new(enabled));
    }

    fn watch(&self, f: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.shared.watchers.insert(f)
    }

    fn can_go_back(&self) -> bool {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            return webkitgtk::can_go_back(self.native.ptr);
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");
    }

    fn can_go_forward(&self) -> bool {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            return webkitgtk::can_go_forward(self.native.ptr);
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        panic!("{WEBKIT_FEATURE_MSG}");
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        {
            let cookie_value = cookie.to_string();
            let uri = self.current_uri_or_default();
            // Parsed against the document's own URI: a cookie with no explicit
            // `Domain` takes its domain from the origin it arrived at, and one
            // parsed against no origin has none at all, which is what made
            // `webkit_cookie_manager_add_cookie` refuse it.
            let raw_cookie = webkitgtk::parse_cookie(&cookie_value, &uri);
            if raw_cookie.is_null() {
                self.shared
                    .emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
                        "Invalid Set-Cookie header",
                    ))));
                return;
            }
            let data = CookieAddData {
                shared: self.shared.clone(),
                cookie_manager: self.native.cookie_manager,
                cookie: raw_cookie,
            };
            webkitgtk::add_cookie(
                self.native.cookie_manager,
                raw_cookie,
                Some(on_cookie_added),
                Box::into_raw(Box::new(data)).cast(),
            );
            return;
        }
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        {
            let _ = cookie;
            panic!("{WEBKIT_FEATURE_MSG}");
        }
    }

    /// Asks the cookie manager for the current document's cookies.
    ///
    /// Queried live rather than read from a cache refreshed at load: a cookie the
    /// page sets afterwards — an XHR that logs in — was invisible until the next
    /// navigation, while every other backend answered from the live store.
    fn get_cookies(&self) -> impl std::future::Future<Output = Vec<Cookie<'static>>> {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        let cookies: BoxFuture<Vec<Cookie<'static>>> = {
            let state = Rc::new(RefCell::new(PendingState::new()));
            let data = CookieQueryData {
                state: Rc::clone(&state),
                shared: self.shared.clone(),
                _view: self.widget.clone(),
                cookie_manager: self.native.cookie_manager,
            };
            webkitgtk::get_cookies(
                self.native.cookie_manager,
                &self.current_uri_or_default(),
                Some(on_cookies_queried),
                Box::into_raw(Box::new(data)).cast(),
            );
            Box::pin(CookieFuture { state })
        };
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        let cookies: BoxFuture<Vec<Cookie<'static>>> = panic!("{WEBKIT_FEATURE_MSG}");
        cookies
    }

    fn run_javascript(&self, script: &str) -> impl std::future::Future<Output = Result<Str, Str>> {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        let result: BoxFuture<Result<Str, Str>> = Box::pin(self.evaluate(
            script,
            webkitgtk::evaluate_javascript,
            webkitgtk::evaluate_javascript_finish,
        ));
        // Every other method fast-fails without a linkable WebKitGTK; this one used
        // to answer with an `Err` instead, which reads as "the script failed".
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        let result: BoxFuture<Result<Str, Str>> = {
            let _ = script;
            panic!("{WEBKIT_FEATURE_MSG}")
        };
        result
    }

    fn call_async_javascript(
        &self,
        body: &str,
    ) -> impl std::future::Future<Output = Result<Str, Str>> {
        #[cfg(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        ))]
        let result: BoxFuture<Result<Str, Str>> = Box::pin(self.evaluate(
            body,
            webkitgtk::call_async_javascript_function,
            webkitgtk::call_async_javascript_function_finish,
        ));
        #[cfg(not(all(
            feature = "webkitgtk",
            gtk_webkitgtk_link_available,
            unix,
            not(target_os = "macos")
        )))]
        let result: BoxFuture<Result<Str, Str>> = {
            let _ = body;
            panic!("{WEBKIT_FEATURE_MSG}")
        };
        result
    }
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
#[derive(Clone)]
struct DecidePolicyData {
    shared: Rc<SharedState>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
#[derive(Clone)]
struct LoadFailedData {
    shared: Rc<SharedState>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
#[derive(Clone)]
struct TlsFailedData {
    shared: Rc<SharedState>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct ScriptMessageData {
    shared: Rc<SharedState>,
    /// Weak, because this data lives in a closure the web view's own user content
    /// manager owns: a strong reference here would keep the view alive forever.
    /// Upgrading before each use is also the liveness check the reply path needs,
    /// since a handler may await while the view is torn down.
    view: gtk4::glib::WeakRef<Widget>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct CookieAddData {
    shared: Rc<SharedState>,
    cookie_manager: std::ptr::NonNull<webkitgtk::WebKitCookieManager>,
    cookie: *mut webkitgtk::SoupCookie,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct CookieQueryData {
    state: Rc<RefCell<PendingState<Vec<Cookie<'static>>>>>,
    shared: Rc<SharedState>,
    /// Strong, and held for its reference alone: the cookie manager belongs to
    /// the web view, and this query dereferences it in a callback that runs after
    /// the view could otherwise have been dropped.
    _view: Widget,
    cookie_manager: std::ptr::NonNull<webkitgtk::WebKitCookieManager>,
}

/// One value a GLib async callback will produce, and the waker waiting for it.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct PendingState<T> {
    result: Option<T>,
    waker: Option<std::task::Waker>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
impl<T> PendingState<T> {
    const fn new() -> Self {
        Self {
            result: None,
            waker: None,
        }
    }
}

/// The future half of a [`PendingState`].
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct Pending<T> {
    state: Rc<RefCell<PendingState<T>>>,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
impl<T> std::future::Future for Pending<T> {
    type Output = T;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut state = self.state.borrow_mut();
        if let Some(result) = state.result.take() {
            std::task::Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    }
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
type JsFuture = Pending<Result<Str, Str>>;

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
type CookieFuture = Pending<Vec<Cookie<'static>>>;

/// Starts one evaluation. `webkit_web_view_evaluate_javascript` runs the script
/// as written; `webkit_web_view_call_async_javascript_function` awaits the
/// promise it returns.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
type JsStart = fn(
    std::ptr::NonNull<webkitgtk::WebKitWebView>,
    &str,
    gtk4::gio::ffi::GAsyncReadyCallback,
    *mut std::ffi::c_void,
) -> Result<(), String>;

/// Collects the result of whichever [`JsStart`] began the evaluation.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
type JsFinish = fn(
    std::ptr::NonNull<webkitgtk::WebKitWebView>,
    *mut gtk4::gio::ffi::GAsyncResult,
) -> Result<*mut webkitgtk::JSCValue, String>;

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
struct JsEvalData {
    state: Rc<RefCell<PendingState<Result<Str, Str>>>>,
    /// Strong: `finish` dereferences the web view, so the view has to outlive the
    /// evaluation it was asked for.
    view: Widget,
    finish: JsFinish,
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
fn settle<T>(state: &Rc<RefCell<PendingState<T>>>, result: T) {
    let mut state = state.borrow_mut();
    state.result = Some(result);
    if let Some(waker) = state.waker.take() {
        waker.wake();
    }
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
unsafe extern "C" fn on_javascript_evaluated(
    _source_object: *mut gtk4::glib::gobject_ffi::GObject,
    result: *mut gtk4::gio::ffi::GAsyncResult,
    user_data: *mut std::ffi::c_void,
) {
    let data = unsafe { Box::from_raw(user_data.cast::<JsEvalData>()) };
    let outcome = match (data.finish)(webview_ptr(&data.view), result) {
        Ok(value) => {
            let output = webkitgtk::jsc_value_to_rust(value);
            webkitgtk::unref_jsc_value(value);
            Ok(output)
        }
        Err(err) => Err(Str::from(err)),
    };
    settle(&data.state, outcome);
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
unsafe extern "C" fn on_cookies_queried(
    _source_object: *mut gtk4::glib::gobject_ffi::GObject,
    result: *mut gtk4::gio::ffi::GAsyncResult,
    user_data: *mut std::ffi::c_void,
) {
    let data = unsafe { Box::from_raw(user_data.cast::<CookieQueryData>()) };
    let cookies = match webkitgtk::get_cookies_finish(data.cookie_manager, result) {
        Ok(list) => {
            let cookies = collect_cookies(list);
            webkitgtk::free_cookie_list(list);
            cookies
        }
        Err(err) => {
            data.shared
                .emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
                    err,
                ))));
            Vec::new()
        }
    };
    settle(&data.state, cookies);
}

/// Reads a `GList` of `SoupCookie` into the shared cookie type.
///
/// A cookie WebKit hands back that this parser rejects is reported and skipped:
/// one malformed cookie in the store used to abort the application from inside a
/// getter.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
fn collect_cookies(list: *mut gtk4::glib::ffi::GList) -> Vec<Cookie<'static>> {
    let mut cookies = Vec::new();
    let mut node = list;
    while !node.is_null() {
        let cookie = unsafe { (*node).data.cast::<webkitgtk::SoupCookie>() };
        if let Some(header) = webkitgtk::cookie_to_set_cookie_header(cookie) {
            match Cookie::parse(header) {
                Ok(cookie) => cookies.push(cookie.into_owned()),
                Err(error) => {
                    tracing::warn!(%error, "skipping a cookie WebKitGTK returned that is not parseable");
                }
            }
        }
        node = unsafe { (*node).next };
    }
    cookies
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
unsafe extern "C" fn on_cookie_added(
    _source_object: *mut gtk4::glib::gobject_ffi::GObject,
    result: *mut gtk4::gio::ffi::GAsyncResult,
    user_data: *mut std::ffi::c_void,
) {
    let data = unsafe { Box::from_raw(user_data.cast::<CookieAddData>()) };
    if let Err(err) = webkitgtk::add_cookie_finish(data.cookie_manager, result) {
        data.shared
            .emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
                err,
            ))));
    }
    webkitgtk::free_cookie(data.cookie);
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
/// Receives one `waterui.invoke(...)` envelope from page script.
///
/// Page script reaches this transport directly, so a malformed envelope, an
/// origin outside the policy or an unknown handler name is rejected back to
/// JavaScript rather than being fatal.
unsafe extern "C" fn on_script_message_received(
    _manager: *mut webkitgtk::WebKitUserContentManager,
    value: *mut webkitgtk::JSCValue,
    user_data: *mut std::ffi::c_void,
) {
    let data = unsafe { &*(user_data.cast::<ScriptMessageData>()) };
    let Some(view) = data.view.upgrade() else {
        tracing::warn!("a WaterUI bridge call arrived after its web view was destroyed");
        return;
    };
    let envelope = webkitgtk::jsc_value_to_rust(value);
    let request = match bridge::Request::parse(envelope.as_str()) {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(%error, "page script sent a malformed WaterUI bridge request");
            return;
        }
    };

    if !document_may_use_bridge(&data.shared, &view) {
        tracing::warn!(
            handler = %request.name,
            "a document outside the bridge origin policy tried to call a WaterUI handler"
        );
        deliver_reply(
            &view,
            &bridge::Reply::failure("this document may not use the WaterUI bridge"),
            request.id,
        );
        return;
    }

    // Release the borrow before invoking: a handler may register or remove
    // handlers on the same web view.
    let handler = data
        .shared
        .handler_callbacks
        .borrow()
        .get(&request.name)
        .cloned();
    let Some(handler) = handler else {
        tracing::warn!(
            handler = %request.name,
            "page script called a WaterUI handler that is not registered"
        );
        deliver_reply(
            &view,
            &bridge::Reply::failure(&format!("no WaterUI handler named `{}`", request.name)),
            request.id,
        );
        return;
    };

    // Handlers are asynchronous, so the promise settles when the future
    // completes rather than when this callback returns.
    let future = handler(&request.payload);
    // Weak across the await: a handler is free to await while the view it was
    // called from is torn down, and the reply then has nowhere to go. Carrying a
    // raw `WebKitWebView` pointer through the await evaluated JavaScript in freed
    // memory instead.
    let view = data.view.clone();
    executor_core::spawn_local(async move {
        let reply = match future.await {
            Ok(reply) => bridge::Reply::from(reply),
            Err(message) => bridge::Reply::Failure(message),
        };
        let Some(view) = view.upgrade() else {
            tracing::warn!(
                "a WaterUI handler answered after its web view was destroyed; dropping the reply"
            );
            return;
        };
        deliver_reply(&view, &reply, request.id);
    })
    .detach();
}

/// Whether the document now loaded in `view` may reach the bridge.
///
/// The origin comes from `WebKit`'s own `WebKitSecurityOrigin`, and the decision
/// from the shared [`OriginPolicy`](waterui_webview::OriginPolicy), so this
/// backend does not get to invent its own idea of what two origins being equal
/// means. No policy installed denies: the policy is installed before the first
/// handler exists, so a call arriving without one cannot be authenticated.
///
/// WebKitGTK reports script messages without the frame that sent them, so this
/// authenticates the document the view is showing. Subframe content is kept away
/// from the bridge by injecting the transport into the top frame only, and only
/// into documents the policy's URI patterns admit.
#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
fn document_may_use_bridge(shared: &SharedState, view: &Widget) -> bool {
    let Some(policy) = shared.bridge_origins.borrow().clone() else {
        return false;
    };
    let uri = webkitgtk::current_uri(webview_ptr(view));
    let Some(origin) = webkitgtk::security_origin(&uri) else {
        // An opaque origin — `about:blank`, `data:`, a sandboxed document — cannot
        // be authenticated, so it reaches nothing.
        return false;
    };
    policy.allows_origin(&origin)
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
unsafe extern "C" fn on_decide_policy(
    _web_view: *mut webkitgtk::WebKitWebView,
    decision: *mut webkitgtk::WebKitPolicyDecision,
    decision_type: i32,
    user_data: *mut std::ffi::c_void,
) -> gtk4::glib::ffi::gboolean {
    if decision_type != webkitgtk::WEBKIT_POLICY_DECISION_TYPE_RESPONSE {
        return 0;
    }

    let data = unsafe { &*(user_data.cast::<DecidePolicyData>()) };
    if data.shared.redirects_enabled.borrow().get() {
        return 0;
    }

    let response = webkitgtk::response_from_decision(decision);
    assert!(
        !response.is_null(),
        "on_decide_policy: response_from_decision returned null for response decision type"
    );

    let status = webkitgtk::response_status(response);
    if !(300..400).contains(&status) {
        return 0;
    }

    let from = webkitgtk::response_uri(response);
    let to = webkitgtk::response_redirect_target(response).unwrap_or_else(|| from.clone());
    data.shared.emit(WebViewEvent::Redirect {
        from: GtkWebViewHandle::parse_url(&from),
        to: GtkWebViewHandle::parse_url(&to),
    });
    webkitgtk::policy_ignore(decision);
    1
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
unsafe extern "C" fn on_load_failed(
    _web_view: *mut webkitgtk::WebKitWebView,
    _load_event: i32,
    _failing_uri: *const std::ffi::c_char,
    error: *mut gtk4::glib::ffi::GError,
    user_data: *mut std::ffi::c_void,
) -> gtk4::glib::ffi::gboolean {
    let data = unsafe { &*(user_data.cast::<LoadFailedData>()) };
    assert!(
        !error.is_null(),
        "on_load_failed: WebKit passed a null GError pointer"
    );
    let message = webkitgtk::cstr_to_string(unsafe { (*error).message });
    data.shared
        .emit(WebViewEvent::Error(WebViewError::LoadFailed(Str::from(
            message,
        ))));
    0
}

#[cfg(all(
    feature = "webkitgtk",
    gtk_webkitgtk_link_available,
    unix,
    not(target_os = "macos")
))]
unsafe extern "C" fn on_load_failed_with_tls_errors(
    _web_view: *mut webkitgtk::WebKitWebView,
    failing_uri: *const std::ffi::c_char,
    _certificate: *mut std::ffi::c_void,
    errors: u32,
    user_data: *mut std::ffi::c_void,
) -> gtk4::glib::ffi::gboolean {
    let data = unsafe { &*(user_data.cast::<TlsFailedData>()) };
    let uri = webkitgtk::cstr_to_string(failing_uri);
    let message = format!("TLS error flags: 0x{errors:08x}");
    data.shared.emit(WebViewEvent::Error(WebViewError::Ssl {
        url: GtkWebViewHandle::parse_url(&uri),
        message: Str::from(message),
    }));
    1
}
