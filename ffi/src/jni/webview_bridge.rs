//! JNI bridge for Android WebView integration.
//!
//! This module provides:
//! - A native WebView handle implementation (function pointers) for WaterUI's WebViewController
//! - JNI callbacks invoked by Kotlin WebView wrappers (events, JS messaging, JS results)

extern crate alloc;
extern crate std;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::string::ToString;

use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat, jint, jlong, jobject};
use jni::{JNIEnv, JavaVM};

use crate::closure::WuiFn;
use crate::components::webview::{
    FfiWebViewHandle, WuiJsCallback, WuiScriptInjectionTime, WuiStringCallback, WuiWebViewEvent,
    WuiWebViewEventType, WuiWebViewHandle, WuiWebViewMessage,
};
use crate::reactive::WuiComputed;
use crate::{IntoFFI, IntoRust, WuiStr};
use waterui_webview::{CustomWebViewController, WebViewController, WebViewHandle};

use std::{collections::HashMap, sync::Arc};

pub struct AndroidWebViewHandle {
    jvm: Arc<JavaVM>,
    pub wrapper: GlobalRef,
    pub event_callback: Option<Rc<WuiFn<WuiWebViewEvent>>>,
    pub handlers: HashMap<String, Rc<WuiFn<WuiWebViewMessage>>>,
}

impl AndroidWebViewHandle {
    fn with_env<R>(&self, f: impl FnOnce(&mut JNIEnv) -> R) -> R {
        let mut env = self
            .jvm
            .attach_current_thread()
            .expect("AndroidWebViewHandle failed to attach its JVM thread");
        f(&mut env)
    }
}

struct AndroidWebViewFactory {
    jvm: Arc<JavaVM>,
    factory: GlobalRef,
}

impl AndroidWebViewFactory {
    fn with_env<R>(&self, f: impl FnOnce(&mut JNIEnv) -> R) -> R {
        let mut env = self
            .jvm
            .attach_current_thread()
            .expect("AndroidWebViewFactory failed to attach its JVM thread");
        f(&mut env)
    }

    fn create_webview(&self) -> WuiWebViewHandle {
        self.with_env(|env| {
            let wrapper_obj = env
                .call_method(
                    &self.factory,
                    "create",
                    "()Ldev/waterui/android/components/WebViewWrapper;",
                    &[],
                )
                .expect("Android WebViewFactory.create failed")
                .l()
                .expect("Android WebViewFactory.create did not return a WebViewWrapper");

            let wrapper_ref = env
                .new_global_ref(wrapper_obj)
                .expect("Failed to retain WebViewWrapper");

            let handle = Box::new(AndroidWebViewHandle {
                jvm: Arc::clone(&self.jvm),
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
}

impl CustomWebViewController for AndroidWebViewFactory {
    fn open(&self) -> impl WebViewHandle {
        FfiWebViewHandle::new(self.create_webview())
    }
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
    handle.with_env(|env| {
        env.call_method(&handle.wrapper, "goBack", "()V", &[])
            .expect("webview_go_back: failed to call WebViewWrapper.goBack()");
    });
}

unsafe extern "C" fn webview_go_forward(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    handle.with_env(|env| {
        env.call_method(&handle.wrapper, "goForward", "()V", &[])
            .expect("webview_go_forward: failed to call WebViewWrapper.goForward()");
    });
}

unsafe extern "C" fn webview_go_to(data: *mut (), url: WuiStr) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let url = wui_str_to_string(url);
    handle.with_env(|env| {
        let jurl = java_string(env, &url);
        env.call_method(
            &handle.wrapper,
            "goTo",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jurl)],
        )
        .expect("webview_go_to: failed to call WebViewWrapper.goTo(String)");
    });
}

unsafe extern "C" fn webview_stop(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    handle.with_env(|env| {
        env.call_method(&handle.wrapper, "stop", "()V", &[])
            .expect("webview_stop: failed to call WebViewWrapper.stop()");
    });
}

unsafe extern "C" fn webview_refresh(data: *mut ()) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    handle.with_env(|env| {
        env.call_method(&handle.wrapper, "refresh", "()V", &[])
            .expect("webview_refresh: failed to call WebViewWrapper.refresh()");
    });
}

unsafe extern "C" fn webview_can_go_back(data: *const ()) -> bool {
    let handle = unsafe { &*data.cast::<AndroidWebViewHandle>() };
    handle.with_env(|env| {
        env.call_method(&handle.wrapper, "canGoBack", "()Z", &[])
            .expect("webview_can_go_back: failed to call WebViewWrapper.canGoBack()")
            .z()
            .expect("webview_can_go_back: canGoBack() did not return boolean")
    })
}

unsafe extern "C" fn webview_can_go_forward(data: *const ()) -> bool {
    let handle = unsafe { &*data.cast::<AndroidWebViewHandle>() };
    handle.with_env(|env| {
        env.call_method(&handle.wrapper, "canGoForward", "()Z", &[])
            .expect("webview_can_go_forward: failed to call WebViewWrapper.canGoForward()")
            .z()
            .expect("webview_can_go_forward: canGoForward() did not return boolean")
    })
}

unsafe extern "C" fn webview_set_user_agent(data: *mut (), user_agent: WuiStr) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let ua = wui_str_to_string(user_agent);
    handle.with_env(|env| {
        let jua = java_string(env, &ua);
        env.call_method(
            &handle.wrapper,
            "setUserAgent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jua)],
        )
        .expect("webview_set_user_agent: failed to call WebViewWrapper.setUserAgent(String)");
    });
}

unsafe extern "C" fn webview_set_redirects_enabled(data: *mut (), enabled: *mut WuiComputed<bool>) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    handle.with_env(|env| {
        env.call_method(
            &handle.wrapper,
            "setRedirectsEnabled",
            "(J)V",
            &[JValue::Long(enabled as jlong)],
        )
        .expect(
            "webview_set_redirects_enabled: failed to transfer redirect signal to WebViewWrapper",
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
    handle.with_env(|env| {
        let jscript = java_string(env, &script);
        env.call_method(
            &handle.wrapper,
            "injectScript",
            "(Ljava/lang/String;I)V",
            &[JValue::Object(&jscript), JValue::Int(time as jint)],
        )
        .expect("webview_inject_script: failed to call WebViewWrapper.injectScript(String, int)");
    });
}

unsafe extern "C" fn webview_watch(data: *mut (), callback: WuiFn<WuiWebViewEvent>) {
    let handle = unsafe { &mut *data.cast::<AndroidWebViewHandle>() };
    handle.event_callback = Some(Rc::new(callback));
    let handle_ptr = handle as *mut AndroidWebViewHandle as jlong;
    let wrapper = handle.wrapper.clone();

    handle.with_env(|env| {
        let cb_class = env
            .find_class("dev/waterui/android/components/NativeWebViewEventCallback")
            .expect("NativeWebViewEventCallback class not found");
        let cb_obj = env
            .new_object(cb_class, "(J)V", &[JValue::Long(handle_ptr)])
            .expect("Failed to create NativeWebViewEventCallback");

        env.call_method(
            &wrapper,
            "setEventCallback",
            "(Ldev/waterui/android/components/WebViewEventCallback;)V",
            &[JValue::Object(&cb_obj)],
        )
        .expect("webview_watch: failed to call WebViewWrapper.setEventCallback(callback)");
    });
}

unsafe extern "C" fn webview_add_handler(
    data: *mut (),
    name: WuiStr,
    handler: WuiFn<WuiWebViewMessage>,
) {
    let handle = unsafe { &mut *data.cast::<AndroidWebViewHandle>() };
    let name = wui_str_to_string(name);
    handle.handlers.insert(name.clone(), Rc::new(handler));
    let handle_ptr = handle as *mut AndroidWebViewHandle as jlong;
    let wrapper = handle.wrapper.clone();

    handle.with_env(|env| {
        let jname = java_string(env, &name);
        env.call_method(
            &wrapper,
            "addHandler",
            "(Ljava/lang/String;J)V",
            &[JValue::Object(&jname), JValue::Long(handle_ptr)],
        )
        .expect("webview_add_handler: failed to call WebViewWrapper.addHandler(String, long)");
    });
}

unsafe extern "C" fn webview_remove_handler(data: *mut (), name: WuiStr) {
    let handle = unsafe { &mut *data.cast::<AndroidWebViewHandle>() };
    let name = wui_str_to_string(name);

    handle.with_env(|env| {
        let jname = java_string(env, &name);
        env.call_method(
            &handle.wrapper,
            "removeHandler",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jname)],
        )
        .expect("webview_remove_handler: failed to call WebViewWrapper.removeHandler(String)");
    });

    handle.handlers.remove(&name);
}

unsafe extern "C" fn webview_set_cookie(data: *mut (), cookie: WuiStr) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let cookie = wui_str_to_string(cookie);
    handle.with_env(|env| {
        let jcookie = java_string(env, &cookie);
        env.call_method(
            &handle.wrapper,
            "setCookie",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jcookie)],
        )
        .expect("webview_set_cookie: failed to call WebViewWrapper.setCookie(String)");
    });
}

unsafe extern "C" fn webview_get_cookies(data: *const (), callback: WuiStringCallback) {
    let handle = unsafe { &*data.cast::<AndroidWebViewHandle>() };
    let call_ptr = callback.call as usize as jlong;
    handle.with_env(|env| {
        env.call_method(
            &handle.wrapper,
            "getCookies",
            "(JJ)V",
            &[JValue::Long(callback.data as jlong), JValue::Long(call_ptr)],
        )
        .expect("webview_get_cookies: failed to call getCookies(callback)");
    });
}

unsafe extern "C" fn webview_run_javascript(
    data: *mut (),
    script: WuiStr,
    callback: WuiJsCallback,
) {
    let handle = unsafe { &*(data as *const AndroidWebViewHandle) };
    let script = wui_str_to_string(script);
    let call_ptr = callback.call as usize as jlong;
    handle.with_env(|env| {
        let jscript = java_string(env, &script);
        env.call_method(
            &handle.wrapper,
            "runJavaScript",
            "(Ljava/lang/String;JJ)V",
            &[
                JValue::Object(&jscript),
                JValue::Long(callback.data as jlong),
                JValue::Long(call_ptr),
            ],
        )
        .expect("webview_run_javascript: failed to call WebViewWrapper.runJavaScript");
    });
}

unsafe extern "C" fn webview_drop(data: *mut ()) {
    let mut handle = unsafe { Box::from_raw(data as *mut AndroidWebViewHandle) };

    handle.with_env(|env| {
        env.call_method(
            &handle.wrapper,
            "setEventCallback",
            "(Ldev/waterui/android/components/WebViewEventCallback;)V",
            &[JValue::Object(&JObject::null())],
        )
        .expect("webview_drop: failed to clear WebViewWrapper event callback");
        env.call_method(&handle.wrapper, "release", "()V", &[])
            .expect("webview_drop: failed to call WebViewWrapper.release()");
    });

    // Drop callbacks.
    handle.event_callback.take();
    handle.handlers.clear();
}

/// Installs an Android WebView controller that owns its Java VM capability.
///
/// # Safety
///
/// `wui_env` must point to a live WaterUI environment owned by the caller.
pub unsafe fn install_android_webview_controller(
    env: &JNIEnv,
    wui_env: *mut crate::WuiEnv,
    factory: JObject,
) {
    let controller = WebViewController::new(AndroidWebViewFactory {
        jvm: Arc::new(
            env.get_java_vm()
                .expect("WebView factory installation failed to access JavaVM"),
        ),
        factory: env
            .new_global_ref(factory)
            .expect("WebView factory installation failed to retain Java factory"),
    });
    let env = unsafe { crate::borrow_ffi_mut(wui_env) };
    env.0.insert(controller);
}

pub fn webview_native_view<'local>(env: &mut JNIEnv<'local>, handle_ptr: jlong) -> jobject {
    let handle = unsafe { &*(handle_ptr as *mut AndroidWebViewHandle) };
    env.call_method(
        &handle.wrapper,
        "getWebView",
        "()Landroid/webkit/WebView;",
        &[],
    )
    .expect("webview_native_view: failed to call WebViewWrapper.getWebView()")
    .l()
    .expect("webview_native_view: getWebView() did not return an object")
    .into_raw()
}

// =============================================================================
// Kotlin -> Rust callback trampolines
// =============================================================================

type JsCallbackFn = unsafe extern "C" fn(*mut (), bool, WuiStr);
type StringCallbackFn = unsafe extern "C" fn(*mut (), WuiStr);

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_components_WebViewWrapper_nativeCompleteCookies<
    'local,
>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    callback_data: jlong,
    callback_fn: jlong,
    result: JString<'local>,
) {
    let call: StringCallbackFn =
        unsafe { core::mem::transmute::<usize, StringCallbackFn>(callback_fn as usize) };
    let text: std::string::String = env
        .get_string(&result)
        .expect("WebViewWrapper.nativeCompleteCookies: result")
        .into();
    unsafe {
        call(
            callback_data as *mut (),
            waterui::Str::from(text).into_ffi(),
        );
    }
}

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
    jvm: Arc<JavaVM>,
    wrapper: GlobalRef,
    request_id: String,
}

unsafe extern "C" fn reply_call(data: *mut (), success: bool, payload_b64: WuiStr) {
    let ctx = unsafe { Box::from_raw(data as *mut ReplyCtx) };
    let payload: waterui::Str = unsafe { payload_b64.into_rust() };
    let payload = payload.as_str().to_string();

    let mut env = ctx
        .jvm
        .attach_current_thread()
        .expect("WebView reply failed to attach its JVM thread");
    {
        let env = &mut env;
        let jreq = java_string(env, &ctx.request_id);
        let jpayload = java_string(env, &payload);
        env.call_method(
            &ctx.wrapper,
            "resolveMessage",
            "(Ljava/lang/String;ZLjava/lang/String;)V",
            &[
                JValue::Object(&jreq),
                JValue::Bool(if success { 1 } else { 0 }),
                JValue::Object(&jpayload),
            ],
        )
        .expect(
            "reply_call: failed to call WebViewWrapper.resolveMessage(String, boolean, String)",
        );
    }
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
    let name: std::string::String = env
        .get_string(&name)
        .expect("webview.native_on_message received an invalid name string")
        .into();
    let request_id: std::string::String = env
        .get_string(&request_id)
        .expect("webview.native_on_message received an invalid requestId string")
        .into();
    let payload_base64: std::string::String = env
        .get_string(&payload_base64)
        .expect("webview.native_on_message received an invalid payload string")
        .into();

    let (handler, jvm, wrapper) =
        {
            let handle = unsafe { &*(native_ptr as *const AndroidWebViewHandle) };
            (
                Rc::clone(handle.handlers.get(&name).unwrap_or_else(|| {
                    panic!("webview.native_on_message missing handler '{name}'")
                })),
                Arc::clone(&handle.jvm),
                handle.wrapper.clone(),
            )
        };

    let reply_wrapper = env
        .new_global_ref(wrapper.as_obj())
        .expect("webview.native_on_message failed to clone wrapper ref");
    let reply_ctx = Box::new(ReplyCtx {
        jvm,
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
    let callback = unsafe { &*(native_ptr as *const AndroidWebViewHandle) }
        .event_callback
        .as_ref()
        .cloned()
        .expect("webview.native_on_event missing registered Rust callback");

    let event_type = match event_type {
        1 => WuiWebViewEventType::WillNavigate,
        2 => WuiWebViewEventType::Loading,
        3 => WuiWebViewEventType::Loaded,
        4 => WuiWebViewEventType::Redirect,
        5 => WuiWebViewEventType::SslError,
        6 => WuiWebViewEventType::Error,
        7 => WuiWebViewEventType::StateChanged,
        _ => panic!("webview.native_on_event received unknown event type {event_type}"),
    };

    fn take_java_string(env: &mut JNIEnv, value: &JString, field: &'static str) -> *mut WuiStr {
        let value: std::string::String = env
            .get_string(value)
            .unwrap_or_else(|_| panic!("webview.native_on_event requires {field}"))
            .into();
        Box::into_raw(Box::new(waterui::Str::from(value).into_ffi()))
    }

    let (url, url2, message) = match event_type {
        WuiWebViewEventType::WillNavigate => (
            take_java_string(&mut env, &url, "url"),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        ),
        WuiWebViewEventType::Redirect => (
            take_java_string(&mut env, &url, "url"),
            take_java_string(&mut env, &url2, "url2"),
            core::ptr::null_mut(),
        ),
        WuiWebViewEventType::SslError => (
            take_java_string(&mut env, &url, "url"),
            core::ptr::null_mut(),
            take_java_string(&mut env, &message, "message"),
        ),
        WuiWebViewEventType::Error => (
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            take_java_string(&mut env, &message, "message"),
        ),
        WuiWebViewEventType::Loading
        | WuiWebViewEventType::Loaded
        | WuiWebViewEventType::StateChanged => (
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        ),
        WuiWebViewEventType::None => panic!("Android WebView cannot emit the None event type"),
    };

    let event = WuiWebViewEvent {
        event_type,
        url,
        url2,
        message,
        progress,
        can_go_back: can_go_back != 0,
        can_go_forward: can_go_forward != 0,
    };

    callback.call(event);
}
