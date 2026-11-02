//! `apple_available!` expands to a runtime check on every Apple target.
//!
//! At a deployment target that already meets 27.0 the macro is always true and
//! warns at its call site, so this test exists only below it.

#![cfg(all(target_vendor = "apple", not(waterui_apple_os_met = "27.0")))]

use waterui_macros::apple_available;

#[test]
fn the_selection_asks_the_runtime_for_the_version_on_every_platform() {
    assert_eq!(
        apple_available!(27.0),
        waterui_apple_os::objc2::available!(macos = 27.0, ios = 27.0)
    );
}
