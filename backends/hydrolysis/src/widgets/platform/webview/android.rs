//! The Android `WebView` bridge: `AndroidSystemWebViewController` mounts the
//! platform's `android.webkit.WebView` as a platform-view *instance* —
//! `WebView` in `dev.waterui.hydrolysis.webview.HydrolysisWebView` — and the
//! handle drives it over JNI.
//!
//! The Kotlin side owns the `WebView`'s callbacks; everything Rust cares about
//! arrives through the `native…` functions at the bottom of this file, the
//! same division `ffi`'s webview bridge draws against the Kotlin runtime. The
//! parts that need no JNI — origin-rule conversion, the cookie URL, script
//! composition, async-result parsing, the pending-call registry, the
//! navigation event state machine and the method table — live in
//! [`super::android_protocol`] so the host test suite exercises them. Kotlin
//! forwards the raw navigation callbacks; [`NavigationTracker`] alone decides
//! which `WebViewEvent`s they report.

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::format;
use std::rc::{Rc, Weak};
use std::string::{String, ToString};
use std::vec::Vec;

use executor_core::spawn_local;
use futures::channel::oneshot;
use jni::objects::{GlobalRef, JClass, JMethodID, JObject, JStaticMethodID, JString, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::sys::{jboolean, jint, jlong, jobject, jvalue};
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
    ASYNC_RESULT_OBJECT, BRIDGE_OBJECT, NavigationTracker, PendingCall, PendingCalls,
    TRANSPORT_SCRIPT, WEBVIEW_ASSET_RESPONSE_INIT, WEBVIEW_CREATE, WEBVIEW_METHODS,
    WebViewMethodId, androidx_origin_rules, compose_async_call, compose_document_end, cookie_url,
    fresh_async_token, origin_may_use_bridge, watch_bool_with_initial,
};

/// The Kotlin wrapper class, resolved at session-create time: a thread that
/// did not enter from Java cannot resolve app classes later.
const WRAPPER_CLASS: &str = "dev/waterui/hydrolysis/webview/HydrolysisWebView";

/// The class `nativeAssetRespond` constructs by name — its constructor is
/// part of the JNI contract the `@CalledFromNative` keep rule preserves.
const ASSET_RESPONSE_CLASS: &str = "dev/waterui/hydrolysis/webview/AssetResponse";

/// The JNI method ids resolved once at session create, in
/// [`WebViewMethodId`] order. A method R8 stripped or renamed fails the
/// session's create, not the first call that reaches for it.
#[derive(Clone)]
struct WebViewMethods {
    methods: Rc<Vec<JMethodID>>,
    create: JStaticMethodID,
}

impl WebViewMethods {
    fn resolve(env: &mut JNIEnv<'_>, class: &JClass<'_>) -> Result<Self, JniError> {
        let mut methods = Vec::with_capacity(WEBVIEW_METHODS.len());
        for method in WEBVIEW_METHODS {
            let id = env.get_method_id(class, method.name, method.signature)?;
            methods.push(id);
        }
        let create =
            env.get_static_method_id(class, WEBVIEW_CREATE.name, WEBVIEW_CREATE.signature)?;
        Ok(Self {
            methods: Rc::new(methods),
            create,
        })
    }

    fn id(&self, method: WebViewMethodId) -> JMethodID {
        self.methods[method as usize]
    }
}

/// `panic!` carrying the pending Java exception's own `toString` when there
/// is one — the named Kotlin errors ("lacks `WEB_MESSAGE_LISTENER`", an
/// unbound session, a non-Activity context) must reach the panic text. The
/// exception is cleared so the guard's `IllegalStateException` stays the
/// pending report.
fn panic_with_java_detail(env: &mut JNIEnv<'_>, context: &str, error: &jni::errors::Error) -> ! {
    let detail = env
        .exception_occurred()
        .ok()
        .and_then(|throwable| {
            let _ = env.exception_clear();
            env.call_method(&throwable, "toString", "()Ljava/lang/String;", &[])
                .ok()
                .and_then(|value| value.l().ok())
                .and_then(|object| {
                    env.get_string(&JString::from(object))
                        .ok()
                        .map(|s| s.to_string_lossy().into_owned())
                })
        })
        .unwrap_or_else(|| error.to_string());
    panic!("android webview: {context} failed: {detail}");
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
    /// The cached wrapper-method table the reply path calls through.
    methods: WebViewMethods,
    /// Backend-event watchers — `watch` on the handle.
    watchers: WatcherSet<BackendEvent>,
    /// Named script-message handlers; `nativeOnBridgeMessage` dispatches.
    handlers: RefCell<HashMap<String, Rc<ScriptMessageHandler>>>,
    /// User-injected scripts in first-injection order — the push list is
    /// rebuilt on every `inject_script`, as `rebuildDocumentStartScripts`
    /// does in the Kotlin runtime.
    scripts: RefCell<Vec<(String, String, ScriptInjectionTime)>>,
    /// In-flight calls indexed by call id — the one place every settlement
    /// path consults, so navigation and teardown can settle all of them at
    /// once.
    pending: RefCell<PendingCalls>,
    /// The bridge origin policy as the androidx rule strings — the bridge
    /// listener admits a message by asking `nativeOriginMayUseBridge` here.
    origin_rules: RefCell<Vec<String>>,
    /// The navigation event state machine every navigation callback feeds;
    /// it also knows the main-frame URL the cookie path stores against.
    navigation: RefCell<NavigationTracker>,
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
        tracing::debug!(?event, "android webview: event");
        self.emit(&BackendEvent::Event(event));
    }

    /// Feed one navigation input to the tracker and emit what it reports.
    /// The emission runs after the tracker's borrow ends: an event handler
    /// that navigates again re-enters the tracker through Kotlin.
    fn navigate<T>(
        &self,
        input: impl FnOnce(&mut NavigationTracker) -> (Vec<WebViewEvent>, T),
    ) -> T {
        let (events, answer) = input(&mut self.navigation.borrow_mut());
        for event in events {
            self.emit_webview(event);
        }
        answer
    }

    /// `evaluateBridgeScript` — the fire-and-forget evaluation bridge replies
    /// take. Callable without a handle because the wrapper lives here. Runs
    /// inside a local JNI frame: executor tasks enter JNI from the `ALooper`
    /// fd callback, which has no frame, and locals would otherwise leak.
    fn evaluate_bridge_script(&self, script: String) {
        let Some(wrapper) = self.wrapper.borrow().clone() else {
            tracing::warn!("android webview: a bridge reply arrived before the wrapper was set");
            return;
        };
        let method = self.methods.id(WebViewMethodId::EvaluateBridgeScript);
        let mut env = self
            .vm
            .get_env()
            .expect("bridge replies run on the UI thread, which is attached");
        env.with_local_frame::<_, _, jni::errors::Error>(16, |env| {
            let script = env
                .new_string(script)
                .expect("a reply script is Java-safe UTF-8");
            // SAFETY: `method` was resolved for the wrapper class at create
            // time and the signature is `evaluateBridgeScript`'s `(String)V`.
            unsafe {
                env.call_method_unchecked(
                    wrapper.as_obj(),
                    method,
                    ReturnType::Primitive(Primitive::Void),
                    &[JValue::Object(script.as_ref()).as_jni()],
                )
                .map(|_| ())
            }
        })
        .unwrap_or_else(|error| panic_with_java_detail(&mut env, "evaluateBridgeScript", &error));
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

/// The controller the session's environment carries. `new` resolves the
/// wrapper class and its method table while still on the JNI thread, and
/// `open` builds wrappers against both.
pub struct AndroidSystemWebViewController {
    vm: JavaVM,
    /// The `HydrolysisSession` — the object `HostBridge` calls back into, and
    /// the instance registry `create` registers the wrapper on.
    session: GlobalRef,
    webview_class: GlobalRef,
    methods: WebViewMethods,
    /// Placement instance ids, owned by the controller rather than a static:
    /// a session's registry counts from its own zero.
    next_instance: Cell<u64>,
}

impl AndroidSystemWebViewController {
    /// Resolve the wrapper class and its method table, and capture the
    /// session. `env` must be the JNI environment of `nativeCreateSession` —
    /// the only call that reaches app classes by name on every thread model.
    /// A method R8 stripped or renamed fails here, naming it.
    pub fn new(env: &mut JNIEnv<'_>, session: GlobalRef) -> Result<Self, JniError> {
        let class = env.find_class(WRAPPER_CLASS).map_err(JniError::from)?;
        let methods = WebViewMethods::resolve(env, &class)?;
        let webview_class = env.new_global_ref(&class).map_err(JniError::from)?;
        Ok(Self {
            vm: env.get_java_vm().map_err(JniError::from)?,
            session,
            webview_class,
            methods,
            next_instance: Cell::new(0),
        })
    }
}

impl CustomWebViewController for AndroidSystemWebViewController {
    fn open(&self, config: WebViewConfig) -> impl WebViewHandle {
        let instance = self.next_instance.get() + 1;
        self.next_instance.set(instance);

        // `config.asset_server` crosses as a leaked `Box` Kotlin hands
        // back to `nativeAssetServerAcquire`/`nativeFreeAssetServer`. The
        // `AssetServer` inside is already an `Arc` — the box is a handle,
        // not a second refcount.
        let asset_server = config
            .asset_server
            .map_or(0, |server| Box::into_raw(Box::new(server)) as jlong);

        let mut env = self
            .vm
            .get_env()
            .expect("open must run on the UI thread, which is attached");
        let vm = env
            .get_java_vm()
            .expect("a JNIEnv always yields its JavaVM");
        let shared = Rc::new(SharedState {
            vm,
            wrapper: RefCell::new(None),
            methods: self.methods.clone(),
            watchers: WatcherSet::new(),
            handlers: RefCell::new(HashMap::new()),
            scripts: RefCell::new(Vec::new()),
            pending: RefCell::new(PendingCalls::default()),
            origin_rules: RefCell::new(Vec::new()),
            navigation: RefCell::new(NavigationTracker::default()),
            asset_origin: (asset_server != 0).then(|| {
                ASSET_HTTPS_ORIGIN
                    .parse()
                    .expect("the asset origin is a constant")
            }),
        });
        let native_handle = Box::into_raw(Box::new(Rc::downgrade(&shared))) as jlong;

        let wrapper = env
            .with_local_frame::<_, _, jni::errors::Error>(16, |env| {
                self.create_wrapper(env, instance, native_handle, asset_server)
            })
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
                panic_with_java_detail(&mut env, "HydrolysisWebView.create", &error)
            });
        *shared.wrapper.borrow_mut() = Some(wrapper.clone());

        AndroidSystemWebViewHandle {
            inner: Rc::new(HandleInner {
                wrapper,
                methods: self.methods.clone(),
                shared,
                instance,
                native_handle,
                next_call: Cell::new(1),
                redirects_guard: RefCell::new(None),
            }),
        }
    }
}

/// The Kotlin `HydrolysisWebView.create` call — the companion's static
/// factory, resolved on the class at session create. Returns the wrapper as
/// a `GlobalRef`.
impl AndroidSystemWebViewController {
    fn create_wrapper(
        &self,
        env: &mut JNIEnv<'_>,
        instance: u64,
        native_handle: jlong,
        asset_server: jlong,
    ) -> Result<GlobalRef, jni::errors::Error> {
        let bridge_object = env.new_string(BRIDGE_OBJECT)?;
        let async_result_object = env.new_string(ASYNC_RESULT_OBJECT)?;
        let asset_host = env.new_string(ANDROID_ASSET_HOST)?;
        // SAFETY: `create` was resolved for `webview_class` and the
        // arguments match its `(Session,J,J,J,String,String,String)` signature.
        let wrapper = unsafe {
            env.call_static_method_unchecked(
                &self.webview_class,
                self.methods.create,
                ReturnType::Object,
                &[
                    JValue::Object(self.session.as_ref()).as_jni(),
                    JValue::Long(jlong::try_from(instance).expect("an instance id fits a jlong"))
                        .as_jni(),
                    JValue::Long(native_handle).as_jni(),
                    JValue::Long(asset_server).as_jni(),
                    JValue::Object(bridge_object.as_ref()).as_jni(),
                    JValue::Object(async_result_object.as_ref()).as_jni(),
                    JValue::Object(asset_host.as_ref()).as_jni(),
                ],
            )?
            .l()?
        };
        env.new_global_ref(&wrapper)
    }
}

/// The host name of `ASSET_HTTPS_ORIGIN` — requests to it are intercepted
/// and answered through `nativeAssetRespond`. Passed to Kotlin by `create`,
/// like the injected object names.
const ANDROID_ASSET_HOST: &str = "waterui.localhost";

/// The live wrapper and the bookkeeping around it.
struct HandleInner {
    wrapper: GlobalRef,
    methods: WebViewMethods,
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

/// The local-frame capacity every wrapper call runs under — the fixed
/// arguments plus a margin. The script-array loop is unbounded, so
/// `call_str_array` adds the array length on top of this.
const LOCAL_FRAME_CAPACITY: i32 = 32;

impl HandleInner {
    /// Run `f` with a JNI env inside a fresh local reference frame sized to
    /// `locals`: the calls the handle makes run inside executor tasks on the
    /// `ALooper` fd callback, which supplies no JNI frame, so locals would
    /// otherwise leak.
    ///
    /// Every Rust-called command funnels through here: after the render
    /// process died, `nativeReleased` already marked the registry dead, and
    /// forwarding would dispatch a setter or an evaluation onto a `WebView`
    /// whose `destroy()` ran — Chromium keeps answering
    /// `setDocumentStartScripts`/`loadUrl` on a destroyed view and
    /// `onHostRebound` would re-observe a dead page's lifecycle. A dead view
    /// answers `dead_value` without dispatching; the handle's drop calls
    /// `release` itself, bypassing `call`, so teardown still reaches Kotlin.
    fn jni<T>(
        &self,
        locals: i32,
        dead_value: T,
        f: impl FnOnce(&mut JNIEnv<'_>) -> Result<T, jni::errors::Error>,
        context: &str,
    ) -> T {
        if self.shared.pending.borrow().dead() {
            return dead_value;
        }
        let mut env = self
            .shared
            .vm
            .get_env()
            .expect("webview calls run on the UI thread, which is attached");
        env.with_local_frame(locals, f)
            .unwrap_or_else(|error| panic_with_java_detail(&mut env, context, &error))
    }

    /// `call_method_unchecked` on the wrapper through the cached table.
    fn call(&self, method: WebViewMethodId, args: &[JValue]) {
        let id = self.methods.id(method);
        self.jni(
            LOCAL_FRAME_CAPACITY,
            (),
            |env| unsafe {
                // SAFETY: `id` was resolved for the wrapper class and the
                // table entry's signature declares void.
                env.call_method_unchecked(
                    self.wrapper.as_obj(),
                    id,
                    ReturnType::Primitive(Primitive::Void),
                    &args.iter().map(JValue::as_jni).collect::<Vec<jvalue>>(),
                )
                .map(|_| ())
            },
            WEBVIEW_METHODS[method as usize].name,
        );
    }

    fn call_bool(&self, method: WebViewMethodId, args: &[JValue]) -> bool {
        let id = self.methods.id(method);
        // A dead view answers `false` itself, ahead of `jni`'s `dead_value`
        // — the caller cannot tell a dead-view `false` from the call's own.
        if self.shared.pending.borrow().dead() {
            return false;
        }
        self.jni(
            LOCAL_FRAME_CAPACITY,
            false,
            |env| unsafe {
                // SAFETY: `id` was resolved for the wrapper class and the
                // table entry's signature declares a boolean return.
                env.call_method_unchecked(
                    self.wrapper.as_obj(),
                    id,
                    ReturnType::Primitive(Primitive::Boolean),
                    &args.iter().map(JValue::as_jni).collect::<Vec<jvalue>>(),
                )
                .and_then(jni::objects::JValueGen::z)
            },
            WEBVIEW_METHODS[method as usize].name,
        )
    }

    /// One string argument — `loadUrl`, `setUserAgent`, `evaluate`.
    fn call_str(&self, method: WebViewMethodId, value: &str, tail: &[JValue]) {
        let id = self.methods.id(method);
        self.jni(
            LOCAL_FRAME_CAPACITY,
            (),
            |env| {
                let value = env
                    .new_string(value)
                    .expect("the argument is Java-safe UTF-8");
                let mut args: Vec<jvalue> = Vec::with_capacity(tail.len() + 1);
                args.push(JValue::Object(value.as_ref()).as_jni());
                args.extend(tail.iter().map(JValue::as_jni));
                // SAFETY: `id` was resolved for the wrapper class and the
                // table entry's signature declares `(String, ...)void`,
                // matching the arguments pushed above.
                unsafe {
                    env.call_method_unchecked(
                        self.wrapper.as_obj(),
                        id,
                        ReturnType::Primitive(Primitive::Void),
                        &args,
                    )
                }
                .map(|_| ())
            },
            WEBVIEW_METHODS[method as usize].name,
        );
    }

    /// Push a `String[]` argument to a one-argument setter. The frame
    /// covers the array itself plus one local per element — the list is
    /// unbounded, so a fixed capacity would overflow on enough scripts.
    fn call_str_array(&self, method: WebViewMethodId, values: &[String]) {
        let id = self.methods.id(method);
        let locals = LOCAL_FRAME_CAPACITY
            .saturating_add(i32::try_from(values.len()).expect("a script list fits a jint"));
        self.jni(
            locals,
            (),
            |env| {
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
                // SAFETY: `id` was resolved for the wrapper class and the
                // table entry's signature declares `([Ljava/lang/String;)V`.
                unsafe {
                    env.call_method_unchecked(
                        self.wrapper.as_obj(),
                        id,
                        ReturnType::Primitive(Primitive::Void),
                        &[JValue::Object(&array).as_jni()],
                    )
                }
                .map(|_| ())
            },
            WEBVIEW_METHODS[method as usize].name,
        );
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
        self.call_str_array(WebViewMethodId::SetDocumentStartScripts, &sources);
    }

    /// Register `pending` under a fresh call id and issue `call` against
    /// it. `token` is the per-call secret `PendingCall::Async` stores and
    /// `compose_async_call` stamps — the other call kinds ignore it. A
    /// released view never issues the Kotlin call: `begin` settles the
    /// entry at once, and `call` would hit a zeroed `nativeHandle`.
    /// If the call itself panics the pending entry is still in the
    /// registry — the panic path is a teardown anyway.
    fn start_call<T>(
        &self,
        make_pending: impl FnOnce(oneshot::Sender<T>, String) -> PendingCall,
        call: impl FnOnce(u64, &str),
    ) -> oneshot::Receiver<T> {
        let (sender, receiver) = oneshot::channel();
        let call_id = self.next_call.get();
        self.next_call.set(call_id + 1);
        let token = fresh_async_token();
        if self
            .shared
            .pending
            .borrow_mut()
            .begin(call_id, make_pending(sender, token.clone()))
        {
            call(call_id, &token);
        }
        receiver
    }
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        // This drop runs wherever the view tree lets go of the handle: in a
        // frame that removed the leaf, or inside `nativeDestroySession`, with
        // the session's `drop_in_place` half done on the stack. `release()`
        // therefore makes no native call — it zeroes Kotlin's `nativeHandle`
        // copy, destroys the view, unregisters the instance and hands back
        // the asset server — and the rest of the teardown happens here, in
        // Rust, with nothing re-entering it.
        let mut env = self
            .shared
            .vm
            .get_env()
            .expect("webview calls run on the UI thread, which is attached");
        let release = self.methods.id(WebViewMethodId::Release);
        // SAFETY: `release` was resolved for the wrapper class at create
        // and its table signature is `()J`.
        let asset_server = env
            .with_local_frame::<_, _, jni::errors::Error>(4, |env| unsafe {
                env.call_method_unchecked(
                    self.wrapper.as_obj(),
                    release,
                    ReturnType::Primitive(Primitive::Long),
                    &[],
                )?
                .j()
            })
            .unwrap_or_else(|error| panic_with_java_detail(&mut env, "release", &error));
        // Marking the registry dead settles every call still in flight —
        // the async ids Kotlin forgot after the started sentinel included —
        // and a call begun after this settles at once rather than waiting on
        // a reply that cannot come. Already dead after a render-process-gone
        // teardown, which drained it through `nativeReleased`.
        let calls = self.shared.pending.borrow_mut().release();
        PendingCalls::settle_many(calls, "the web view was closed");
        if asset_server != 0 {
            // SAFETY: `asset_server` was `Box::into_raw`'d in `open`, and
            // `release()` took it under the write lock every interception's
            // acquire holds the read half of: no acquire can reach it any
            // more, and an interception still dispatching owns its own `Arc`
            // clone. Kotlin zeroed its copy, so this is the one free.
            unsafe { drop(Box::from_raw(asset_server as *mut AssetServer)) };
        }
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
    // The four loads the application starts go through Kotlin's
    // `navigate…` methods: Android reports none of them to
    // `shouldOverrideUrlLoading`, so the wrapper reports each target to
    // `nativeOpenNavigation` itself, in the engine's spelling — the one
    // every later callback carries, not the string handed to `go_to`.
    fn go_back(&self) {
        self.inner.call(WebViewMethodId::NavigateBack, &[]);
    }

    fn go_forward(&self) {
        self.inner.call(WebViewMethodId::NavigateForward, &[]);
    }

    fn go_to(&self, url: &Url) {
        self.inner
            .call_str(WebViewMethodId::NavigateTo, url.as_str(), &[]);
    }

    fn stop(&self) {
        self.inner.shared.navigation.borrow_mut().stopped();
        self.inner.call(WebViewMethodId::StopLoading, &[]);
    }

    fn refresh(&self) {
        self.inner.call(WebViewMethodId::NavigateReload, &[]);
    }

    fn set_user_agent(&self, user_agent: &str) {
        self.inner
            .call_str(WebViewMethodId::SetUserAgent, user_agent, &[]);
    }

    fn can_go_back(&self) -> bool {
        self.inner.call_bool(WebViewMethodId::CanGoBack, &[])
    }

    fn can_go_forward(&self) -> bool {
        self.inner.call_bool(WebViewMethodId::CanGoForward, &[])
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
            .shared
            .origin_rules
            .borrow_mut()
            .clone_from(&rules);
        self.inner
            .call_str_array(WebViewMethodId::SetBridgeOrigins, &rules);
    }

    fn set_redirects_enabled(&self, enabled: impl Signal<Output = bool>) {
        let enabled = Computed::new(enabled);
        let weak = Rc::downgrade(&self.inner.shared);
        // `watch` alone fires only on change — and a constant's never — so
        // the value the signal already holds is applied first, or the
        // tracker would keep its default (`true`) for a constant `false`.
        let guard = watch_bool_with_initial(&enabled, move |enabled| {
            if let Some(shared) = weak.upgrade() {
                shared
                    .navigation
                    .borrow_mut()
                    .set_redirects_enabled(enabled);
            }
        });
        *self.inner.redirects_guard.borrow_mut() = Some(guard);
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        let url = {
            let navigation = self.inner.shared.navigation.borrow();
            cookie_url(&cookie, navigation.current_url().map(Url::as_str))
        };
        let Some(url) = url else {
            // The Kotlin runtime's `cookieUrl` nullability: a host-only cookie
            // before the first navigation has nowhere to be stored.
            tracing::warn!(
                "android webview: dropping a host-only cookie before the first navigation"
            );
            return;
        };
        let id = self.inner.methods.id(WebViewMethodId::SetCookie);
        self.inner.jni(
            LOCAL_FRAME_CAPACITY,
            (),
            |env| {
                let url = env.new_string(url).expect("Java-safe UTF-8");
                let header = env.new_string(cookie.to_string()).expect("Java-safe UTF-8");
                // SAFETY: `id` was resolved for the wrapper class and the
                // table entry's signature declares `(String,String)V`.
                unsafe {
                    env.call_method_unchecked(
                        self.inner.wrapper.as_obj(),
                        id,
                        ReturnType::Primitive(Primitive::Void),
                        &[
                            JValue::Object(url.as_ref()).as_jni(),
                            JValue::Object(header.as_ref()).as_jni(),
                        ],
                    )
                }
                .map(|_| ())
            },
            "setCookie",
        );
    }

    fn get_cookies(&self) -> impl Future<Output = Vec<Cookie<'static>>> {
        let receiver = self.inner.start_call(
            |sender, _token| {
                PendingCall::Cookies(Box::new(move |cookies| {
                    let _ = sender.send(cookies);
                }))
            },
            |call_id, _token| {
                self.inner.call(
                    WebViewMethodId::GetCookies,
                    &[JValue::Long(
                        jlong::try_from(call_id).expect("a call id fits a jlong"),
                    )],
                );
            },
        );
        async move { receiver.await.expect("get_cookies calls settle") }
    }

    #[expect(
        clippy::future_not_send,
        reason = "the system WebView is confined to the UI thread"
    )]
    fn run_javascript(&self, script: &str) -> impl Future<Output = Result<Str, Str>> {
        let receiver = self.inner.start_call(
            |sender, _token| {
                PendingCall::JavaScript(Box::new(move |result| {
                    let _ = sender.send(result);
                }))
            },
            |call_id, _token| {
                self.inner.call_str(
                    WebViewMethodId::Evaluate,
                    script,
                    &[JValue::Long(
                        jlong::try_from(call_id).expect("a call id fits a jlong"),
                    )],
                );
            },
        );
        async move { receiver.await.expect("run_javascript calls settle") }
    }

    #[expect(
        clippy::future_not_send,
        reason = "the system WebView is confined to the UI thread"
    )]
    fn call_async_javascript(&self, script: &str) -> impl Future<Output = Result<Str, Str>> {
        let receiver = self.inner.start_call(
            |sender, token| PendingCall::Async {
                settle: Box::new(move |result| {
                    let _ = sender.send(result);
                }),
                token,
            },
            |call_id, token| {
                let generation = self.inner.shared.pending.borrow().generation();
                let body = compose_async_call(call_id, generation, token, script);
                self.inner.call_str(
                    WebViewMethodId::Evaluate,
                    &body,
                    &[JValue::Long(
                        jlong::try_from(call_id).expect("a call id fits a jlong"),
                    )],
                );
            },
        );
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

/// A URL a navigation callback carried; `what` names it in the error.
fn parse_url(url: &str, what: &str) -> Result<Url, JniError> {
    url.parse()
        .map_err(|_| JniError(format!("android webview: {what} {url:?} is not a URL")))
}

/// `onProgressChanged`'s percentage, which the platform keeps in `0..=100`.
fn percent(progress: jint) -> Result<u8, JniError> {
    u8::try_from(progress)
        .ok()
        .filter(|percent| *percent <= 100)
        .ok_or_else(|| {
            JniError(format!(
                "android webview: a progress report of {progress} is outside 0..=100"
            ))
        })
}

/// An application-started load — `navigateTo`, `navigateBack`,
/// `navigateForward` or `navigateReload` — reporting its target as the
/// engine spells it. Android routes none of them through
/// `shouldOverrideUrlLoading`, so this is where they open.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeOpenNavigation(
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
        let url = parse_url(&get_string(env, &url)?, "a navigation to")?;
        shared.navigate(|tracker| (tracker.open(url), ()));
        Ok(())
    });
}

/// `shouldOverrideUrlLoading`, answered by the tracker: `true` cancels the
/// request — a server redirect while redirects are disabled. A subframe
/// request is the page's own and passes before its URL is read.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeShouldOverrideUrlLoading(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    url: JString,
    is_main_frame: jboolean,
    is_redirect: jboolean,
) -> jboolean {
    guard_val(&mut env, jboolean::from(false), |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: a request on a released view passed");
            return Ok(jboolean::from(false));
        };
        if is_main_frame == 0 {
            return Ok(jboolean::from(false));
        }
        let url = parse_url(&get_string(env, &url)?, "a request for")?;
        let block = shared.navigate(|tracker| {
            let decision = tracker.request(url, is_redirect != 0);
            (decision.events, decision.block)
        });
        Ok(jboolean::from(block))
    })
}

/// `onPageStarted` — the one client callback that fires exactly on a
/// main-frame cross-document commit. The calls the old document owed
/// settle here, and the generation bump retires its in-flight async
/// results before the new document's calls begin; then the tracker hears
/// of the commit.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativePageStarted(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    url: JString,
    progress: jint,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: a document commit on a released view dropped");
            return Ok(());
        };
        let url = parse_url(&get_string(env, &url)?, "a commit of")?;
        let progress = percent(progress)?;
        let calls = shared.pending.borrow_mut().document_replaced();
        PendingCalls::settle_many(calls, "the document was replaced before the script ran");
        shared.navigate(|tracker| (tracker.page_started(url, progress), ()));
        Ok(())
    });
}

/// `doUpdateVisitedHistory`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeHistoryUpdated(
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
        let url = parse_url(&get_string(env, &url)?, "a history entry for")?;
        shared.navigation.borrow_mut().history_updated(url);
        Ok(())
    });
}

/// `onProgressChanged` — the navigation's progress, and its finish.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeProgressChanged(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    progress: jint,
) {
    guard(&mut env, |_env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        let progress = percent(progress)?;
        shared.navigate(|tracker| (tracker.progress(progress), ()));
        Ok(())
    });
}

/// `onReceivedError`, for any frame. A subframe's — a failed image, a
/// blocked iframe — is no navigation's and drops before its URL is read.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeReceivedError(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    url: JString,
    is_main_frame: jboolean,
    message: JString,
) {
    guard(&mut env, |env| {
        let Some(shared) = shared_from_handle(handle) else {
            tracing::debug!("android webview: an event on a released view dropped");
            return Ok(());
        };
        if is_main_frame == 0 {
            return Ok(());
        }
        let url = parse_url(&get_string(env, &url)?, "a failed request for")?;
        let message = get_string(env, &message)?;
        let error = WebViewError::Network(Str::from(message));
        shared.navigate(|tracker| (tracker.received_error(url, error), ()));
        Ok(())
    });
}

/// The render process is gone and the view is being torn down — not a
/// navigation's end but the view's, reported unconditionally.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeRenderProcessGone(
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
        let message = Str::from(get_string(env, &message)?);
        // `SslError.url` can be absent — the error still reports, as a
        // network-level failure carrying no URL.
        let url = if url.is_null() {
            None
        } else {
            Some(parse_url(&get_string(env, &url)?, "an SSL error on")?)
        };
        shared.navigate(|tracker| (tracker.ssl_error(url, message), ()));
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

/// Whether a bridge message's source origin is admitted — the single
/// implementation of the runtime's `originMayUseBridge`, called by the
/// Kotlin listener per message.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeOriginMayUseBridge(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    origin: JString,
) -> jboolean {
    guard_val(&mut env, jboolean::from(false), |env| {
        let Some(shared) = shared_from_handle(handle) else {
            return Ok(jboolean::from(false));
        };
        let origin = get_string(env, &origin)?;
        Ok(jboolean::from(origin_may_use_bridge(
            &shared.origin_rules.borrow(),
            &origin,
        )))
    })
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
        shared.pending.borrow_mut().settle_async(&payload);
        Ok(())
    });
}

/// The render process died and Kotlin tore the view down: every call still
/// in flight — an async call past the started sentinel included — settles
/// here, so a receiver is never left unsettled, and the registry is dead
/// until the handle drops. The handle's own drop never comes through here:
/// it runs inside the session's teardown and drains the registry itself.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeReleased(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    guard(&mut env, |_env| {
        let Some(shared) = shared_from_handle(handle) else {
            return Ok(());
        };
        // `release` also marks the registry dead: a call begun after this
        // settles at once rather than waiting on a reply that cannot come.
        let calls = shared.pending.borrow_mut().release();
        PendingCalls::settle_many(calls, "the web view was closed");
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
        shared.pending.borrow_mut().settle(
            u64::try_from(call_id).expect("call ids are allocated non-negative"),
            ok != 0,
            &value,
        );
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
        shared.pending.borrow_mut().settle(
            u64::try_from(call_id).expect("call ids are allocated non-negative"),
            true,
            &cookies,
        );
        Ok(())
    });
}

/// Acquire a reference on the leaked `Box<AssetServer>` `create` passed
/// Kotlin. `shouldInterceptRequest` takes the asset lock's read half only
/// around this call, then dispatches `nativeAssetRespond` outside the lock
/// and hands the clone back through `nativeAssetServerRelease` — a slow
/// request can never stall the teardown taking the pointer. The clone is the
/// `AssetServer`'s own `Arc`, boxed into a fresh handle: the share lives on
/// the server itself, not on the handle.
///
/// SAFETY: `assetServerPtr` is a live `Box::into_raw` pointer while the
/// read lock is held: the teardown takes it under the write half and only
/// then frees it, so the clone here can never race the drop.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeAssetServerAcquire(
    mut env: JNIEnv,
    _class: JClass,
    asset_server: jlong,
) -> jlong {
    guard_val(&mut env, 0, |_env| {
        if asset_server == 0 {
            return Err(JniError(
                "android webview: an asset request reached a null asset server".into(),
            ));
        }
        // SAFETY: the read lock guarantees the `Box` this pointer names is
        // still alive — the teardown takes the pointer under the write lock
        // before anything frees it — so the `AssetServer` clone inside
        // publishes a new owner the caller releases through
        // `nativeAssetServerRelease`.
        let clone = unsafe { &*(asset_server as *const AssetServer) }.clone();
        Ok(Box::into_raw(Box::new(clone)) as jlong)
    })
}

/// Drop the `AssetServer` clone `nativeAssetServerAcquire` handed out —
/// called from `shouldInterceptRequest`'s `finally`, outside the asset lock.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_webview_HydrolysisWebView_nativeAssetServerRelease(
    mut env: JNIEnv,
    _class: JClass,
    asset_server: jlong,
) {
    guard(&mut env, |_env| {
        if asset_server != 0 {
            // SAFETY: each pointer here is a boxed `AssetServer` clone from
            // `nativeAssetServerAcquire`, released exactly once — here.
            unsafe { drop(Box::from_raw(asset_server as *mut AssetServer)) };
        }
        Ok(())
    });
}

/// One intercepted asset-origin request, answered synchronously on the
/// `WebView`'s worker thread. `asset_server` is the boxed `AssetServer`
/// clone `nativeAssetServerAcquire` handed out; a zero pointer is a
/// contract break, not a 404.
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
        // SAFETY: `asset_server` is a boxed `AssetServer` clone
        // `nativeAssetServerAcquire` produced under the read lock; the
        // caller's `finally` drops it only after this call returns, so the
        // dereference can never race the free of the `create`-time box —
        // even on a `destroy()` that does not join the IO threads.
        let server = unsafe { &*(asset_server as *const AssetServer) };
        let method = get_string(env, &method)?;
        let path = get_string(env, &path)?;
        let query = if query.is_null() {
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
            WEBVIEW_ASSET_RESPONSE_INIT.signature,
            &[
                JValue::Int(status),
                JValue::Object(headers.as_ref()),
                JValue::Object(body.as_ref()),
            ],
        )?;
        Ok(response.into_raw())
    })
}

/// Drop the `create`-time `Box<AssetServer>` after a render-process-gone
/// teardown took it under the asset lock's write half — the handle's drop
/// frees the pointer `release` returns itself. Interceptions holding clones
/// keep the server alive until their `nativeAssetServerRelease`.
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
        // pointer; cloned handles keep the server alive until their
        // `nativeAssetServerRelease`.
        unsafe { drop(Box::from_raw(asset_server as *mut AssetServer)) };
        Ok(())
    });
}
