//! Platform [`AccessibilityAnnouncer`] installation for Apple backends.
//!
//! Routes announcements to `NSAccessibilityPostNotificationWithUserInfo` on
//! `AppKit` and `UIAccessibilityPostNotification` on `UIKit`, and queries
//! `VoiceOver` state for assistive-technology adaptations such as extended
//! snackbar timeouts.

use waterui_accessibility::AccessibilityAnnouncer;
use waterui_accessibility::LiveRegion;
use waterui_backend_core::Environment;

/// Installs the platform [`AccessibilityAnnouncer`] into `env`.
pub fn install(env: &mut Environment) {
    env.install(AccessibilityAnnouncer::with_assistive_technology_probe(
        |message, priority| {
            cocoa_ui::accessibility::announce(message, priority == LiveRegion::Assertive);
        },
        cocoa_ui::accessibility::is_voiceover_running,
    ));
}
