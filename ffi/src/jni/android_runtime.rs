//! JNI glue for the Android runtime lifecycle.

use jni::EnvUnowned;
use jni::objects::{Global, JClass, JObject};
use jni::sys::jlong;

struct AndroidContextOwner {
    activity: Global<JObject<'static>>,
}

/// Install the Activity-backed Android context used by WaterKit.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_initializeAndroidContext<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    activity: JObject<'local>,
) -> jlong {
    super::with_env(&mut env, |env| {
        let java_vm = env
            .get_java_vm()
            .expect("Android context initialization failed to access JavaVM");
        let owner = Box::new(AndroidContextOwner {
            activity: env
                .new_global_ref(activity)
                .expect("Android context initialization failed to retain Activity"),
        });
        unsafe {
            ndk_context::initialize_android_context(
                java_vm.get_raw().cast(),
                owner.activity.as_obj().as_raw().cast(),
            );
        }
        Box::into_raw(owner) as jlong
    })
}

/// Release the Activity-backed Android context.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_releaseAndroidContext<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    owner: jlong,
) {
    assert_ne!(owner, 0, "Android context owner must not be null");
    unsafe {
        ndk_context::release_android_context();
        drop(Box::from_raw(owner as *mut AndroidContextOwner));
    }
}
