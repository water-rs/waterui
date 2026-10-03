//! JNI entry points for structural transitions.
//!
//! `Metadata<TransitionSpec>` reaches Kotlin as a `MetadataTransitionStruct`
//! (content plus the owning spec handle) built by `forceAsMetadataTransition`.
//! On a structural removal the backend detaches semantic membership, retains
//! the last layout as a fixed slot, then starts a `WuiTransitionState` clock
//! through `transitionStart`. A pixel effect reads window-relative geometry
//! from the clock's frame binding, updated by `transitionSetFrame` after every
//! placement, and its animation definition through `transitionAnimation`. The
//! spec is released by `transitionDropSpec`, the clock by `transitionDropState`
//! once `transitionAdvance` reports the lifetime has ended.

use jni::EnvUnowned;
use jni::objects::JClass;
use jni::sys::{jboolean, jfloat, jlong, jobject};

use super::components::pack_animation;
use super::convert::{jlong_to_ptr, jlong_to_ptr_mut, struct_to_java};
use super::with_env;
use crate::events::transition::{
    WuiTransitionFrame, WuiTransitionSpec, WuiTransitionState, waterui_transition_advance,
    waterui_transition_animation, waterui_transition_body, waterui_transition_captures_content,
    waterui_transition_drop_spec, waterui_transition_drop_state, waterui_transition_properties,
    waterui_transition_set_frame, waterui_transition_start,
};

/// Reassembles the flattened Kotlin frame arguments into `WuiTransitionFrame`.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the flattened C ABI frame descriptor"
)]
const fn transition_frame(
    source_x: jfloat,
    source_y: jfloat,
    source_width: jfloat,
    source_height: jfloat,
    window_width: jfloat,
    window_height: jfloat,
    scale_factor: jfloat,
    right_to_left: jboolean,
) -> WuiTransitionFrame {
    WuiTransitionFrame {
        source_x,
        source_y,
        source_width,
        source_height,
        window_width,
        window_height,
        scale_factor,
        right_to_left,
    }
}

/// Starts an independent transition clock; the returned handle is released by
/// `transitionDropState` after `transitionAdvance` returns false.
///
/// `spec` is borrowed. The frame fields carry the retained slot's initial
/// window-relative geometry — `source*` in window logical coordinates,
/// `window*` the overlay's logical size, `scaleFactor` device pixels per
/// logical pixel, `rightToLeft` the layout direction. Push updates with
/// `transitionSetFrame` after every placement.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionStart<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    spec: jlong,
    removal: jboolean,
    reduce_motion: jboolean,
    source_x: jfloat,
    source_y: jfloat,
    source_width: jfloat,
    source_height: jfloat,
    window_width: jfloat,
    window_height: jfloat,
    scale_factor: jfloat,
    right_to_left: jboolean,
) -> jlong {
    // SAFETY: Kotlin passes back the live spec handle `forceAsMetadataTransition`
    // handed it, borrowed for this call on the UI thread.
    unsafe {
        waterui_transition_start(
            jlong_to_ptr::<WuiTransitionSpec>(spec),
            removal,
            reduce_motion,
            transition_frame(
                source_x,
                source_y,
                source_width,
                source_height,
                window_width,
                window_height,
                scale_factor,
                right_to_left,
            ),
        ) as jlong
    }
}

/// Pushes the slot's current window-relative geometry into the transition.
///
/// Call after every placement — including scrolling and ancestor movement —
/// so the pixel overlay reads the live frame through the signal the effect
/// subscribed to at `transitionBody`.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionSetFrame<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
    source_x: jfloat,
    source_y: jfloat,
    source_width: jfloat,
    source_height: jfloat,
    window_width: jfloat,
    window_height: jfloat,
    scale_factor: jfloat,
    right_to_left: jboolean,
) {
    // SAFETY: Kotlin passes back a live clock handle, borrowed for this call on
    // the UI thread.
    unsafe {
        waterui_transition_set_frame(
            jlong_to_ptr::<WuiTransitionState>(state),
            transition_frame(
                source_x,
                source_y,
                source_width,
                source_height,
                window_width,
                window_height,
                scale_factor,
                right_to_left,
            ),
        );
    }
}

/// Advances the clock by `deltaNs` from the host frame tick; false means the
/// transition's lifetime has ended and the slot may be released.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionAdvance<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
    delta_ns: jlong,
) -> jboolean {
    // SAFETY: the UI thread calls one frame tick at a time, so this exclusive
    // borrow is the only live reference for the call.
    unsafe {
        waterui_transition_advance(
            jlong_to_ptr_mut::<WuiTransitionState>(state),
            delta_ns.cast_unsigned(),
        )
    }
}

/// Samples the native visual properties for the current untransformed slot
/// geometry as a `TransitionPropertiesStruct`.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionProperties<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
    width: jfloat,
    height: jfloat,
    right_to_left: jboolean,
    absent_endpoint: jboolean,
) -> jobject {
    // SAFETY: Kotlin passes back a live clock handle, borrowed for this call on
    // the UI thread.
    let properties = unsafe {
        waterui_transition_properties(
            jlong_to_ptr::<WuiTransitionState>(state),
            width,
            height,
            right_to_left,
            absent_endpoint,
        )
    };
    with_env(&mut env, |env| struct_to_java(env, properties).into_raw())
}

/// Returns the resolved effect's animation definition as an `AnimationStruct`
/// carrying the same packed kind/duration and parameter pairs as the
/// `getAnimation*Packed` accessors.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionAnimation<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
) -> jobject {
    // SAFETY: Kotlin passes back a live clock handle, borrowed for this call on
    // the UI thread.
    let animation =
        unsafe { waterui_transition_animation(jlong_to_ptr::<WuiTransitionState>(state)) };
    let (kind_duration, params12, params34) = pack_animation(&animation);
    with_env(&mut env, |env| {
        env.new_object(
            jni::jni_str!("dev/waterui/android/runtime/AnimationStruct"),
            jni::jni_sig!("(JJJ)V"),
            &[
                jni::objects::JValue::Long(kind_duration),
                jni::objects::JValue::Long(params12),
                jni::objects::JValue::Long(params34),
            ],
        )
        .expect("Failed to create AnimationStruct")
        .into_raw()
    })
}

/// Whether the resolved phase uses the GPU capture/effect path.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionCapturesContent<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
) -> jboolean {
    // SAFETY: Kotlin passes back a live clock handle, borrowed for this call on
    // the UI thread.
    unsafe { waterui_transition_captures_content(jlong_to_ptr::<WuiTransitionState>(state)) }
}

/// Constructs the pixel effect once; `content` is an owning view handle
/// consumed by the call and the returned handle is the transferred body.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionBody<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
    content: jlong,
) -> jlong {
    // SAFETY: `content` is an owning view handle consumed exactly once and the
    // state is live for the borrow, both on the UI thread.
    unsafe {
        waterui_transition_body(
            jlong_to_ptr::<WuiTransitionState>(state),
            jlong_to_ptr_mut::<crate::WuiAnyView>(content),
        ) as jlong
    }
}

/// Releases one declaration; existing clocks remain independently owned.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionDropSpec<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    spec: jlong,
) {
    // SAFETY: Kotlin drops the owning spec handle exactly once, after every
    // clock that referenced it was released.
    unsafe { waterui_transition_drop_spec(jlong_to_ptr_mut::<WuiTransitionSpec>(spec)) }
}

/// Releases a clock after `transitionAdvance` reported it finished.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_transitionDropState<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    state: jlong,
) {
    // SAFETY: Kotlin drops the owning clock handle exactly once on the UI
    // thread, after frame ticks stopped calling into it.
    unsafe { waterui_transition_drop_state(jlong_to_ptr_mut::<WuiTransitionState>(state)) }
}
