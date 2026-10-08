//! The Android `WebView` bridge: `AndroidSystemWebViewController` mounts the
//! platform's `android.webkit.WebView` as a platform-view *instance* —
//! `WebView` in `dev.waterui.hydrolysis.webview.HydrolysisWebView` — and the
//! handle drives it over JNI.
//!
//! The Kotlin side owns the `WebView`'s callbacks; everything Rust cares about
//! arrives through the `native…` functions at the bottom of this file, the
//! same division `ffi`'s webview bridge draws against the Kotlin runtime. The
//! parts that need no JNI — origin-rule conversion, the cookie URL, script
//! composition, async-result parsing — live in [`super::android_protocol`]
//! so the host test suite exercises them.

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::format;
use std::rc::{Rc, Weak};
use std::string::{String, ToString};
use std::vec::Vec;

use executor_core::spawn_local;
use futures::channel::oneshot;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat, jint, jlong, jobject};
use jni::{JNIEnv, JavaVM};
use nami::Signal;
use nami::watcher::BoxWatcherGuard;
use waterui_core::{Computed, Str};
use waterui_webview::{
    ASSET_HTTPS_ORIGIN, AssetServer, BackendEvent, Cookie, CustomWebViewController,
    DOCUMENT_START_SCRIPT, OriginPolicy, ScriptInjectionTime, ScriptMessageHandler, Url,
    WatcherGuard, WatcherSet, WebViewConfig, WebViewError, WebViewEvent, WebViewHandle, assets,
    bridge,
};

use crate::runner::android::jni::{JniError, get_string, guard, guard_val};

use super::android_protocol::{
    ASYNC_CALL_STARTED, ASYNC_RESULT_OBJECT, BRIDGE_OBJECT, TRANSPORT_SCRIPT,
    androidx_origin_rules, compose_async_call, compose_document_end, cookie_url,
    parse_async_result, parse_cookie_header,
};

/// What `create` takes and returns over JNI: the session the wrapper
/// registers itself on, its own instance id, the Rust shared state Kotlin
/// calls back into, the asset-server pointer, and the two injected-object
/// names.
const CREATE_SIG: &str = "(Ldev/waterui/hydrolysis/HydrolysisSession;JJJLjava/lang/String;Ljava/lang/String;)Ldev/waterui/hydrolysis/webview/HydrolysisWebView;";

/// The Kotlin wrapper class, resolved at session-create time: a thread that
/// did not enter from Java cannot resolve app classes later.
const WRAPPER_CLASS: &str = "dev/waterui/hydrolysis/webview/HydrolysisWebView";

/// The class `nativeAssetRespond` constructs by name — its constructor is
/// part of the JNI contract the consumer `ProGuard` rules keep.
const ASSET_RESPONSE_CLASS: &str = "dev/waterui/hydrolysis/webview/AssetResponse";
const ASSET_RESPONSE_SIG: &str = "(ILjava/lang/String;[B)V";

/// A pending `run_javascript`/`call_async_javascript`/`get_cookies` call
/// waiting on its matching native, keyed by call id. The Kotlin runtime's
/// `JsCompletion` trio, flattened into one registry: Kotlin reports every
/// settlement — completed or abandoned — through `nativeJsResult`, and only
/// a live async call hears from `nativeAsyncResult` afterwards.
enum PendingCall {
    /// `run_javascript`: the synchronous `evaluateJavascript` reply is the
    /// script's JSON answer.
    JavaScript(oneshot::Sender<Result<Str, Str>>),
    /// `call_async_javascript`: a reply of [`ASYNC_CALL_STARTED`] means the
    /// promise was posted and the real result comes through
    /// `nativeAsyncResult`; anything else failed at launch.
    Async(oneshot::Sender<Result<Str, Str>>),
    /// `get_cookies`: the reply is the `CookieManager` header the native
    /// splits into pairs.
    Cookies(oneshot::Sender<Vec<Cookie<'static>>>),
}

/// The Rust half of one mounted `HydrolysisWebView`. Kotlin holds a leaked
/// `Box<Weak<SharedState>>` as `nativeHandle` and calls back through it; a
/// dead weak means `release()` already ran and the callback drops.
pub(super) struct SharedState {
    /// The VM replies and event dispatches attach through — the natives and
    /// the spawned handler tasks all run on the UI thread.
    vm: JavaVM,
    /// The Kotlin wrapper, installed by `open` once `create` returns. The
    /// bridge reply path needs it without the handle.
    wrapper: RefCell<Option<GlobalRef>>,
    /// Backend-event watchers — `watch` on the handle.
    watchers: WatcherSet<BackendEvent>,
    /// Named script-message handlers; `nativeOnBridgeMessage` dispatches.
    handlers: RefCell<HashMap<String, Rc<ScriptMessageHandler>>>,
    /// User-injected scripts in first-injection order — the push list is
    /// rebuilt on every `inject_script`, as `rebuildDocumentStartScripts`
    /// does in the Kotlin runtime.
    scripts: RefCell<Vec<(String, String, ScriptInjectionTime)>>,
    /// In-flight calls indexed by call id.
    pending: RefCell<HashMap<u64, PendingCall>>,
    /// The URL the view is committed to — what `webView.url` answered in the
    /// Kotlin runtime's `cookieUrl`, kept from the navigation events.
    current_url: RefCell<Option<String>>,
    /// Set at `create`: the asset origin the view answers, when it was opened
    /// with a server.
    asset_origin: Option<Url>,
}

impl SharedState {
    /// The state-event fan-out the macOS realization runs on `WKWebView`
    /// callbacks.
    fn emit(&self, event: &BackendEvent) {
        self.watchers.emit(event);
    }

    fn emit_webview(&self, event: WebViewEvent) {
        self.emit(&BackendEvent::Event(event));
    }

    /// `evaluateBridgeScript` — the fire-and-forget evaluation bridge replies
    /// take. Callable without a handle because the wrapper lives here.
    fn evaluate_bridge_script(&self, script: String) {
        let Some(wrapper) = self.wrapper.borrow().clone() else {
            tracing::warn!("android webview: a bridge reply arrived before the wrapper was set");
            return;
        };
        let mut env = self
            .vm
            .get_env()
            .expect("bridge replies run on the UI thread, which is attached");
        let script = env
            .new_string(script)
            .expect("a reply script is Java-safe UTF-8");
        env.call_method(
            wrapper.as_obj(),
            "evaluateBridgeScript",
            "(Ljava/lang/String;)V",
            &[JValue::Object(script.as_ref())],
        )
        .expect("android webview: evaluateBridgeScript failed");
    }

    /// Settle a pending call with a synchronous `evaluateJavascript`-style
    /// reply — the channel `nativeJsResult` and Kotlin's abandonment share.
    /// `ok`/`value` carry the JavaScript outcome for JavaScript calls, the
    /// failure message for abandoned calls, and the cookie header for
    /// `get_cookies`.
    fn settle_js(&self, call_id: u64, ok: bool, value: String) {
        match self.pending.borrow_mut().remove(&call_id) {
            Some(PendingCall::JavaScript(sender)) => {
                let _ = sender.send(if ok {
                    Ok(Str::from(value))
                } else {
                    Err(Str::from(value))
                });
            }
            Some(PendingCall::Async(sender)) => {
                if ok && value == ASYNC_CALL_STARTED {
                    // The promise posted: re-arm and wait for
                    // `nativeAsyncResult`, which carries the same id.
                    self.pending
                        .borrow_mut()
                        .insert(call_id, PendingCall::Async(sender));
                } else {
                    // A launch failure — a parse error, a page gone under the
                    // call, or Kotlin's abandonment of a live call.
                    let _ = sender.send(Err(Str::from(value)));
                }
            }
            Some(PendingCall::Cookies(sender)) => {
                let _ = sender.send(parse_cookie_header(&value));
            }
            None => tracing::debug!(
                "android webview: a reply for call {call_id} arrived with nothing pending"
            ),
        }
    }

    /// Settle an async call from its posted `{id, ok, value}` envelope.
    fn settle_async(&self, payload: &str) {
        let result = match parse_async_result(payload) {
            Ok(result) => result,
            Err(error) => {
                // A malformed envelope cannot identify its call; surface the
                // breach rather than guess.
                tracing::warn!("android webview: malformed async result: {error}");
                return;
            }
        };
        match self.pending.borrow_mut().remove(&result.id) {
            Some(PendingCall::Async(sender) | PendingCall::JavaScript(sender)) => {
                let value = Str::from(result.value);
                let _ = sender.send(if result.ok { Ok(value) } else { Err(value) });
            }
            Some(PendingCall::Cookies(_)) => {
                tracing::warn!(
                    "android webview: an async result addressed call {}, a cookie call",
                    result.id
                );
            }
            None => tracing::debug!(
                "android webview: an async result for call {} arrived with nothing pending",
                result.id
            ),
        }
    }

    /// `nativeOnBridgeMessage`: parse the envelope, dispatch the named
    /// handler on the executor, and push the reply into the page with
    /// `evaluateBridgeScript` — the ffi bridge's flow, minus its FFI shim.
    fn on_bridge_message(self: Rc<Self>, envelope: &str) {
        let request = match bridge::Request::parse(envelope) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, "android webview: a malformed bridge envelope");
                return;
            }
        };
        let request_id = request.id;
        let Some(handler) = self.handlers.borrow().get(&request.name).cloned() else {
            // Page script reached a name nothing registered for; it still
            // awaits an answer, and rejecting is the same shape the Kotlin
            // runtime gives it.
            tracing::warn!(
                handler = %request.name,
                "android webview: a bridge message named an unregistered handler"
            );
            self.evaluate_bridge_script(
                bridge::Reply::failure(&format!("no WaterUI handler named `{}`", request.name))
                    .resolve_script(request_id),
            );
            return;
        };
        // Handlers are asynchronous: the page's promise settles when the
        // future completes rather than when this callback returns.
        let weak = Rc::downgrade(&self);
        spawn_local(async move {
            let reply = match handler(&request.payload).await {
                Ok(reply) => bridge::Reply::from(reply),
                Err(message) => bridge::Reply::Failure(message),
            };
            if let Some(shared) = weak.upgrade() {
                shared.evaluate_bridge_script(reply.resolve_script(request_id));
            }
        })
        .detach();
    }
}

/// The controller the session's environment carries. `create` resolves the
/// wrapper class while still on the JNI thread and `open` builds wrappers
/// against it.
pub struct AndroidSystemWebViewController {
    vm: JavaVM,
    /// The `HydrolysisSession` — the object `HostBridge` calls back into, and
    /// the instance registry `create` registers the wrapper on.
    session: GlobalRef,
    webview_class: GlobalRef,
    /// Placement instance ids, owned by the controller rather than a static:
    /// a session's registry counts from its own zero.
    next_instance: Cell<u64>,
}

impl AndroidSystemWebViewController {
    /// Resolve the wrapper class and capture the session. `env` must be the
    /// JNI environment of `nativeCreateSession` — the only call that reaches
    /// app classes by name on every thread model.
    pub fn new(env: &mut JNIEnv<'_>, session: GlobalRef) -> Result<Self, JniError> {
        let class = env.find_class(WRAPPER_CLASS).map_err(JniError::from)?;
        let webview_class = env.new_global_ref(&class).map_err(JniError::from)?;
        Ok(Self {
            vm: env.get_java_vm().map_err(JniError::from)?,
            session,
            webview_class,
            next_instance: Cell::new(0),
        })
    }
}

impl CustomWebViewController for AndroidSystemWebViewController {
    fn open(&self, config: WebViewConfig) -> impl WebViewHandle {
        let instance = self.next_instance.get() + 1;
        self.next_instance.set(instance);

        // `config.asset_server` crosses as a leaked Box pointer Kotlin hands
        // back to `nativeAssetRespond`/`nativeFreeAssetServer`.
        let asset_server = config
            .asset_server
            .map_or(0, |server| Box::into_raw(Box::new(server)) as jlong);

        let mut env = self
            .vm
            .get_env()
            .expect("open must run on the UI thread, which is attached");
        let shared = Rc::new(SharedState {
            vm: env
                .get_java_vm()
                .map_err(JniError::from)
                .expect("a JNIEnv always yields its JavaVM"),
            wrapper: RefCell::new(None),
            watchers: WatcherSet::new(),
            handlers: RefCell::new(HashMap::new()),
            scripts: RefCell::new(Vec::new()),
            pending: RefCell::new(HashMap::new()),
            current_url: RefCell::new(None),
            asset_origin: (asset_server != 0).then(|| {
                ASSET_HTTPS_ORIGIN
                    .parse()
                    .expect("the asset origin is a constant")
            }),
        });
        let native_handle = Box::into_raw(Box::new(Rc::downgrade(&shared))) as jlong;

        let wrapper = create_wrapper(
            &mut env,
            &self.webview_class,
            &self.session,
            instance,
            native_handle,
            asset_server,
        )
        .unwrap_or_else(|error| {
            // Keep the leak surface honest on the failure path too.
            // SAFETY: `native_handle` was `Box::into_raw`'d at the top of
            // `open`; this is the only site that drops it, and only on the
            // path where Kotlin never saw the wrapper.
            unsafe {
                drop(Box::from_raw(native_handle as *mut Weak<SharedState>));
                if asset_server != 0 {
                    drop(Box::from_raw(asset_server as *mut AssetServer));
                }
            }
            panic!("android webview: HydrolysisWebView.create failed: {error:?}");
        });
        *shared.wrapper.borrow_mut() = Some(wrapper.clone());

        AndroidSystemWebViewHandle {
            inner: Rc::new(HandleInner {
                vm: env
                    .get_java_vm()
                    .expect("a JNIEnv always yields its JavaVM"),
                wrapper,
                shared,
                instance,
                native_handle,
                next_call: Cell::new(1),
                redirects_guard: RefCell::new(None),
            }),
        }
    }
}

/// Kotlin `HydrolysisWebView.create` — returns the wrapper as a `GlobalRef`.
fn create_wrapper(
    env: &mut JNIEnv<'_>,
    class: &GlobalRef,
    session: &GlobalRef,
    instance: u64,
    native_handle: jlong,
    asset_server: jlong,
) -> Result<GlobalRef, JniError> {
    let bridge_object = env.new_string(BRIDGE_OBJECT)?;
    let async_result_object = env.new_string(ASYNC_RESULT_OBJECT)?;
    let wrapper = env
        .call_static_method(
            class,
            "create",
            CREATE_SIG,
            &[
                JValue::Object(session.as_ref()),
                JValue::Long(jlong::try_from(instance).expect("an instance id fits a jlong")),
                JValue::Long(native_handle),
                JValue::Long(asset_server),
                JValue::Object(bridge_object.as_ref()),
                JValue::Object(async_result_object.as_ref()),
            ],
        )?
        .l()?;
    env.new_global_ref(&wrapper).map_err(JniError::from)
}

/// The live wrapper and the bookkeeping around it.
struct HandleInner {
    vm: JavaVM,
    wrapper: GlobalRef,
    shared: Rc<SharedState>,
    /// The placement-instance id Kotlin registered the wrapper under.
    instance: u64,
    /// The leaked `Box<Weak<SharedState>>` Kotlin calls back through; freed
    /// on `Drop` after `release()` zeroes its copy.
    native_handle: jlong,
    next_call: Cell<u64>,
    /// Keeps the `set_redirects_enabled` subscription alive.
    redirects_guard: RefCell<Option<BoxWatcherGuard>>,
}

impl HandleInner {
    fn env(&self) -> JNIEnv<'_> {
        self.vm
            .get_env()
            .expect("webview calls run on the UI thread, which is attached")
    }

    fn alloc_call(&self) -> u64 {
        let id = self.next_call.get();
        self.next_call.set(id + 1);
        id
    }

    /// `call_method` on the wrapper — the error path panics: these calls run
    /// on the UI thread inside view code, where a JNI failure is a bug, not a
    /// recoverable condition.
    fn call(&self, name: &'static str, sig: &'static str, args: &[JValue]) {
        self.env()
            .call_method(self.wrapper.as_obj(), name, sig, args)
            .unwrap_or_else(|error| panic!("android webview: {name} failed: {error}"));
    }

    fn call_bool(&self, name: &'static str, sig: &'static str, args: &[JValue]) -> bool {
        self.env()
            .call_method(self.wrapper.as_obj(), name, sig, args)
            .and_then(jni::objects::JValueGen::z)
            .unwrap_or_else(|error| panic!("android webview: {name} failed: {error}"))
    }

    /// One string argument — `loadUrl`, `setUserAgent`, `evaluate`.
    fn call_str(&self, name: &'static str, sig: &'static str, value: &str, tail: &[JValue]) {
        let value = self
            .env()
            .new_string(value)
            .expect("the argument is Java-safe UTF-8");
        let mut args: Vec<JValue> = Vec::with_capacity(tail.len() + 1);
        args.push(JValue::Object(value.as_ref()));
        args.extend_from_slice(tail);
        self.call(name, sig, &args);
    }

    /// Push a `String[]` argument to a one-argument setter.
    fn call_str_array(&self, name: &'static str, sig: &'static str, values: &[String]) {
        let mut env = self.env();
        let array = env
            .new_object_array(
                jint::try_from(values.len()).expect("a script list fits a jint"),
                "java/lang/String",
                JObject::null(),
            )
            .expect("android webview: allocating a string array failed");
        for (index, value) in values.iter().enumerate() {
            let value = env.new_string(value).expect("Java-safe UTF-8");
            env.set_object_array_element(
                &array,
                jint::try_from(index).expect("an index fits a jint"),
                &value,
            )
            .expect("android webview: writing a string array failed");
        }
        self.call(name, sig, &[JValue::Object(&array)]);
    }

    /// Push the document-start script list — the transport, the shared bridge
    /// sources, then the user scripts in first-injection order — to Kotlin,
    /// which rebuilds its `ScriptHandler` set under the current origin rules.
    /// `rebuildDocumentStartScripts` on this side of JNI.
    fn push_document_start_scripts(&self) {
        let sources = {
            let scripts = self.shared.scripts.borrow();
            let mut sources = Vec::with_capacity(scripts.len() + 2);
            sources.push(TRANSPORT_SCRIPT.to_string());
            sources.push(DOCUMENT_START_SCRIPT.to_string());
            sources.extend(scripts.iter().map(|(_key, source, time)| {
                if *time == ScriptInjectionTime::DocumentEnd {
                    compose_document_end(source)
                } else {
                    source.clone()
                }
            }));
            sources
        };
        self.call_str_array(
            "setDocumentStartScripts",
            "([Ljava/lang/String;)V",
            &sources,
        );
    }

    /// Register `pending` under a fresh call id and issue `call` against it.
    fn start_call<T>(
        &self,
        make_pending: impl FnOnce(oneshot::Sender<T>) -> PendingCall,
        call: impl FnOnce(u64),
    ) -> oneshot::Receiver<T> {
        let (sender, receiver) = oneshot::channel();
        let call_id = self.alloc_call();
        self.shared
            .pending
            .borrow_mut()
            .insert(call_id, make_pending(sender));
        call(call_id);
        receiver
    }
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        // `release()` zeroes Kotlin's `nativeHandle` copy and unregisters the
        // instance, so no native call can arrive afterwards; the Weak box can
        // then be freed here.
        let mut env = self.env();
        let _ = env.call_method(self.wrapper.as_obj(), "release", "()V", &[]);
        // SAFETY: `native_handle` was `Box::into_raw`'d in `open` and
        // survives exactly one drop: this one, after `release()` has zeroed
        // Kotlin's copy so no native call can dereference it again.
        unsafe { drop(Box::from_raw(self.native_handle as *mut Weak<SharedState>)) };
    }
}

/// The `WebViewHandle` for the Android system `WebView` — every method is a
/// JNI call on the `HydrolysisWebView` wrapper.
pub struct AndroidSystemWebViewHandle {
    inner: Rc<HandleInner>,
}

impl Clone for AndroidSystemWebViewHandle {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl core::fmt::Debug for AndroidSystemWebViewHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AndroidSystemWebViewHandle")
            .field("instance", &self.inner.instance)
            .finish_non_exhaustive()
    }
}

impl AndroidSystemWebViewHandle {
    /// The placement-instance id the leaf embeds by.
    pub(crate) fn instance(&self) -> u64 {
        self.inner.instance
    }
}

impl WebViewHandle for AndroidSystemWebViewHandle {
    fn go_back(&self) {
        self.inner.call("goBack", "()V", &[]);
    }

    fn go_forward(&self) {
        self.inner.call("goForward", "()V", &[]);
    }

    fn go_to(&self, url: &Url) {
        self.inner
            .call_str("loadUrl", "(Ljava/lang/String;)V", url.as_str(), &[]);
    }

    fn stop(&self) {
        self.inner.call("stopLoading", "()V", &[]);
    }

    fn refresh(&self) {
        self.inner.call("reload", "()V", &[]);
    }

    fn set_user_agent(&self, user_agent: &str) {
        self.inner
            .call_str("setUserAgent", "(Ljava/lang/String;)V", user_agent, &[]);
    }

    fn can_go_back(&self) -> bool {
        self.inner.call_bool("canGoBack", "()Z", &[])
    }

    fn can_go_forward(&self) -> bool {
        self.inner.call_bool("canGoForward", "()Z", &[])
    }

    fn inject_script(&self, key: &str, script: &str, time: ScriptInjectionTime) {
        {
            let mut scripts = self.inner.shared.scripts.borrow_mut();
            scripts.retain(|held| held.0 != key);
            scripts.push((key.to_string(), script.to_string(), time));
        }
        self.inner.push_document_start_scripts();
    }

    fn set_bridge_origins(&self, policy: OriginPolicy) {
        let rules = androidx_origin_rules(&policy);
        self.inner
            .call_str_array("setBridgeOrigins", "([Ljava/lang/String;)V", &rules);
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        let enabled = Computed::new(enabled);
        let weak = Rc::downgrade(&self.inner);
        let guard = enabled.watch(move |ctx| {
            if let Some(inner) = weak.upgrade() {
                inner.call(
                    "setRedirectsEnabled",
                    "(Z)V",
                    &[JValue::Bool(jboolean::from(ctx.into_value()))],
                );
            }
        });
        *self.inner.redirects_guard.borrow_mut() = Some(guard);
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let url = {
            let current = self.inner.shared.current_url.borrow();
            cookie_url(&cookie, current.as_deref())
        };
        let Some(url) = url else {
            // The Kotlin runtime's `cookieUrl` nullability: a host-only cookie
            // before the first navigation has nowhere to be stored.
            tracing::warn!(
                "android webview: dropping a host-only cookie before the first navigation"
            );
            return;
        };
        let env = self.inner.env();
        let url = env.new_string(url).expect("Java-safe UTF-8");
        let header = env.new_string(cookie.to_string()).expect("Java-safe UTF-8");
        self.inner.call(
            "setCookie",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::Object(url.as_ref()),
                JValue::Object(header.as_ref()),
            ],
        );
    }

    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
        let receiver = self.inner.start_call(PendingCall::Cookies, |call_id| {
            self.inner.call(
                "getCookies",
                "(J)V",
                &[JValue::Long(
                    jlong::try_from(call_id).expect("a call id fits a jlong"),
                )],
            );
        });
        async move { receiver.await.expect("get_cookies calls settle") }
    }

    #[expect(
        clippy::future_not_send,
        reason = "the system WebView is confined to the UI thread"
    )]
    fn run_javascript(&self, script: &str) -> impl Future<Output = Result<Str, Str>> {
        let receiver = self.inner.start_call(PendingCall::JavaScript, |call_id| {
            self.inner.call_str(
                "evaluate",
                "(Ljava/lang/String;J)V",
                script,
                &[JValue::Long(
                    jlong::try_from(call_id).expect("a call id fits a jlong"),
                )],
            );
        });
        async move { receiver.await.expect("run_javascript calls settle") }
    }

    #[expect(
        clippy::future_not_send,
        reason = "the system WebView is confined to the UI thread"
    )]
    fn call_async_javascript(&self, script: &str) -> impl Future<Output = Result<Str, Str>> {
        let receiver = self.inner.start_call(PendingCall::Async, |call_id| {
            let body = compose_async_call(call_id, script);
            self.inner.call_str(
                "evaluate",
                "(Ljava/lang/String;J)V",
                &body,
                &[JValue::Long(
                    jlong::try_from(call_id).expect("a call id fits a jlong"),
                )],
            );
        });
        async move { receiver.await.expect("call_async_javascript calls settle") }
    }

    fn add_handler(&self, name: &str, handler: Box<ScriptMessageHandler>) {
        self.inner
            .shared
            .handlers
            .borrow_mut()
            .insert(name.to_string(), Rc::from(handler));
    }

    fn remove_handler(&self, name: &str) {
        self.inner.shared.handlers.borrow_mut().remove(name);
    }

    fn asset_origin(&self) -> Option<Url> {
        self.inner.shared.asset_origin.clone()
    }

    fn watch(&self, watcher: impl Fn(BackendEvent) + 'static) -> WatcherGuard {
        self.inner.shared.watchers.insert(watcher)
    }
}

/// Upgrade the `Box<Weak<SharedState>>` Kotlin carries. `None` — including a
/// zero pointer — means `release()` already ran and the callback drops.
fn shared_from_handle(handle: jlong) -> Option<Rc<SharedState>> {
    if handle == 0 {
        return None;
    }
    // SAFETY: nonzero handles were `Box::into_raw`'d `Weak` pointers from
    // `open`; they stay live until `release`, which zeroes Kotlin's copy, so
    // a native call can never carry a dangling one.
    unsafe { &*(handle as *const Weak<SharedState>) }.upgrade()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeWillNavigate(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    url: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        let url = get_string(env, &url)?;
        let parsed: Url = url.parse().map_err(|_| {
            JniError(format!(
                "android webview: a navigation to {url:?} is not a URL"
            ))
        })?;
        *shared.current_url.borrow_mut() = Some(url);
        shared.emit_webview(WebViewEvent::WillNavigate { url: parsed });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeLoading(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    progress: jfloat,
) {
    guard(&mut env, |_env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        shared.emit_webview(WebViewEvent::Loading { progress });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeLoaded(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    guard(&mut env, |_env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        shared.emit_webview(WebViewEvent::Loaded);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeRedirect(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    from: JString,
    to: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        let from = get_string(env, &from)?;
        let to = get_string(env, &to)?;
        let parsed_from: Url = from.parse().map_err(|_| {
            JniError(format!(
                "android webview: a redirect from {from:?} is not a URL"
            ))
        })?;
        let parsed_to: Url = to.parse().map_err(|_| {
            JniError(format!(
                "android webview: a redirect to {to:?} is not a URL"
            ))
        })?;
        *shared.current_url.borrow_mut() = Some(to);
        shared.emit_webview(WebViewEvent::Redirect {
            from: parsed_from,
            to: parsed_to,
        });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeError(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    message: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        let message = get_string(env, &message)?;
        shared.emit_webview(WebViewEvent::Error(WebViewError::Network(Str::from(
            message,
        ))));
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeSslError(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    url: JString,
    message: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        let url = get_string(env, &url)?;
        let message = get_string(env, &message)?;
        let url: Url = url.parse().map_err(|_| {
            JniError(format!(
                "android webview: an SSL error on {url:?} is not a URL"
            ))
        })?;
        shared.emit_webview(WebViewEvent::Error(WebViewError::Ssl {
            url,
            message: Str::from(message),
        }));
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeNavigationState(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    can_go_back: jboolean,
    can_go_forward: jboolean,
) {
    guard(&mut env, |_env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        shared.emit(&BackendEvent::NavigationState {
            can_go_back: can_go_back != 0,
            can_go_forward: can_go_forward != 0,
        });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeOnBridgeMessage(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    envelope: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: a bridge message on a released view dropped");
            return Ok(());
        };
        let envelope = get_string(env, &envelope)?;
        shared.on_bridge_message(&envelope);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeAsyncResult(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    payload: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an async result on a released view dropped");
            return Ok(());
        };
        let payload = get_string(env, &payload)?;
        shared.settle_async(&payload);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeJsResult(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    call_id: jlong,
    ok: jboolean,
    value: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: a call result on a released view dropped");
            return Ok(());
        };
        let value = get_string(env, &value)?;
        shared.settle_js(u64::try_from(call_id).unwrap_or_default(), ok != 0, value);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeCookies(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    call_id: jlong,
    cookies: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: a cookie reply on a released view dropped");
            return Ok(());
        };
        let cookies = get_string(env, &cookies)?;
        shared.settle_js(u64::try_from(call_id).unwrap_or_default(), true, cookies);
        Ok(())
    });
}

/// One intercepted asset-origin request, answered synchronously on the
/// `WebView`'s worker thread. `asset_server` is the leaked `Box<AssetServer>`
/// `create` passed Kotlin; a zero pointer is a contract break, not a 404.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeAssetRespond(
    mut env: JNIEnv,
    _class: JClass,
    asset_server: jlong,
    method: JString,
    path: JString,
    query: JString,
) -> jobject {
    guard_val(&mut env, std::ptr::null_mut(), |env| {
        if asset_server == 0 {
            return Err(JniError(
                "android webview: an asset request reached a null asset server".into(),
            ));
        }
        // SAFETY: Kotlin hands back the `Box::into_raw` pointer `create`
        // received; `nativeFreeAssetServer` runs only after `destroy()`, so
        // no request can outlive it.
        let server = unsafe { &*(asset_server as *const AssetServer) };
        let method = get_string(env, &method)?;
        let path = get_string(env, &path)?;
        let query = if query.as_raw().is_null() {
            None
        } else {
            Some(get_string(env, &query)?)
        };
        let response = assets::dispatch(server, &method, &path, query.as_deref());

        // The reply crosses as one `AssetResponse` object: status, the
        // newline-joined `Name: value` header block Kotlin splits again on
        // the other side, and the body bytes.
        let headers = response
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let status = jint::from(response.status);
        let headers = env.new_string(headers)?;
        let body = env.byte_array_from_slice(&response.body)?;
        let response = env.new_object(
            ASSET_RESPONSE_CLASS,
            ASSET_RESPONSE_SIG,
            &[
                JValue::Int(status),
                JValue::Object(headers.as_ref()),
                JValue::Object(body.as_ref()),
            ],
        )?;
        Ok(response.into_raw())
    })
}

/// Drop the `Box<AssetServer>` `create` leaked — called by `release()` after
/// `destroy()`, so no request can still be inside `nativeAssetRespond`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeFreeAssetServer(
    mut env: JNIEnv,
    _class: JClass,
    asset_server: jlong,
) {
    guard(&mut env, |_env| {
        if asset_server == 0 {
            return Ok(());
        }
        // SAFETY: `create` leaked exactly one `Box<AssetServer>` for this
        // pointer; `release()` calls this after `destroy()`, past the last
        // request the view could make.
        unsafe { drop(Box::from_raw(asset_server as *mut AssetServer)) };
        Ok(())
    });
}
