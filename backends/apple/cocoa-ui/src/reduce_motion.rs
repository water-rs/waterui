//! The user's system-wide Reduce Motion accessibility preference.

use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
#[cfg(target_os = "ios")]
use objc2_ui_kit::{
    UIAccessibilityIsReduceMotionEnabled, UIAccessibilityReduceMotionStatusDidChangeNotification,
};

use crate::notification::{self, NotificationName, NotificationObserver};

/// Whether the system's Reduce Motion preference is enabled.
#[must_use]
pub fn system_reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }
    #[cfg(target_os = "ios")]
    {
        UIAccessibilityIsReduceMotionEnabled()
    }
}

/// Calls `handler` on the main thread with the updated preference whenever the
/// system's Reduce Motion setting changes, until the returned guard is dropped.
pub fn observe_changes(
    mtm: MainThreadMarker,
    handler: impl Fn(bool) + 'static,
) -> NotificationObserver {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: reads a string constant `AppKit` owns for the process's
        // lifetime.
        let name = NotificationName::framework(unsafe {
            NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification
        });
        notification::observe_workspace(mtm, &name, move || handler(system_reduce_motion()))
    }
    #[cfg(target_os = "ios")]
    {
        // SAFETY: reads a string constant `UIKit` owns for the process's
        // lifetime.
        let name = NotificationName::framework(unsafe {
            UIAccessibilityReduceMotionStatusDidChangeNotification
        });
        notification::observe(mtm, &name, move || handler(system_reduce_motion()))
    }
}
