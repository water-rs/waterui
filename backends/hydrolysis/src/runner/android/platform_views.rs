//! Publishing embedded platform-view placements to the Kotlin
//! `PlatformViewRegistry`.
//!
//! `PlatformView` leaves record their window-space frames onto the session's
//! [`PlatformViewSink`] during each flush (see
//! [`crate::platform_view`]); this module promotes that table once per
//! rendered frame and notifies the host — placements arrive as one JSON
//! frame set per change, addressed by stable view id, so the registry never
//! merges per-view deltas itself.
//!
//! Coordinates cross the JNI edge in the window's logical units; the Kotlin
//! side scales to physical px with the same density the input pipeline
//! divides by — one unit space on every edge keeps the placements, hit
//! targets, IME rects and accessibility bounds coherent.

use super::host::AndroidSession;
use super::jni::JniError;

/// Serializes the published placement set for `nativePlatformViewFrames`.
/// Always serializes the current set — the Kotlin registry only reads after
/// a change notification.
pub fn placements_json(session: &AndroidSession) -> Result<String, JniError> {
    let table = session.platform_views.table().borrow();
    serde_json::to_string(table.placements()).map_err(|error| {
        JniError(format!(
            "hydrolysis android: platform-view placements failed to serialize: {error}"
        ))
    })
}

/// Ends the frame transaction for platform views: promote the set the flush
/// recorded and notify the host when it changed. Called once per frame with
/// `rendered` reporting whether a flush actually ran — an idle frame must
/// not publish, or every platform view would drop (its leaf records only
/// while encoding).
pub fn publish_if_pending(session: &AndroidSession, rendered: bool) {
    if !rendered {
        return;
    }
    let changed = {
        let mut table = session.platform_views.table().borrow_mut();
        table.publish();
        table.take_dirty()
    };
    if changed {
        session.runtime.platform.bridge.platform_views_changed();
    }
}
