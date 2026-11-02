//! The JNI entry points behind `dev.waterui.hydrolysis.NativeBridge`.
//!
//! Every export is a thin edge: it decodes arguments, calls the session
//! method, and maps failure to Java exceptions — a [`JniError`] or a panic
//! becomes `IllegalStateException`, so a GPU or lifecycle failure is an
//! explicit error on the Kotlin side, never a silent drop. Sessions are raw
//! pointers carried as `jlong` and driven only on the UI thread, which the
//! Kotlin host guarantees.

use std::cell::Cell;
use std::ptr::addr_of;
use std::sync::OnceLock;

use jni::errors::ErrorPolicy;
use jni::objects::{Global, JClass, JObject, JString};
use jni::strings::JNIString;
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong};
use jni::vm::JavaVM;
use jni::{Env, EnvUnowned, jni_str};

use crate::platform::{
    BackEdge, BackNavigation, InputEvent, Modifiers, PointerButton, PointerKind,
};

use super::host::{AndroidSession, MetricsSnapshot, UiThreadServices};

/// The JNI schema this build of the runner speaks — `nativeInit` returns it.
/// 24 adds `DefineEffect` (`effect, stage_count, kind + 12 floats` stages)
/// for native `RenderEffect` chains (#1750).
/// and the Kotlin `NativeBridge` refuses a mismatch, so a stale native
/// library cannot load against a newer host.
/// Schema history: 1 = initial surface/input/IME events; 2 =
/// `nativeSetVisible` (Activity `onStart`/`onStop` → pump visibility); 3 =
/// the `InputConnection` range protocol (`nativeEditOp`/`nativeEditingState`,
/// `onNativeEditingState`/`onNativeCursorAnchorInfo` pushes) and the
/// `Context` passed to `nativeCreateSession`; 4 = `nativeAccessibilityAction`
/// takes the accesskit action index plus selection-bounds, text and numeric
/// payload channels; 5 = `onNativeAccessibilityTreeChanged` carries the
/// diffed event-list JSON and `nativeAccessibilityHitTest` maps a point to
/// the served virtual node for explore-by-touch; 6 = `nativeInit` carries
/// the launch intent's `waterui.log.level` extra (the CLI's `--logs`
/// level) and logging init moves out of the app cdylib's `JNI_OnLoad`; 7 =
/// `nativeSetMetrics` carries the `ViewConfiguration` touch-scroll
/// parameters (slop, min/max fling velocity, scroll friction); 8 =
/// `nativeCreateSession` drops `sdkInt`. The API floor is 31, so
/// `ANativeWindow_setFrameRate` is linked directly; 9 = `nativeBackEvent`
/// and `onNativeBackAvailable` carry system back into the navigation stack
/// and report whether a back target is registered; 10 = `nativeSetMetrics`
/// splits the window insets into the container and keyboard regions of
/// layout-spec.md §7.1, and the host's `WindowInsetsAnimationCompat` progress
/// pushes each IME animation frame; 11 = `nativeUiThreadServices` creates
/// the one executor per UI thread at load time, and `nativeCreateSession`
/// takes its handle so every session shares it; 12 = `nativeSetHighRefresh`
/// takes a typed `active` flag in place of the `-1f` sentinel float the
/// runner decoded as a release, and `nativeSurfaceAttached` carries the
/// display's peak refresh rate that flag asks for; 13 = a platform-view
/// placement names either a factory `kind` or a registered `instance`, and
/// the `HydrolysisWebView` natives join the edge; 14 =
/// `nativePlatformViewFocus` reported whether a mounted platform-view child
/// held UI focus; 15 = the report reads through the frame instead —
/// `onNativePlatformViewFocus` / `platform_view_focus_inside` replaces the
/// pushed native, which left with it; 16 = `onNativeFatalError` leaves the
/// table — a lost GPU device is recreated instead of ending the session
/// (water-rs/waterui#2742), and a recreation that exhausts its loss budget
/// already reaches Kotlin as the frame call's `IllegalStateException`;
/// 17 = `nativeSetFrameDemand` separates continuous work and touch demand,
/// and surface attachment supplies the display's suggested ARR rates. 18 =
/// the unlanded HWUI render-target work (water-rs/waterui#1811); the reserved
/// number keeps a stale native/Kotlin pair from pairing up mid-development.
/// 19 = the HWUI text wire carries a default-style row: row zero of every
/// packed shape request is the paragraph's base paint. 20 = `nativeCreateSession`
/// takes the session-owned painter object, and `nativeSetHostAttached` reports
/// the host view's bind state (water-rs/waterui#1811). 21 = the
/// `BindPlatformView` op binds a platform view inside its owner's `RenderNode`.
/// 21->22 = `nativePlatformViewHitTest` answers the retained tree's platform
/// view under a host pixel — the HWUI painter's ordered-input query.
/// 23 = `nativeCreateSession` carries the configuration's night-mode flag so
/// the colour-scheme signal starts on the real `uiMode`, and
/// `nativeSetColorScheme` reports every later change (water-rs/waterui#3544).
pub const JNI_SCHEMA: jint = 24;

/// A failure crossing the JNI boundary as an exception.
#[derive(Debug)]
pub struct JniError(pub String);

impl From<jni::errors::Error> for JniError {
    fn from(error: jni::errors::Error) -> Self {
        Self(format!("hydrolysis android: {error}"))
    }
}

#[cfg(hydrolysis_gpu_painter)]
impl From<super::gpu::GpuError> for JniError {
    fn from(error: super::gpu::GpuError) -> Self {
        Self(error.0)
    }
}

/// The `JavaVM` the host runs against, captured at `nativeInit` — JNI calls
/// back into Kotlin (redraw requests, IME state, accessibility publishes)
/// attach envs through it.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// The process's `JavaVM`, captured at `nativeInit`. Entry points receive a
/// `JNIEnv` from their caller, but code that runs outside a Java-to-native
/// entry point — the executor's `ALooper` fd callback — resolves the env
/// through this.
pub(super) fn java_vm() -> &'static JavaVM {
    JAVA_VM
        .get()
        .expect("hydrolysis android: nativeInit sets JAVA_VM before any session exists")
}

/// The `Application` published to `ndk_context`, held as a JNI global
/// reference for the life of the process.
///
/// `ndk_context` stores the context pointer process-wide and hands it to
/// every service crate that resolves it at use time (waterkit-clipboard's
/// Android backend reads it through `ndk_context::android_context`), so the
/// reference it points at must outlive every such use: this slot owns it and
/// is never cleared. It holds the `Application`, never an `Activity`, so
/// nothing keeps a destroyed activity alive.
static APPLICATION_CONTEXT: OnceLock<Global<JObject<'static>>> = OnceLock::new();

/// Publishes the `Application` of `context` to `ndk_context`, exactly once
/// per process.
///
/// The first session publishes it; every later session — an activity
/// finished and relaunched in the same process — must carry the same
/// `Application` and leaves the published one in place. The `Application`
/// is a per-process singleton, so a different one is a broken contract and
/// fails the session instead of replacing (and leaking) the published
/// reference. Sessions are created only on the UI thread, so the check and
/// the publish do not race.
pub(super) fn publish_application_context(
    env: &mut Env,
    vm: &JavaVM,
    context: &JObject,
) -> Result<(), JniError> {
    let application = env
        .call_method(
            context,
            jni_str!("getApplicationContext"),
            jni::jni_sig!("()Landroid/content/Context;"),
            &[],
        )?
        .l()?;
    if let Some(published) = APPLICATION_CONTEXT.get() {
        return if env.is_same_object(published.as_obj(), &application)? {
            Ok(())
        } else {
            Err(JniError(
                "hydrolysis android: a session was created with an Application \
                 other than the one already published to ndk_context; the \
                 Application is a per-process singleton"
                    .to_owned(),
            ))
        };
    }
    let application = env.new_global_ref(&application)?;
    let raw = application.as_obj().as_raw();
    APPLICATION_CONTEXT
        .set(application)
        .expect("hydrolysis android: sessions are created only on the UI thread");
    // SAFETY: `ndk_context` keeps both pointers for the rest of the process.
    // `vm` is the process's single JavaVM, which lives as long as the
    // process. `raw` is the global reference now owned by
    // `APPLICATION_CONTEXT`, which is never cleared, so it stays valid on
    // every thread for the rest of the process. This is the only call to
    // `initialize_android_context`, guarded by that slot being empty, so
    // `ndk_context`'s at-most-once precondition holds.
    unsafe {
        ndk_context::initialize_android_context(vm.get_raw().cast(), raw.cast());
    }
    Ok(())
}

/// The heap allocation a session `jlong` points at. The Kotlin host owns
/// it for the session's life; every entry point borrows `session` through
/// [`session`], which `entered` makes exclusive.
///
/// `entered` sits beside `session` rather than inside it: a borrow is
/// checked and cleared through `&Cell` on the flag's own field while the
/// disjoint `session` field is lent mutably — the two never overlap.
struct SessionBox {
    entered: Cell<bool>,
    session: AndroidSession,
}

/// One JNI call's exclusive borrow of the session [`session`] names.
///
/// While the guard lives, a re-entrant [`session`] call — a Kotlin
/// callback synchronously invoking a native entry point under a session
/// call — panics rather than minting a second `&mut AndroidSession`
/// (water-rs/waterui#2953); `guard` reports the panic to that nested
/// caller as `IllegalStateException`, the same surface every entry
/// failure takes. Dropping the guard releases the borrow.
struct SessionGuard {
    entry: *mut SessionBox,
}

impl SessionGuard {
    /// The `entered` flag of this guard's box, read through a field
    /// projection so the shared borrow never covers the disjoint
    /// `session` field this guard lends mutably.
    fn entered(&self) -> &Cell<bool> {
        // SAFETY: `addr_of!` projects to the `entered` field without
        // borrowing the whole box, so this `&Cell` never aliases the
        // `&mut AndroidSession` derived from the disjoint `session`
        // field below.
        unsafe { &*addr_of!((*self.entry).entered) }
    }
}

impl std::ops::Deref for SessionGuard {
    type Target = AndroidSession;
    fn deref(&self) -> &AndroidSession {
        // SAFETY: `session()` proves no other borrow was live when it
        // minted this guard, and `entered` keeps a second guard from
        // existing until this one drops.
        unsafe { &(*self.entry).session }
    }
}

impl std::ops::DerefMut for SessionGuard {
    fn deref_mut(&mut self) -> &mut AndroidSession {
        // SAFETY: same argument as `deref` — the guard is the exclusive
        // borrow for the whole JNI call.
        unsafe { &mut (*self.entry).session }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.entered().set(false);
    }
}

/// Decodes a session pointer and borrows its session for this call.
/// The host passes exactly what `nativeCreateSession` returned; anything
/// else is a programming error.
///
/// # Panics
/// Panics when a borrow is already live: a Kotlin callback re-entered
/// JNI under a running session call, where minting a second `&mut
/// AndroidSession` would alias the first.
fn session(ptr: jlong) -> SessionGuard {
    assert!(ptr != 0, "hydrolysis android: null session pointer");
    // SAFETY: the pointer came from `Box::into_raw` on a `SessionBox` in
    // `nativeCreateSession`, and the Kotlin host keeps it alive until
    // `nativeDestroySession`.
    let entry = ptr as *mut SessionBox;
    // Read and set the flag before the guard exists: a failed assertion
    // has no guard to drop, so the live borrow's `entered` stays set
    // instead of being released under it.
    //
    // SAFETY: same projection argument as `SessionGuard::entered` — the
    // flag field never overlaps the disjoint `session` field.
    let entered = unsafe { &*addr_of!((*entry).entered) };
    assert!(
        !entered.replace(true),
        "hydrolysis android: a Kotlin callback re-entered JNI while a \
         session call was still running"
    );
    SessionGuard { entry }
}

/// Decodes the UI-thread services pointer. The host passes exactly what
/// `nativeUiThreadServices` returned — a process-lifetime object the load
/// hook created once.
fn services(ptr: jlong) -> &'static UiThreadServices {
    assert!(ptr != 0, "hydrolysis android: null services pointer");
    // SAFETY: the pointer came from Box::into_raw on the UiThreadServices
    // the load hook created and the Kotlin host keeps for the process's
    // life; sessions only ever read it, on the UI thread.
    unsafe { &*(ptr as *const UiThreadServices) }
}

/// The error policy every entry point resolves through: a `JniError` or a
/// panic becomes `IllegalStateException` (the panic's payload in the
/// message), so a GPU or lifecycle failure is an explicit error on the
/// Kotlin side, never a silent drop. The caller's default is the returned
/// value on either path.
struct ThrowIllegalState;

impl<T: Default> ErrorPolicy<T, JniError> for ThrowIllegalState {
    type Captures<'unowned_env_local: 'native_method, 'native_method> = T;

    fn on_error<'unowned_env_local: 'native_method, 'native_method>(
        env: &mut Env<'unowned_env_local>,
        default: &mut T,
        error: JniError,
    ) -> jni::errors::Result<T> {
        let _ = env.throw_new(
            jni_str!("java/lang/IllegalStateException"),
            JNIString::new(&error.0),
        );
        Ok(std::mem::take(default))
    }

    fn on_panic<'unowned_env_local: 'native_method, 'native_method>(
        env: &mut Env<'unowned_env_local>,
        default: &mut T,
        payload: Box<dyn std::any::Any + Send + 'static>,
    ) -> jni::errors::Result<T> {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        let _ = env.throw_new(
            jni_str!("java/lang/IllegalStateException"),
            JNIString::new(format!("hydrolysis android: {message}")),
        );
        Ok(std::mem::take(default))
    }
}

/// Runs `f` on the session inside the caller's JNI frame, mapping
/// `JniError` → `IllegalStateException` and a panic → `IllegalStateException`
/// (with the panic payload in the message).
pub fn guard<'local, F>(env: &mut EnvUnowned<'local>, f: F)
where
    F: FnOnce(&mut Env<'local>) -> Result<(), JniError>,
{
    env.with_env(f).resolve_with::<ThrowIllegalState, _>(|| ());
}

/// `guard` for calls returning a value — the error path throws and returns
/// `default`.
pub fn guard_val<'local, T, F>(env: &mut EnvUnowned<'local>, default: T, f: F) -> T
where
    T: Default,
    F: FnOnce(&mut Env<'local>) -> Result<T, JniError>,
{
    env.with_env(f)
        .resolve_with::<ThrowIllegalState, _>(|| default)
}

/// `guard` for calls returning a string — the error path throws and returns
/// null.
pub fn guard_string<'local, F>(env: &mut EnvUnowned<'local>, f: F) -> JString<'local>
where
    F: FnOnce(&mut Env<'local>) -> Result<Option<String>, JniError>,
{
    env.with_env(|env| match f(env)? {
        Some(json) => Ok(env.new_string(json)?),
        None => Ok(JString::default()),
    })
    .resolve_with::<ThrowIllegalState, _>(JString::default)
}

pub fn get_string(env: &mut Env, value: &JString) -> Result<String, JniError> {
    value.try_to_string(env).map_err(JniError::from)
}

/// `log_level` is the `waterui.log.level` launch extra (a `tracing` level
/// name), or null when the launch carried none — [`super::init_logging`]
/// keeps its INFO default then. An unrecognized name is a contract breach,
/// not something to guess around: it throws.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeInit<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    _version: jint,
    log_level: JString<'local>,
) -> jint {
    guard_val(&mut env, 0, |env| {
        // A null `waterui.log.level` extra keeps the INFO default.
        let level = if log_level.as_raw().is_null() {
            None
        } else {
            Some(get_string(env, &log_level)?)
        };
        init_process(env, level.as_deref())?;
        Ok(JNI_SCHEMA)
    })
}

/// The load hook's second half, after the schema handshake: creates the
/// UI-thread services — the one executor per UI thread, registered with the
/// main `ALooper`, plus the process environment that carries the
/// inspector — and returns them as an opaque handle the Kotlin `NativeBridge`
/// keeps for the process's life and hands to every `nativeCreateSession`.
///
/// Called once, on the UI thread (its main looper must already exist).
/// The returned pointer is process-owned and never freed.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeUiThreadServices<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jlong {
    guard_val(&mut env, 0, |_env| {
        let services = UiThreadServices::init()?;
        Ok(Box::into_raw(Box::new(services)) as jlong)
    })
}

/// The one-time process setup every Hydrolysis JNI entry runs first: capture
/// the `JavaVM` for the host's later calls, parse the caller's optional
/// `tracing` level name (`None` keeps [`super::init_logging`]'s default), and
/// install logging.
pub(super) fn init_process(env: &mut Env, log_level: Option<&str>) -> Result<(), JniError> {
    let vm = env.get_java_vm()?;
    let _ = JAVA_VM.set(vm);
    let level = log_level
        .map(|level| {
            level
                .parse::<tracing::level_filters::LevelFilter>()
                .map_err(|_| {
                    JniError(format!(
                        "hydrolysis android: unrecognized log level {level:?}"
                    ))
                })
        })
        .transpose()?;
    super::init_logging(level);
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeCreateSession<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    host_view: JObject<'local>,
    context: JObject<'local>,
    painter: JObject<'local>,
    services_ptr: jlong,
    night: jboolean,
) -> jlong {
    guard_val(&mut env, 0, |env| {
        let vm = env.get_java_vm()?;
        let _ = JAVA_VM.set(env.get_java_vm()?);
        publish_application_context(env, &vm, &context)?;
        let host_view = env.new_global_ref(&host_view)?;
        // Metrics arrive through `nativeSetMetrics` on the first layout —
        // the session starts zero-sized and the Resize event moves it.
        let metrics = MetricsSnapshot {
            width_px: 0,
            height_px: 0,
            density: 1.0,
            font_scale: 1.0,
            refresh_hz: None,
            container_insets_px: [0; 4],
            keyboard_insets_px: [0; 4],
            touch_slop_px: 0.0,
            min_fling_velocity_px: 0.0,
            max_fling_velocity_px: 0.0,
            scroll_friction: 0.0,
        };
        let session = AndroidSession::create(
            env,
            vm,
            host_view,
            &painter,
            metrics,
            night,
            services(services_ptr),
        )?;
        Ok(Box::into_raw(Box::new(SessionBox {
            entered: Cell::new(false),
            session: *session,
        })) as jlong)
    })
}

/// Frees the session. A panic while its state drops is reported to the
/// Kotlin caller as `IllegalStateException`, like every other entry point:
/// a teardown that failed halfway is a defect to see, not one to discard.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeDestroySession<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
) {
    guard(&mut env, |_env| {
        assert!(session_ptr != 0, "hydrolysis android: null session pointer");
        let entry = session_ptr as *mut SessionBox;
        assert!(
            // SAFETY: the pointer came from nativeCreateSession, and the
            // Kotlin session hands it here exactly once, on the UI thread,
            // after closing its accessor. The flag read is the same
            // disjoint-field projection `session` uses.
            !unsafe { &*addr_of!((*entry).entered) }.get(),
            "hydrolysis android: destroying a session while a call into \
             it is still running"
        );
        // SAFETY: same pointer and exclusivity as the flag read above —
        // `entered` being clear means no borrow of `session` is still live,
        // so reclaiming the box frees nothing a borrow points into.
        drop(unsafe { Box::from_raw(entry) });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetMetrics<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    width_px: jint,
    height_px: jint,
    density: jfloat,
    font_scale: jfloat,
    refresh_hz: jfloat,
    inset_l: jint,
    inset_t: jint,
    inset_r: jint,
    inset_b: jint,
    ime_l: jint,
    ime_t: jint,
    ime_r: jint,
    ime_b: jint,
    touch_slop: jfloat,
    min_fling_velocity: jfloat,
    max_fling_velocity: jfloat,
    scroll_friction: jfloat,
) {
    guard(&mut env, |_env| {
        session(session_ptr).set_metrics(MetricsSnapshot {
            width_px: crate::num_cast::i32_as_u32(width_px.max(0)),
            height_px: crate::num_cast::i32_as_u32(height_px.max(0)),
            density: f64::from(density).max(f64::EPSILON),
            font_scale: f64::from(font_scale).max(f64::EPSILON),
            refresh_hz: (refresh_hz > 0.0).then_some(f64::from(refresh_hz)),
            container_insets_px: [inset_l, inset_t, inset_r, inset_b],
            keyboard_insets_px: [ime_l, ime_t, ime_r, ime_b],
            touch_slop_px: touch_slop,
            min_fling_velocity_px: min_fling_velocity,
            max_fling_velocity_px: max_fling_velocity,
            scroll_friction: f64::from(scroll_friction),
        });
        Ok(())
    });
}

/// The Choreographer callback. Returns a bitmask: `1` = the engine wants
/// another frame; `2` = the window asked to close. A deadline is read out
/// with `nativeFrameDeadlineInNanos` when `4` is set.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeOnFrame<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    _vsync_nanos: jlong,
) -> jlong {
    guard_val(&mut env, 0, |_env| {
        let outcome = session(session_ptr).on_frame();
        let mut bits: jlong = 0;
        if outcome.wants_next_frame {
            bits |= 1;
        }
        if outcome.should_close {
            bits |= 2;
        }
        if outcome.deadline_in_nanos.is_some() {
            bits |= 4;
        }
        Ok(bits)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeFrameDeadlineInNanos<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
) -> jlong {
    guard_val(&mut env, -1, |_env| {
        Ok(session(session_ptr).frame_deadline_in_nanos())
    })
}

#[cfg(hydrolysis_gpu_painter)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSurfaceAttached<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    surface: JObject<'local>,
    peak_refresh_hz: jfloat,
    normal_refresh_hz: jfloat,
    high_refresh_hz: jfloat,
    width: jint,
    height: jint,
    generation: jlong,
) -> jboolean {
    guard_val(&mut env, false, |env| {
        if ![peak_refresh_hz, normal_refresh_hz, high_refresh_hz]
            .into_iter()
            .all(|rate| rate.is_finite() && rate > 0.0)
        {
            return Err(JniError(format!(
                "hydrolysis android: invalid display rates: peak={peak_refresh_hz}, \
                 normal={normal_refresh_hz}, high={high_refresh_hz}; rates must be positive and finite"
            )));
        }
        let window = unsafe {
            // SAFETY: env is a live JNI environment and `surface` is the
            // android.view.Surface the band just produced; the call acquires
            // the reference the attachment owns. ndk 0.9 binds `jni-sys`
            // 0.3's JNIEnv while jni 0.22 binds 0.4's — both are the
            // platform's `JNINativeInterface_` table pointer.
            ndk::native_window::NativeWindow::from_surface(env.get_raw().cast(), surface.as_raw())
        };
        let Some(window) = window else {
            return Err(JniError(
                "hydrolysis android: ANativeWindow_fromSurface returned null".to_owned(),
            ));
        };
        session(session_ptr)
            .surface_attached_with_generation(
                window,
                super::gpu::DisplayRefresh {
                    peak: peak_refresh_hz,
                    normal: normal_refresh_hz,
                    high: high_refresh_hz,
                },
                crate::num_cast::i32_as_u32(width.max(0)),
                crate::num_cast::i32_as_u32(height.max(0)),
                crate::num_cast::i64_as_u64(generation),
            )
            .map_err(JniError)?;
        Ok(true)
    })
}

#[cfg(hydrolysis_gpu_painter)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSurfaceChanged<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    width: jint,
    height: jint,
    generation: jlong,
) {
    guard(&mut env, |_env| {
        session(session_ptr).surface_resized(
            crate::num_cast::i32_as_u32(width.max(0)),
            crate::num_cast::i32_as_u32(height.max(0)),
            crate::num_cast::i64_as_u64(generation),
        );
        Ok(())
    });
}

#[cfg(hydrolysis_gpu_painter)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSurfaceDestroyed<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    generation: jlong,
) {
    guard(&mut env, |_env| {
        session(session_ptr).surface_detached(crate::num_cast::i64_as_u64(generation));
        Ok(())
    });
}

/// The configuration's night-mode flag — pushed by the host at bind and on
/// every `onConfigurationChanged`, into the session's colour-scheme signal.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetColorScheme<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    night: jboolean,
) {
    guard(&mut env, |_env| {
        session(session_ptr).set_night_mode(night);
        Ok(())
    });
}

/// The Activity's started state — `onStart`/`onStop` drive the pump's
/// hidden flag alongside the surface's own attach/detach.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetVisible<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    visible: jboolean,
) {
    guard(&mut env, |_env| {
        session(session_ptr).set_visible(visible);
        Ok(())
    });
}

/// The retained tree's platform view under host pixel (`x`, `y`) — the
/// ordered hit test the Kotlin host's gesture dispatch asks before latching
/// a native gesture. `0` means no view is drawn there or an interactive
/// target painted above it takes the point; non-interactive content never
/// blocks a view. HWUI builds only: the GPU path's slots own their own hit
/// order.
#[cfg(hydrolysis_hwui)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativePlatformViewHitTest<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    x: jfloat,
    y: jfloat,
) -> jlong {
    use crate::platform::PlatformWindow as _;

    guard_val(&mut env, 0, |_env| {
        let session = session(session_ptr);
        let id = session
            .runtime
            .renderer
            .platform_view_hit(
                session.hwui.surface.encoder(),
                kurbo::Point::new(f64::from(x), f64::from(y)),
                session.runtime.platform.scale_factor(),
            )
            .map_err(|e| JniError(e.to_string()))?;
        Ok(id.map_or(0, |id| i64::from_ne_bytes(id.to_ne_bytes())))
    })
}

/// The Kotlin host bound (`true`) or unbound (`false`) its view: the
/// session's attach gate — the HWUI painter's draw window and the common
/// occlusion report's host half. GPU builds keep their own surface
/// callbacks; this only records the host's binding state.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetHostAttached<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    attached: jboolean,
) {
    guard(&mut env, |_env| {
        session(session_ptr).set_host_attached(attached);
        Ok(())
    });
}

/// The host's continuous-work and touch demand. The session combines these
/// with the current engine content-rate range before voting on its window.
#[cfg(hydrolysis_gpu_painter)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetFrameDemand<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    continuous: jboolean,
    touching: jboolean,
) {
    guard(&mut env, |_env| {
        session(session_ptr)
            .set_frame_demand(continuous, touching)
            .map_err(JniError)
    });
}

// ---------------------------------------------------------------------------
// Input

/// The `MotionEvent` tool-type constants the host maps onto `PointerKind`.
const fn pointer_kind(tool_type: jint) -> PointerKind {
    // android.view.MotionEvent.TOOL_TYPE_{FINGER,STYLUS,MOUSE,ERASER}
    match tool_type {
        2 | 4 => PointerKind::Pen,
        3 => PointerKind::Mouse,
        _ => PointerKind::Touch,
    }
}

/// One pointer event already decoded by the host: `action` is the
/// `MotionEvent` masked action (0 down, 1 up, 2 move, 3 cancel — matching
/// `MotionEvent.ACTION_*`), `button` maps `getActionButton` for stylus
/// barrel presses (0 primary, 1 secondary, 2 middle).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativePointerEvent<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    action: jint,
    pointer_id: jint,
    x: jfloat,
    y: jfloat,
    tool_type: jint,
    button: jint,
) {
    guard(&mut env, |_env| {
        let kind = pointer_kind(tool_type);
        let button = match button {
            1 => PointerButton::Secondary,
            2 => PointerButton::Middle,
            _ => PointerButton::Primary,
        };
        let id = u64::try_from(pointer_id).unwrap_or(0);
        let event = match action {
            0 => InputEvent::PointerDown {
                id,
                kind,
                x,
                y,
                button,
            },
            1 => InputEvent::PointerUp {
                id,
                kind,
                x,
                y,
                button,
            },
            2 => InputEvent::PointerMove { id, kind, x, y },
            _ => InputEvent::PointerCancel { id, kind },
        };
        session(session_ptr).runtime.platform.push_event(event);
        Ok(())
    });
}

/// One system-back phase. `phase` is 0 Started, 1 Progressed, 2 Cancelled,
/// 3 Invoked. `edge` is the platform swipe edge and is read only for Started:
/// `BackEvent.EDGE_LEFT` (0), `EDGE_RIGHT` (1), or `EDGE_NONE` (2) — the value
/// a back button's predictive animation carries. `progress` is the platform's
/// `0..=1` report.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeBackEvent<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    phase: jint,
    edge: jint,
    progress: jdouble,
) {
    guard(&mut env, |_env| {
        let event = match phase {
            0 => {
                let edge = match edge {
                    0 => BackEdge::Left,
                    1 => BackEdge::Right,
                    2 => BackEdge::None,
                    other => panic!("hydrolysis android: unknown back edge {other}"),
                };
                BackNavigation::Started { edge }
            }
            1 => BackNavigation::Progressed { progress },
            2 => BackNavigation::Cancelled,
            3 => BackNavigation::Invoked,
            other => panic!("hydrolysis android: unknown back phase {other}"),
        };
        session(session_ptr)
            .runtime
            .platform
            .push_event(InputEvent::BackNavigation(event));
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeScrollEvent<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    x: jfloat,
    y: jfloat,
    dx: jfloat,
    dy: jfloat,
) {
    guard(&mut env, |_env| {
        session(session_ptr)
            .runtime
            .platform
            .push_event(InputEvent::Scroll {
                x,
                y,
                dx,
                dy,
                is_line_delta: false,
            });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeKeyEvent<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    key: JString<'local>,
    pressed: jboolean,
    shift: jboolean,
    ctrl: jboolean,
    alt: jboolean,
    meta: jboolean,
) {
    guard(&mut env, |env| {
        let key = get_string(env, &key)?;
        let modifiers = Modifiers {
            shift,
            control: ctrl,
            alt,
            super_key: meta,
        };
        let mut session = session(session_ptr);
        super::ime::ImeBridge::key_event(&mut session.runtime.platform, &key, pressed, modifiers);
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// IME — the InputConnection range protocol. One multiplexed entry point
// carries every mutator (the opcodes live in `android/ime.rs` and
// `HydrolysisInputConnection.kt`); the state push/pull travels as JSON.

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeEditOp<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    editor_id: jlong,
    op: jint,
    arg1: jint,
    arg2: jint,
    text: JString<'local>,
) -> jboolean {
    guard_val(&mut env, false, |env| {
        let text = get_string(env, &text)?;
        Ok(session(session_ptr).edit_op(
            crate::num_cast::i64_as_u64(editor_id),
            op,
            arg1,
            arg2,
            &text,
        ))
    })
}

/// The connection's synchronous pull at bind time: the authoritative
/// `EditingState` JSON — its `editorId` becomes the connection's generation
/// token, and `focused=false` marks the connection dead on arrival.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeEditingState<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
) -> JString<'local> {
    guard_string(&mut env, |_env| {
        Ok(Some(super::ime::editing_state_json(
            &session(session_ptr).ime.session.state(),
        )))
    })
}

// ---------------------------------------------------------------------------
// Accessibility + platform views

/// The serialized accesskit `TreeUpdate` the provider mirrors, or null when
/// nothing changed since the last read.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeAccessibilityTree<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
) -> JString<'local> {
    guard_string(&mut env, |_env| {
        #[cfg(feature = "accessibility")]
        {
            Ok(session(session_ptr).a11y.take_json().map(str::to_owned))
        }
        #[cfg(not(feature = "accessibility"))]
        {
            let _ = session_ptr;
            Ok(None)
        }
    })
}

/// The served virtual node under `(x, y)` in logical units, or -1 — the
/// hover hit test the host's `dispatchHoverEvent` consults for
/// explore-by-touch.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeAccessibilityHitTest<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    x: jfloat,
    y: jfloat,
) -> jlong {
    guard_val(&mut env, -1, |_env| {
        #[cfg(feature = "accessibility")]
        {
            Ok(session(session_ptr)
                .a11y
                .published()
                .and_then(|update| {
                    crate::runner::android_accessibility::hit_test(
                        update,
                        f64::from(x),
                        f64::from(y),
                    )
                })
                .map_or(-1, |id| crate::num_cast::u64_as_i64(id.0)))
        }
        #[cfg(not(feature = "accessibility"))]
        {
            let _ = (session_ptr, x, y);
            Ok(-1)
        }
    })
}

/// `action` is the accesskit action index the provider decoded from the
/// node's `actions` bitmask; `arg1`/`arg2` carry the `SetTextSelection`
/// UTF-16 bounds (-1 means none), `text` a string payload (empty string
/// means none) and `numeric` a numeric one (NaN means none) — only the
/// channels the action's data kind uses are ever set.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeAccessibilityAction<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
    virtual_view_id: jlong,
    action: jint,
    arg1: jint,
    arg2: jint,
    text: JString<'local>,
    numeric: jdouble,
) -> jboolean {
    guard_val(&mut env, false, |env| {
        let text = get_string(env, &text)?;
        super::accessibility::perform_action(
            &mut session(session_ptr),
            virtual_view_id,
            action,
            arg1,
            arg2,
            (!text.is_empty()).then_some(text),
            (!numeric.is_nan()).then_some(numeric),
        )
    })
}

/// The platform-view placement set as a JSON frame, or null when unchanged.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativePlatformViewFrames<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    session_ptr: jlong,
) -> JString<'local> {
    guard_string(&mut env, |_env| {
        Ok(Some(super::platform_views::placements_json(&session(
            session_ptr,
        ))?))
    })
}
