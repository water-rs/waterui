//! `apple_available!` expands to a runtime check on every Apple target.

#![cfg(target_vendor = "apple")]

use waterui_macros::apple_available;

#[test]
fn the_selection_asks_the_runtime_for_the_version_on_every_platform() {
    // At a deployment target below 27.0 the answer is the host's OS version;
    // at 27.0 or above the macro warns that it is always true, and the
    // workspace's `-D warnings` build would have refused this test.
    assert_eq!(
        apple_available!(27.0),
        waterui_apple_os::objc2::available!(macos = 27.0, ios = 27.0)
    );
}
