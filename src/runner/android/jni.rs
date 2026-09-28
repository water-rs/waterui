//! The JNI entry points behind `dev.waterui.hydrolysis.NativeBridge`.
//!
//! Every export is a thin edge: it decodes arguments, calls the session
//! method, and maps failure to Java exceptions — a [`JniError`] or a panic
//! becomes `IllegalStateException`, so a GPU or lifecycle failure is an
//! explicit error on the Kotlin side, never a silent drop. Sessions are raw
//! pointers carried as `jlong` and driven only on the UI thread, which the
//! Kotlin host guarantees.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring};
use jni::{JNIEnv, JavaVM};

use crate::platform::{InputEvent, Modifiers, PointerButton, PointerKind};

use super::host::{AndroidSession, MetricsSnapshot};

/// The JNI schema this build of the runner speaks — `nativeInit` returns it
/// and the Kotlin `NativeBridge` refuses a mismatch, so a stale native
/// library cannot load against a newer host.
pub(crate) const JNI_SCHEMA: jint = 1;

/// A failure crossing the JNI boundary as an exception.
#[derive(Debug)]
pub(crate) struct JniError(pub String);

impl From<jni::errors::Error> for JniError {
    fn from(error: jni::errors::Error) -> Self {
        Self(format!("hydrolysis android: {error}"))
    }
}

impl From<super::gpu::GpuError> for JniError {
    fn from(error: super::gpu::GpuError) -> Self {
        Self(error.0)
    }
}

/// The `JavaVM` the host runs against, captured at `nativeInit` — JNI calls
/// back into Kotlin (redraw requests, IME state, accessibility publishes)
/// attach envs through it.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// Decodes a session pointer. The host passes exactly what
/// `nativeCreateSession` returned; anything else is a programming error.
fn session(ptr: jlong) -> &'static mut AndroidSession {
    assert!(ptr != 0, "hydrolysis android: null session pointer");
    // SAFETY: the pointer came from Box::into_raw on a session that the
    // Kotlin host owns, and every entry point runs on the UI thread, so no
    // aliased &mut ever coexists.
    unsafe { &mut *(ptr as *mut AndroidSession) }
}

/// Runs `f` on the session, mapping JniError → `IllegalStateException` and a
/// panic → `IllegalStateException` (with the panic payload in the message).
fn guard<F>(env: &mut JNIEnv, f: F)
where
    F: FnOnce(&mut JNIEnv) -> Result<(), JniError>,
{
    match catch_unwind(AssertUnwindSafe(|| f(env))) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = env.throw_new("java/lang/IllegalStateException", &error.0);
        }
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                format!("hydrolysis android: {message}"),
            );
        }
    }
}

/// `guard` for calls returning a value — the error path throws and returns
/// `default`.
fn guard_val<F, T>(env: &mut JNIEnv, default: T, f: F) -> T
where
    F: FnOnce(&mut JNIEnv) -> Result<T, JniError>,
{
    let mut result = default;
    guard(env, |env| {
        result = f(env)?;
        Ok(())
    });
    result
}

/// `guard` for calls returning a value the caller converts first — throws
/// and returns `default` on failure.
fn guard_string<F>(env: &mut JNIEnv, f: F) -> jstring
where
    F: FnOnce(&mut JNIEnv) -> Result<Option<String>, JniError>,
{
    let mut out: jstring = std::ptr::null_mut();
    guard(env, |env| {
        if let Some(json) = f(env)? {
            let value = env.new_string(json)?;
            out = value.into_raw();
        }
        Ok(())
    });
    out
}

pub(crate) fn get_string(env: &mut JNIEnv, value: &JString) -> Result<String, JniError> {
    env.get_string(value)
        .map(|s| s.into())
        .map_err(JniError::from)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    _version: jint,
) -> jint {
    guard_val(&mut env, 0, |env| {
        let vm = env.get_java_vm()?;
        let _ = JAVA_VM.set(vm);
        Ok(JNI_SCHEMA)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeCreateSession(
    mut env: JNIEnv,
    _class: JClass,
    host_view: JObject,
    sdk_int: jint,
) -> jlong {
    guard_val(&mut env, 0, |env| {
        let vm = env.get_java_vm()?;
        let _ = JAVA_VM.set(env.get_java_vm()?);
        let host_view = env.new_global_ref(&host_view)?;
        // Metrics arrive through `nativeSetMetrics` on the first layout —
        // the session starts zero-sized and the Resize event moves it.
        let metrics = MetricsSnapshot {
            width_px: 0,
            height_px: 0,
            density: 1.0,
            font_scale: 1.0,
            refresh_hz: None,
            insets_px: [0; 4],
        };
        let session = AndroidSession::create(vm, host_view, metrics, sdk_int)?;
        Ok(Box::into_raw(session) as jlong)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeDestroySession(
    _env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
) {
    if session_ptr == 0 {
        return;
    }
    // SAFETY: the pointer came from nativeCreateSession and this is the
    // single destroy call per session, on the UI thread.
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(session_ptr as *mut AndroidSession));
    }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetMetrics(
    mut env: JNIEnv,
    _class: JClass,
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
) {
    guard(&mut env, |_env| {
        session(session_ptr).set_metrics(MetricsSnapshot {
            width_px: width_px.max(0) as u32,
            height_px: height_px.max(0) as u32,
            density: f64::from(density).max(f64::EPSILON),
            font_scale: f64::from(font_scale).max(f64::EPSILON),
            refresh_hz: (refresh_hz > 0.0).then_some(f64::from(refresh_hz)),
            insets_px: [inset_l, inset_t, inset_r, inset_b],
        });
        Ok(())
    });
}

/// The Choreographer callback. Returns a bitmask: `1` = the engine wants
/// another frame; `2` = the window asked to close. A deadline is read out
/// with `nativeFrameDeadlineInNanos` when `4` is set.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeOnFrame(
    mut env: JNIEnv,
    _class: JClass,
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
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeFrameDeadlineInNanos(
    _env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
) -> jlong {
    if session_ptr == 0 {
        -1
    } else {
        session(session_ptr).frame_deadline_in_nanos()
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSurfaceAttached(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    surface: JObject,
    width: jint,
    height: jint,
    generation: jlong,
) -> jboolean {
    guard_val(&mut env, 0, |env| {
        let window = unsafe {
            // SAFETY: env is a live JNIEnv and `surface` is the
            // android.view.Surface the band just produced; the call acquires
            // the reference the attachment owns.
            ndk::native_window::NativeWindow::from_surface(
                env.get_native_interface(),
                surface.as_raw(),
            )
        };
        let Some(window) = window else {
            return Err(JniError(
                "hydrolysis android: ANativeWindow_fromSurface returned null".to_owned(),
            ));
        };
        session(session_ptr)
            .surface_attached_with_generation(
                window,
                width.max(0) as u32,
                height.max(0) as u32,
                generation as u64,
            )
            .map_err(JniError)?;
        Ok(1)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSurfaceChanged(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    width: jint,
    height: jint,
    generation: jlong,
) {
    guard(&mut env, |_env| {
        session(session_ptr)
            .surface_resized(width.max(0) as u32, height.max(0) as u32, generation as u64)
            .map_err(JniError)
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSurfaceDestroyed(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    generation: jlong,
) {
    guard(&mut env, |_env| {
        session(session_ptr).surface_detached(generation as u64);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetHighRefresh(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    fps: jfloat,
) {
    guard(&mut env, |_env| {
        session(session_ptr).set_high_refresh_demand((fps > 0.0).then_some(fps));
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// Input

/// The MotionEvent tool-type constants the host maps onto `PointerKind`.
fn pointer_kind(tool_type: jint) -> PointerKind {
    // android.view.MotionEvent.TOOL_TYPE_{FINGER,STYLUS,MOUSE,ERASER}
    match tool_type {
        2 | 4 => PointerKind::Pen,
        3 => PointerKind::Mouse,
        _ => PointerKind::Touch,
    }
}

/// One pointer event already decoded by the host: `action` is the
/// MotionEvent masked action (0 down, 1 up, 2 move, 3 cancel — matching
/// `MotionEvent.ACTION_*`), `button` maps `getActionButton` for stylus
/// barrel presses (0 primary, 1 secondary, 2 middle).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativePointerEvent(
    mut env: JNIEnv,
    _class: JClass,
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

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeScrollEvent(
    mut env: JNIEnv,
    _class: JClass,
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
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeKeyEvent(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    key: JString,
    pressed: jboolean,
    shift: jboolean,
    ctrl: jboolean,
    alt: jboolean,
    meta: jboolean,
) {
    guard(&mut env, |env| {
        let key = get_string(env, &key)?;
        let modifiers = Modifiers {
            shift: shift != 0,
            control: ctrl != 0,
            alt: alt != 0,
            super_key: meta != 0,
        };
        let session = session(session_ptr);
        session
            .ime
            .key_event(&mut session.runtime.platform, key, pressed != 0, modifiers);
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// IME

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeSetComposingText(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    text: JString,
    caret: jint,
) {
    guard(&mut env, |env| {
        let text = get_string(env, &text)?;
        let caret = usize::try_from(caret).unwrap_or(0);
        let session = session(session_ptr);
        session
            .ime
            .set_composing_text(&mut session.runtime.platform, text, caret);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeCommitText(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    text: JString,
) {
    guard(&mut env, |env| {
        let text = get_string(env, &text)?;
        let session = session(session_ptr);
        session.ime.commit_text(&mut session.runtime.platform, text);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeFinishComposingText(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
) {
    guard(&mut env, |_env| {
        let session = session(session_ptr);
        session.ime.finish_composing(&mut session.runtime.platform);
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// Accessibility + platform views

/// The serialized accesskit `TreeUpdate` the provider mirrors, or null when
/// nothing changed since the last read.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeAccessibilityTree(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
) -> jstring {
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

/// `value` carries the ACTION_SET_TEXT payload; empty string means no data.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativeAccessibilityAction(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
    virtual_view_id: jlong,
    action: jint,
    value: JString,
) -> jboolean {
    guard_val(&mut env, 0, |env| {
        let value = get_string(env, &value)?;
        Ok(super::accessibility::perform_action(
            session(session_ptr),
            virtual_view_id,
            action,
            (!value.is_empty()).then_some(value),
        )? as jboolean)
    })
}

/// The platform-view placement set as a JSON frame, or null when unchanged.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_NativeBridge_nativePlatformViewFrames(
    mut env: JNIEnv,
    _class: JClass,
    session_ptr: jlong,
) -> jstring {
    guard_string(&mut env, |_env| {
        session(session_ptr).platform_views.take_json()
    })
}
