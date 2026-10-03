//! Native Android rendering, lifecycle, and embedding through JNI.
//!
//! Applications use [`export_app!`] to export their entry point; the
//! `dev.waterui.android` host library's `WaterActivity` loads the crate and
//! calls it. Component rendering and layout stay in Rust; the Kotlin host
//! only owns the activity, the root `ViewGroup`, and the lifecycle and
//! configuration callbacks it forwards.

// The runtime path — `entry`, `embedding`, `startup`, `theme`, `locale`,
// and the host-side `jvm` glue — is reached only through `export_app!`'s
// exported `nativeCreate`, which the *application's* crate emits. Inside
// `waterui-android` itself nothing references it, so a host build reports
// the whole device path as dead code while still type-checking it. The
// leaf-facing surface (`native_layout`, `proposal`, the `jvm` unit and
// reference helpers) exists for component ports and is likewise unclaimed
// when every component feature is off.
#![cfg_attr(
    any(
        not(target_os = "android"),
        not(any(feature = "text", feature = "container", feature = "button"))
    ),
    allow(
        dead_code,
        reason = "the JNI runtime path is only reachable through the app's exported entry, and the leaf API only through component ports"
    )
)]

extern crate alloc;

// `export_app!` expands in the *application's* crate, which carries no
// `jni` dependency — the expansion spells these paths instead. Hidden:
// not part of the API, only the macro's private reach-in.
#[doc(hidden)]
pub use jni as __jni;

pub mod contract;
pub mod dispatch;
pub mod embedding;
pub mod entry;
pub(crate) mod handle;
pub(crate) mod jvm;
pub mod policy;
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
            mut unowned_env: $crate::__jni::EnvUnowned<'caller>,
            _this: $crate::__jni::objects::JObject<'caller>,
            activity: $crate::__jni::objects::JObject<'caller>,
            root: $crate::__jni::objects::JObject<'caller>,
        ) -> $crate::__jni::sys::jlong {
            let outcome = unowned_env.with_env(|env| -> $crate::__jni::errors::Result<_> {
                let app =
                    |env: ::waterui::Environment| $app(::waterui::configure_environment!(env));
                $crate::entry::mount(env, activity, root, app)
            });
            outcome.resolve::<$crate::policy::ThrowRuntimeExAndDefault>()
        }
    };
}
