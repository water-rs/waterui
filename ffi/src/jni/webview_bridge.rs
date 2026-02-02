//! JNI bridge for Android WebView integration.
//!
//! This module provides:
//! - A native WebView handle implementation (function pointers) for WaterUI's WebViewController
//! - JNI callbacks invoked by Kotlin WebView wrappers (events, JS messaging, JS results)

extern crate alloc;
extern crate std;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;

use jni::JNIEnv;
use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat, jint, jlong, jobject};

use crate::closure::WuiFn;
use crate::components::webview::{
    WuiJsCallback, WuiScriptInjectionTime, WuiWebViewEvent, WuiWebViewEventType, WuiWebViewHandle,
    WuiWebViewMessage,
};
use crate::{IntoFFI, IntoRust, WuiStr};

use std::collections::HashMap;

pub struct AndroidWebViewHandle {
    pub wrapper: GlobalRef,
    pub event_callback: Option<WuiFn<WuiWebViewEvent>>,
    pub handlers: HashMap<String, WuiFn<WuiWebViewMessage>>,
}

#[inline]
fn jlong_as_ptr<T>(v: jlong) -> *mut T {
    v as *mut T
}

#[inline]
fn ptr_as_jlong<T>(p: *mut T) -> jlong {
    p as jlong
}

fn wui_str_to_string(s: WuiStr) -> String {
    let s: waterui::Str = unsafe { s.into_rust() };
    s.as_str().to_string()
}

fn java_string<'local>(env: &mut JNIEnv<'local>, s: &str) -> JString<'local> {
    env.new_string(s).expect("Failed to create Java string")
}

// =============================================================================
// WebView handle vtable (called from Rust via FFI)
// =============================================================================

unsafe extern "C" fn webview_go_back(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        let _ = env.call_method(&handle.wrapper, "goBack", "()V", &[]);
    });
}

unsafe extern "C" fn webview_go_forward(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        let _ = env.call_method(&handle.wrapper, "goForward", "()V", &[]);
    });
}

unsafe extern "C" fn webview_go_to(data: *mut (), url: WuiStr) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let url = wui_str_to_string(url);
    crate::jni::with_jni_env(|env| {
        let jurl = java_string(env, &url);
        let _ = env.call_method(
            &handle.wrapper,
            "goTo",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jurl)],
        );
    });
}

unsafe extern "C" fn webview_stop(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        let _ = env.call_method(&handle.wrapper, "stop", "()V", &[]);
    });
}

unsafe extern "C" fn webview_refresh(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        let _ = env.call_method(&handle.wrapper, "refresh", "()V", &[]);
    });
}

unsafe extern "C" fn webview_can_go_back(data: *const ()) -> bool {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        env.call_method(&handle.wrapper, "canGoBack", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}

unsafe extern "C" fn webview_can_go_forward(data: *const ()) -> bool {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        env.call_method(&handle.wrapper, "canGoForward", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}

unsafe extern "C" fn webview_set_user_agent(data: *mut (), user_agent: WuiStr) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let ua = wui_str_to_string(user_agent);
    crate::jni::with_jni_env(|env| {
        let jua = java_string(env, &ua);
        let _ = env.call_method(
            &handle.wrapper,
            "setUserAgent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jua)],
        );
    });
}

unsafe extern "C" fn webview_set_redirects_enabled(data: *mut (), enabled: bool) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        let _ = env.call_method(
            &handle.wrapper,
            "setRedirectsEnabled",
            "(Z)V",
            &[JValue::Bool(if enabled { 1 } else { 0 })],
        );
    });
}

unsafe extern "C" fn webview_inject_script(
    data: *mut (),
    script: WuiStr,
    time: WuiScriptInjectionTime,
) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let script = wui_str_to_string(script);
    crate::jni::with_jni_env(|env| {
        let jscript = java_string(env, &script);
        let _ = env.call_method(
            &handle.wrapper,
            "injectScript",
            "(Ljava/lang/String;I)V",
            &[JValue::Object(&jscript), JValue::Int(time as jint)],
        );
    });
}

unsafe extern "C" fn webview_watch(data: *mut (), callback: WuiFn<WuiWebViewEvent>) {
    let handle = unsafe { &mut *(data as *mut AndroidWebViewHandle) };
    handle.event_callback = Some(callback);
    let handle_ptr = ptr_as_jlong(handle as *mut AndroidWebViewHandle);
    let wrapper = handle.wrapper.clone();

    crate::jni::with_jni_env(|env| {
        let cb_class = env
            .find_class("dev/waterui/android/components/NativeWebViewEventCallback")
            .expect("NativeWebViewEventCallback class not found");
        let cb_obj = env
            .new_object(cb_class, "(J)V", &[JValue::Long(handle_ptr)])
            .expect("Failed to create NativeWebViewEventCallback");

        let _ = env.call_method(
            &wrapper,
            "setEventCallback",
            "(Ldev/waterui/android/components/WebViewEventCallback;)V",
            &[JValue::Object(&cb_obj)],
        );
    });
}

unsafe extern "C" fn webview_add_handler(
    data: *mut (),
    name: WuiStr,
    handler: WuiFn<WuiWebViewMessage>,
) {
    let handle = unsafe { &mut *(data as *mut AndroidWebViewHandle) };
    let name = wui_str_to_string(name);
    handle.handlers.insert(name.clone(), handler);
    let handle_ptr = ptr_as_jlong(handle as *mut AndroidWebViewHandle);
    let wrapper = handle.wrapper.clone();

    crate::jni::with_jni_env(|env| {
        let jname = java_string(env, &name);
        let _ = env.call_method(
            &wrapper,
            "addHandler",
            "(Ljava/lang/String;J)V",
            &[JValue::Object(&jname), JValue::Long(handle_ptr)],
        );
    });
}

unsafe extern "C" fn webview_remove_handler(data: *mut (), name: WuiStr) {
    let handle = unsafe { &mut *(data as *mut AndroidWebViewHandle) };
    let name = wui_str_to_string(name);

    crate::jni::with_jni_env(|env| {
        let jname = java_string(env, &name);
        let _ = env.call_method(
            &handle.wrapper,
            "removeHandler",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jname)],
        );
    });

    handle.handlers.remove(&name);
}

unsafe extern "C" fn webview_set_cookie(data: *mut (), cookie: WuiStr) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let cookie = wui_str_to_string(cookie);
    crate::jni::with_jni_env(|env| {
        let jcookie = java_string(env, &cookie);
        let _ = env.call_method(
            &handle.wrapper,
            "setCookie",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jcookie)],
        );
    });
}

unsafe extern "C" fn webview_get_cookies(data: *const ()) -> WuiStr {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    crate::jni::with_jni_env(|env| {
        let Ok(ret) = env.call_method(&handle.wrapper, "getCookies", "()Ljava/lang/String;", &[])
        else {
            return "".into_ffi();
        };
        let Ok(obj) = ret.l() else {
            return "".into_ffi();
        };
        let jstr = JString::from(obj);
        let text: std::string::String = env.get_string(&jstr).map(|s| s.into()).unwrap_or_default();
        waterui::Str::from(text).into_ffi()
    })
}

unsafe extern "C" fn webview_run_javascript(
    data: *mut (),
    script: WuiStr,
    callback: WuiJsCallback,
) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let script = wui_str_to_string(script);
    let call_ptr = callback.call as usize as jlong;
    crate::jni::with_jni_env(|env| {
        let jscript = java_string(env, &script);
        let _ = env.call_method(
            &handle.wrapper,
            "runJavaScript",
            "(Ljava/lang/String;JJ)V",
            &[
                JValue::Object(&jscript),
                JValue::Long(callback.data as jlong),
                JValue::Long(call_ptr),
            ],
        );
    });
}

unsafe extern "C" fn webview_drop(data: *mut ()) {
    if data.is_null() {
        return;
    }
    let mut handle = unsafe { Box::from_raw(data as *mut AndroidWebViewHandle) };

    // Best-effort cleanup on the Java side.
    crate::jni::with_jni_env(|env| {
        let _ = env.call_method(
            &handle.wrapper,
            "setEventCallback",
            "(Ldev/waterui/android/components/WebViewEventCallback;)V",
            &[JValue::Object(&JObject::null())],
        );
        let _ = env.call_method(&handle.wrapper, "release", "()V", &[]);
    });

    // Drop callbacks.
    handle.event_callback.take();
    handle.handlers.clear();
}

pub unsafe extern "C" fn jni_create_webview() -> WuiWebViewHandle {
    crate::jni::with_jni_env(|env| {
        let manager_class = env
            .find_class("dev/waterui/android/components/WebViewManager")
            .expect("WebViewManager class not found");

        let wrapper_obj = env
            .call_static_method(
                manager_class,
                "create",
                "()Ldev/waterui/android/components/WebViewWrapper;",
                &[],
            )
            .expect("WebViewManager.create failed")
            .l()
            .expect("WebViewManager.create should return object");

        let wrapper_ref = env
            .new_global_ref(wrapper_obj)
            .expect("Failed to create global ref for WebViewWrapper");

        let handle = Box::new(AndroidWebViewHandle {
            wrapper: wrapper_ref,
            event_callback: None,
            handlers: HashMap::new(),
        });
        let handle_ptr = Box::into_raw(handle) as *mut ();

        WuiWebViewHandle {
            data: handle_ptr,
            go_back: webview_go_back,
            go_forward: webview_go_forward,
            go_to: webview_go_to,
            stop: webview_stop,
            refresh: webview_refresh,
            can_go_back: webview_can_go_back,
            can_go_forward: webview_can_go_forward,
            set_user_agent: webview_set_user_agent,
            set_redirects_enabled: webview_set_redirects_enabled,
            inject_script: webview_inject_script,
            watch: webview_watch,
            add_handler: Some(webview_add_handler),
            remove_handler: Some(webview_remove_handler),
            set_cookie: Some(webview_set_cookie),
            get_cookies: Some(webview_get_cookies),
            run_javascript: webview_run_javascript,
            drop: webview_drop,
        }
    })
}

pub fn webview_native_view<'local>(env: &mut JNIEnv<'local>, handle_ptr: jlong) -> jobject {
    if handle_ptr == 0 {
        return core::ptr::null_mut();
    }
    let handle = unsafe { &*(jlong_as_ptr::<AndroidWebViewHandle>(handle_ptr)) };
    match env.call_method(
        &handle.wrapper,
        "getWebView",
        "()Landroid/webkit/WebView;",
        &[],
    ) {
        Ok(v) => v.l().map(|o| o.into_raw()).unwrap_or(core::ptr::null_mut()),
        Err(_) => core::ptr::null_mut(),
    }
}

// =============================================================================
// Kotlin -> Rust callback trampolines
// =============================================================================

type JsCallbackFn = unsafe extern "C" fn(*mut (), bool, WuiStr);

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_components_WebViewWrapper_nativeCompleteJsResult<
    'local,
>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    callback_data: jlong,
    callback_fn: jlong,
    success: jboolean,
    result: JString<'local>,
) {
    if callback_fn == 0 {
        return;
    }

    let call: JsCallbackFn =
        unsafe { core::mem::transmute::<usize, JsCallbackFn>(callback_fn as usize) };

    let text: std::string::String = env
        .get_string(&result)
        .expect("WebViewWrapper.nativeCompleteJsResult: result")
        .into();
    let wui_str = waterui::Str::from(text).into_ffi();

    unsafe {
        call(callback_data as *mut (), success != 0, wui_str);
    }
}

struct ReplyCtx {
    wrapper: GlobalRef,
    request_id: String,
}

unsafe extern "C" fn reply_call(data: *mut (), success: bool, payload_b64: WuiStr) {
    let ctx = unsafe { Box::from_raw(data as *mut ReplyCtx) };
    let payload: waterui::Str = unsafe { payload_b64.into_rust() };
    let payload = payload.as_str().to_string();

    crate::jni::with_jni_env(|env| {
        let jreq = java_string(env, &ctx.request_id);
        let jpayload = java_string(env, &payload);
        let _ = env.call_method(
            &ctx.wrapper,
            "resolveMessage",
            "(Ljava/lang/String;ZLjava/lang/String;)V",
            &[
                JValue::Object(&jreq),
                JValue::Bool(if success { 1 } else { 0 }),
                JValue::Object(&jpayload),
            ],
        );
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_components_WebViewWrapper_nativeOnMessage<
    'local,
>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    native_ptr: jlong,
    name: JString<'local>,
    request_id: JString<'local>,
    payload_base64: JString<'local>,
) {
    if native_ptr == 0 {
        return;
    }

    let handle = unsafe { &mut *(jlong_as_ptr::<AndroidWebViewHandle>(native_ptr)) };
    let name: std::string::String = env.get_string(&name).expect("name").into();
    let request_id: std::string::String = env.get_string(&request_id).expect("requestId").into();
    let payload_base64: std::string::String =
        env.get_string(&payload_base64).expect("payload").into();

    let Some(handler) = handle.handlers.get(&name) else {
        return;
    };

    let reply_wrapper = env
        .new_global_ref(handle.wrapper.as_obj())
        .expect("Failed to clone WebViewWrapper global ref");
    let reply_ctx = Box::new(ReplyCtx {
        wrapper: reply_wrapper,
        request_id,
    });
    let reply_ctx_ptr = Box::into_raw(reply_ctx) as *mut ();

    let msg = WuiWebViewMessage {
        payload_base64: waterui::Str::from(payload_base64).into_ffi(),
        reply: WuiJsCallback {
            data: reply_ctx_ptr,
            call: reply_call,
        },
    };

    handler.call(msg);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_components_NativeWebViewEventCallback_nativeOnEvent<
    'local,
>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    native_ptr: jlong,
    event_type: jint,
    url: JString<'local>,
    url2: JString<'local>,
    message: JString<'local>,
    progress: jfloat,
    can_go_back: jboolean,
    can_go_forward: jboolean,
) {
    if native_ptr == 0 {
        return;
    }
    let handle = unsafe { &mut *(jlong_as_ptr::<AndroidWebViewHandle>(native_ptr)) };
    let Some(cb) = handle.event_callback.as_ref() else {
        return;
    };

    let url: std::string::String = env.get_string(&url).map(|s| s.into()).unwrap_or_default();
    let url2: std::string::String = env.get_string(&url2).map(|s| s.into()).unwrap_or_default();
    let message: std::string::String = env
        .get_string(&message)
        .map(|s| s.into())
        .unwrap_or_default();

    let event_type = match event_type {
        1 => WuiWebViewEventType::WillNavigate,
        2 => WuiWebViewEventType::Loading,
        3 => WuiWebViewEventType::Loaded,
        4 => WuiWebViewEventType::Redirect,
        5 => WuiWebViewEventType::SslError,
        6 => WuiWebViewEventType::Error,
        7 => WuiWebViewEventType::StateChanged,
        _ => WuiWebViewEventType::None,
    };

    let event = WuiWebViewEvent {
        event_type,
        url: waterui::Str::from(url).into_ffi(),
        url2: waterui::Str::from(url2).into_ffi(),
        message: waterui::Str::from(message).into_ffi(),
        progress,
        can_go_back: can_go_back != 0,
        can_go_forward: can_go_forward != 0,
    };

    cb.call(event);
}
