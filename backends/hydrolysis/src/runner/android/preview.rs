//! The preview entry `water preview --platform android` runs: the launcher
//! cdylib, built in `waterui-preview-mode`, registers `preview_runtime::run`
//! here from `JNI_OnLoad`, and the preview host's instrumentation calls it
//! through `dev.waterui.hydrolysis.preview.PreviewBridge`.

use std::sync::OnceLock;

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::jint;

use super::jni::{JniError, guard, guard_val, init_process, publish_application_context};

/// Incremented in lock-step with `PreviewBridge.SCHEMA` in the host's
/// `preview` module. History: 1 = `nativeInit(schema, logLevel)` and
/// `nativeRunPreview(context)`.
pub const PREVIEW_JNI_SCHEMA: jint = 1;

static PREVIEW_ENTRY: OnceLock<fn()> = OnceLock::new();

/// Registers the preview the instrumentation runs. Called once, from the
/// launcher cdylib's `JNI_OnLoad` in preview mode.
///
/// # Panics
/// Panics when called a second time.
pub fn register_preview(entry: fn()) {
    assert!(
        PREVIEW_ENTRY.set(entry).is_ok(),
        "register_preview called twice — each JNI_OnLoad registers exactly one preview"
    );
}

/// Schema handshake plus the shared process setup. A library built against a
/// different schema reports so rather than letting the host call entry points
/// that changed underneath it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_preview_PreviewBridge_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    _schema: jint,
    log_level: JString,
) -> jint {
    guard_val(&mut env, 0, |env| {
        init_process(env, &log_level)?;
        Ok(PREVIEW_JNI_SCHEMA)
    })
}

/// Runs the registered preview under the instrumentation's application
/// context. A panic inside the entry becomes an `IllegalStateException`
/// through [`guard`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_hydrolysis_preview_PreviewBridge_nativeRunPreview(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
) {
    guard(&mut env, |env| {
        let vm = env.get_java_vm()?;
        publish_application_context(env, &vm, &context)?;
        let entry = PREVIEW_ENTRY.get().copied().ok_or_else(|| {
            JniError(
                "hydrolysis android: no preview registered — the launcher cdylib must call \
                 hydrolysis::android::register_preview from JNI_OnLoad in preview mode"
                    .to_owned(),
            )
        })?;
        entry();
        Ok(())
    });
}
