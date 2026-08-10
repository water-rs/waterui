use std::os::raw::c_int;

use objc2::MainThreadMarker;

unsafe extern "C" {
    fn waterui_cef_initialize_macos_application() -> c_int;
    fn waterui_cef_macos_application_is_active() -> c_int;
}

/// Installs the CEF-compatible macOS application object.
///
/// This must run on the main thread before any framework requests
/// `NSApplication.sharedApplication`.
///
/// # Panics
///
/// Panics when called off the main thread or after another `NSApplication`
/// class has already created the process application object.
pub fn initialize_macos_application() {
    assert!(
        MainThreadMarker::new().is_some(),
        "the CEF macOS application must be initialized on the main thread"
    );
    assert_ne!(
        // SAFETY: a C entry point in this crate's own helper library taking no
        // arguments and returning a status code.
        unsafe { waterui_cef_initialize_macos_application() },
        0,
        "CEF must install its NSApplication subclass before another framework creates NSApplication"
    );
}

pub fn assert_macos_application_active() {
    assert_ne!(
        // SAFETY: as above — no arguments, status code out.
        unsafe { waterui_cef_macos_application_is_active() },
        0,
        "CEF requires a CefAppProtocol NSApplication subclass before runtime initialization"
    );
}
