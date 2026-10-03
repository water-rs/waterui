//! Native AppKit/UIKit rendering, lifecycle, and embedding through objc2.
//!
//! Applications use [`export_app!`] for standalone and embedding entry points.
//! Component rendering and layout stay in Rust; the Swift package only owns
//! native host views and the opaque runtime and mount handles.

// The `with_env` feature name is fixed by the port contract.
#![allow(clippy::redundant_feature_names)]

extern crate alloc;

pub mod contract;
pub mod dispatch;
pub mod embedding;
pub mod entry;
mod native_layout;
mod native_log;
pub mod resources;

pub(crate) mod components;
pub(crate) mod first_paint;
pub(crate) mod fonts;
#[cfg(feature = "gpu_surface")]
mod gpu_completion;
#[cfg(feature = "gpu_surface")]
mod gpu_input;
mod gpu_runtime;
mod inspector;
mod invalidation;
pub(crate) mod locale;
pub(crate) mod measure_memo;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) mod menus;
pub(crate) mod primary_content;
pub(crate) mod proposal;
mod registry;
#[cfg(any(target_os = "ios", test))]
mod scene_registry;
pub(crate) mod startup;
pub(crate) mod theme;
#[cfg(target_os = "macos")]
mod toolbar;
pub(crate) mod windows;

/// Harness-only internals for `Tests/native.rs`: private `windows` and
/// `embedding` reach for the owned embedding contract. Never compiled
/// into a production build — gated behind `native-test-support`.
#[cfg(feature = "native-test-support")]
#[doc(hidden)]
#[path = "../Tests/native_support.rs"]
pub mod native_test_support;

/// Generates the `waterui_apple_main` entry point for the application
/// crate that calls it.
///
/// The whole launch — process startup, the environment, native services,
/// declared windows and the platform run loop — ends in
/// [`entry::run`], and the Xcode target's `main.swift` is a one-line call
/// into it.
#[macro_export]
macro_rules! export_app {
    ($app:path) => {
        /// Mounts the application in a native host using instance-owned resources.
        ///
        /// # Safety
        /// All pointers follow `embedding::mount`'s contract; call on the main thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn waterui_apple_mount(
            runtime: *const core::ffi::c_void,
            host: *mut core::ffi::c_void,
            assets: *const core::ffi::c_char,
            fonts: *const core::ffi::c_char,
        ) -> *mut core::ffi::c_void {
            // SAFETY: the embedding host supplies the documented runtime, host and paths.
            unsafe {
                $crate::embedding::mount(runtime, host, assets, fonts, |env| {
                    $app(::waterui::configure_environment!(env))
                })
            }
        }

        /// The application's entry: the generated `main.swift` calls this
        /// and nothing else.
        ///
        /// `accessory` selects the macOS activation policy
        /// (`NSApplication.ActivationPolicy.accessory`); it is unused on
        /// iOS.
        ///
        /// # Safety
        ///
        /// Call once, on the platform main thread, as the process entry.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn waterui_apple_main(accessory: bool) {
            let mut env = ::waterui::configure_environment!(::waterui::Environment::new());
            // SAFETY: this is the process's entry on the main thread, and
            // `env` lives in this frame — `run` never returns, so the
            // borrow outlives every native service and startup callback.
            unsafe {
                ::waterui_apple::entry::run($app, &mut env, accessory);
            }
        }
    };
}
