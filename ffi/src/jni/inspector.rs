//! JNI glue for bringing up the inspector from the Android runtime.
//!
//! Android reaches the environment as a raw pointer it already holds, so these
//! mirror the C entry points in [`crate::runtime::inspector`] rather than
//! adding anything new: the endpoint runs inside the application either way.

use jni::EnvUnowned;
use jni::objects::JClass;
use jni::sys::{jboolean, jlong};

use crate::WuiEnv;

/// Reads the environment a Kotlin caller holds, if it is not null.
///
/// # Safety
///
/// `env_ptr` must be an environment handle the runtime is keeping alive.
const unsafe fn environment<'a>(env_ptr: jlong) -> Option<&'a WuiEnv> {
    let pointer = env_ptr as *const WuiEnv;
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the caller contract makes this a live handle for the call.
    Some(unsafe { crate::borrow_ffi(pointer) })
}

/// Whether this build offers inspection at all.
///
/// Asked before showing "Inspect element", so a release build shows nothing
/// rather than an entry that does nothing.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_InspectorJni_isAvailable<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    env_ptr: jlong,
) -> jboolean {
    // SAFETY: the Kotlin caller passes the handle the runtime holds for the
    // lifetime of the application.
    let available = unsafe { environment(env_ptr) }
        .is_some_and(|env| env.get::<waterui::inspector::InspectorRuntime>().is_some());
    jboolean::from(available)
}

/// Opens the inspector on this application.
///
/// A phone is inspected from the developer's computer, so this reports where to
/// attach rather than launching anything on the device.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_InspectorJni_open<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    env_ptr: jlong,
) {
    // SAFETY: as above.
    let Some(env) = (unsafe { environment(env_ptr) }) else {
        return;
    };
    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
        return;
    };
    inspector.open();
}

/// Reveals one accessibility node in the inspector.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_InspectorJni_inspectNode<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    env_ptr: jlong,
    node: jlong,
) {
    // SAFETY: as above.
    let Some(env) = (unsafe { environment(env_ptr) }) else {
        return;
    };
    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
        return;
    };
    #[expect(
        clippy::cast_sign_loss,
        reason = "a node id is an opaque bit pattern; Java has no unsigned long"
    )]
    inspector.inspect_node(waterui::inspector::protocol::NodeId(node as u64));
}

/// Whether anything is watching the accessibility tree.
///
/// Android walks its view hierarchy only when the answer is yes.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_InspectorJni_wantsTree<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    env_ptr: jlong,
) -> jboolean {
    // SAFETY: the Kotlin caller passes the handle the runtime holds for the
    // lifetime of the application.
    let wants = unsafe { environment(env_ptr) }.is_some_and(|env| {
        env.get::<waterui::inspector::TreeRecorder>()
            .is_some_and(waterui::inspector::TreeRecorder::is_active)
    });
    jboolean::from(wants)
}
