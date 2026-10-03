//! Native Android rendering, lifecycle, and embedding through JNI.
//!
//! Applications use [`export_app!`] to export their entry point; the
//! `dev.waterui.android` host library's `WaterActivity` loads the crate and
//! calls it. Component rendering and layout stay in Rust; the Kotlin host
//! only owns the activity, the root `ViewGroup`, and the lifecycle and
//! configuration callbacks it forwards.

extern crate alloc;

pub mod contract;
pub mod dispatch;
pub mod embedding;
pub mod entry;
pub(crate) mod jvm;
pub(crate) mod proposal;

pub(crate) mod components;
pub(crate) mod locale;
pub(crate) mod measure_memo;
mod native_layout;
mod registry;
pub(crate) mod startup;
pub(crate) mod theme;

#[cfg(target_os = "android")]
mod android_log;
#[cfg(target_os = "android")]
pub(crate) mod executor;

/// Generates the JNI entry point an application crate exports to the
/// `dev.waterui.android` host library.
///
/// The expansion emits `Java_dev_waterui_android_WaterRuntime_nativeCreate`:
/// `WaterActivity` calls it on the main thread after `System.loadLibrary`,
/// passing itself and the root `ViewGroup` it prepared. Everything the call
/// does — environment, executors, theme, dispatcher, mount — is owned by the
/// returned runtime handle.
#[macro_export]
macro_rules! export_app {
    ($app:path) => {
        /// Creates the application runtime inside `activity` with content
        /// mounted into `root`, answering the opaque handle the host keeps.
        ///
        /// Called once per process, on the main thread, by
        /// `dev.waterui.android.WaterActivity`.
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeCreate<'caller>(
            unowned_env: ::jni::EnvUnowned<'caller>,
            _this: ::jni::objects::JObject<'caller>,
            activity: ::jni::objects::JObject<'caller>,
            root: ::jni::objects::JObject<'caller>,
        ) -> ::jni::sys::jlong {
            let outcome = unowned_env.with_env(|env| -> ::jni::errors::Result<_> {
                let app = |env: ::waterui::Environment| {
                    $app(::waterui::configure_environment!(env))
                };
                $crate::entry::mount(env, activity, root, app)
            });
            outcome.resolve::<::jni::errors::ThrowRuntimeExAndDefault>()
        }
    };
}
