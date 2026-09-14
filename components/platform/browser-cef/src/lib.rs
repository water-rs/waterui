//! Shared Chromium Embedded Framework runtime for `WaterUI`.
//!
//! The standard `waterui-webview` CEF backend and the independent
//! `waterui-chromium` component both use this crate. CEF is dynamically loaded
//! from the packaged application runtime and is never linked by applications
//! that do not select either surface.

mod app;
#[cfg(target_os = "macos")]
mod application_mac;
#[cfg(any(feature = "chromium", feature = "webview"))]
mod cdp;
#[cfg(any(feature = "chromium", feature = "webview"))]
mod gpu;
#[cfg(any(feature = "chromium", feature = "webview"))]
mod input;
#[cfg(any(feature = "chromium", feature = "webview"))]
mod install;
#[cfg(any(feature = "chromium", feature = "webview"))]
mod page;
mod runtime;
#[cfg(feature = "webview")]
mod webview;

#[cfg(target_os = "macos")]
pub use application_mac::initialize_macos_application;
#[cfg(any(feature = "chromium", feature = "webview"))]
pub use cdp::CefCdpSession;
#[cfg(any(feature = "chromium", feature = "webview"))]
pub use gpu::{gpu_view, gpu_view_with_input};
#[cfg(any(feature = "chromium", feature = "webview"))]
pub use input::CefSurfaceInput;
#[cfg(feature = "webview")]
pub use install::install;
#[cfg(feature = "chromium")]
pub use install::install_chromium;
#[cfg(any(feature = "chromium", feature = "webview"))]
pub use page::{
    AcceleratedFrameSink, CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton,
    CefPopupRect, CefTextRange,
};
#[cfg(target_os = "windows")]
pub use runtime::install_bootstrap_sandbox_info;
pub use runtime::{
    CefRuntime, CefRuntimeConfiguration, CefRuntimePaths, PumpDeadline, run_packaged_subprocess,
};
#[cfg(feature = "webview")]
pub use webview::CefWebViewHandle;

/// Exports the `RunWinMain` and `RunConsoleMain` entry points a Windows CEF
/// application DLL must provide for the bootstrap launcher.
///
/// CEF no longer ships a static `cef_sandbox` library for Windows. The
/// application executable is a renamed copy of `bootstrap.exe`
/// (`/SUBSYSTEM:WINDOWS`) or `bootstrapc.exe` (`/SUBSYSTEM:CONSOLE`) from the
/// distribution; it creates the OS-sandbox information object in-process,
/// loads the application DLL that shares its file name, and calls one of these
/// entry points with that object. The macro installs it through
/// [`install_bootstrap_sandbox_info`](crate::install_bootstrap_sandbox_info)
/// and then runs the real entry function:
///
/// ```ignore
/// waterui_browser_cef::cef_bootstrap_main!(main);
///
/// fn main() {
///     // Ordinary application entry point.
/// }
/// ```
///
/// Both symbols are exported so the same DLL works under either launcher.
/// Subprocesses re-enter through the same pair: `browser_subprocess_path`
/// resolves to the renamed launcher, which loads this DLL again with a
/// sandbox object restricted for that subprocess type.
#[cfg(target_os = "windows")]
#[macro_export]
macro_rules! cef_bootstrap_main {
    ($main:path) => {
        /// Called by `bootstrap.exe` (`/SUBSYSTEM:WINDOWS`). Do not invoke
        /// directly; the bootstrap launcher owns this entry point.
        #[unsafe(no_mangle)]
        pub extern "C" fn RunWinMain(
            _instance: *mut ::core::ffi::c_void,
            _command_line: *mut ::core::ffi::c_void,
            _show_command: ::core::ffi::c_int,
            sandbox_info: *mut ::core::ffi::c_void,
            _version_info: *mut ::core::ffi::c_void,
        ) -> ::core::ffi::c_int {
            $crate::install_bootstrap_sandbox_info(sandbox_info);
            $main();
            0
        }

        /// Called by `bootstrapc.exe` (`/SUBSYSTEM:CONSOLE`). Do not invoke
        /// directly; the bootstrap launcher owns this entry point.
        #[unsafe(no_mangle)]
        pub extern "C" fn RunConsoleMain(
            _argc: ::core::ffi::c_int,
            _argv: *mut *mut ::core::ffi::c_char,
            sandbox_info: *mut ::core::ffi::c_void,
            _version_info: *mut ::core::ffi::c_void,
        ) -> ::core::ffi::c_int {
            $crate::install_bootstrap_sandbox_info(sandbox_info);
            $main();
            0
        }
    };
}
