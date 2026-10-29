//! Native AppKit/UIKit rendering, lifecycle, and embedding through objc2.
//!
//! Applications use [`export_app!`] for standalone and embedding entry points.
//! Component rendering and layout stay in Rust; the Swift package only owns
//! native host views and the opaque runtime and mount handles.

extern crate alloc;

/// The offscreen capture shared by `view_renderer` and the `preview`
/// entry — nothing else references it. With `gpu_surface` the captures go
/// through `capture_image`'s `ViewCapture` instead, so the
/// bitmap-readback half only exists without it.
#[cfg(all(
    not(feature = "gpu_surface"),
    any(feature = "view_renderer", preview_entry)
))]
pub(crate) mod capture;
/// Awaiting the content a captured subtree owes — shared by the same two
/// captures.
#[cfg(any(feature = "view_renderer", preview_entry))]
pub(crate) mod content;
pub mod contract;
pub mod dispatch;
pub mod embedding;
pub mod entry;
mod native_layout;
mod native_log;
#[cfg(preview_entry)]
pub mod preview;
pub mod resources;

mod animation;
#[cfg(feature = "gpu_surface")]
pub(crate) mod capture_image;
pub(crate) mod components;
pub(crate) mod first_paint;
pub(crate) mod fonts;
#[cfg(feature = "gpu_surface")]
mod frozen_frame;
#[cfg(feature = "gpu_surface")]
mod gpu_completion;
#[cfg(feature = "gpu_surface")]
mod gpu_input;
#[cfg(all(feature = "gpu_surface", target_os = "ios"))]
mod gpu_text;
mod gpu_runtime;
mod inspector;
mod invalidation;
pub(crate) mod locale;
#[cfg(any(
    feature = "gpu_surface",
    feature = "applied_filter",
    feature = "view_effect",
    feature = "video",
    feature = "video_player"
))]
pub(crate) mod main_queue_owned;
pub(crate) mod measure_memo;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) mod menus;
#[cfg(feature = "gpu_surface")]
mod presentation_time;
pub(crate) mod primary_content;
pub(crate) mod proposal;
#[cfg(feature = "gpu_surface")]
mod publication_park;
mod registry;
#[cfg(any(target_os = "ios", test))]
mod scene_registry;
pub(crate) mod startup;
#[cfg(feature = "swiftui")]
mod swiftui_meta;
#[cfg(target_os = "macos")]
mod termination;
pub(crate) mod theme;
#[cfg(target_os = "macos")]
mod toolbar;
pub(crate) mod transition;
pub(crate) mod window_background;
pub(crate) mod windows;

/// Harness-only internals for `Tests/native.rs`: private `windows` and
/// `embedding` reach for the owned embedding contract. Never compiled
/// into a production build — gated behind `native-test`.
#[cfg(feature = "native-test")]
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
            dark: bool,
            rtl: bool,
            locale: *const core::ffi::c_char,
            host_fonts: *const f64,
        ) -> *mut core::ffi::c_void {
            // SAFETY: the embedding host supplies the documented runtime, host,
            // paths, and — when `host_fonts` is non-null — a live locale
            // string and eighteen readable `f64`s.
            let host_environment = (!host_fonts.is_null()).then(|| unsafe {
                $crate::embedding::HostEnvironment::from_abi(dark, rtl, locale, host_fonts)
            });
            unsafe {
                $crate::embedding::mount(runtime, host, assets, fonts, host_environment, |env| {
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
