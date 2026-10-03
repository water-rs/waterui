//! `NSControl` helpers for chrome items forwarding activation.
//!
//! # Safety
//!
//! `activate` calls `performClick:`, which `AppKit` binds unsafe because the
//! call site is expected on the main thread — [`MainThreadMarker`]-gated
//! callers uphold that.

use objc2::rc::Retained;
use objc2_app_kit::{NSControl, NSView};

/// The first `NSControl` in `view`'s subtree, depth-first — the control a
/// chrome item forwards its activation to, as `firstButton` did for the
/// `WaterUI` bar item's action.
#[must_use]
pub fn first_control(view: &NSView) -> Option<Retained<NSControl>> {
    if let Some(control) = view.downcast_ref::<NSControl>() {
        return Some(Retained::from(control));
    }
    for subview in &view.subviews() {
        if let Some(control) = first_control(&subview) {
            return Some(control);
        }
    }
    None
}

/// The first [`Button`] in `view`'s subtree, depth-first.
#[must_use]
pub fn first_button(view: &NSView) -> Option<Retained<crate::appkit::Button>> {
    if let Some(button) = view.downcast_ref::<crate::appkit::Button>() {
        return Some(Retained::from(button));
    }
    for subview in &view.subviews() {
        if let Some(button) = first_button(&subview) {
            return Some(button);
        }
    }
    None
}

/// Activates `control` as if the user clicked it — `performClick:`.
///
/// # Safety
///
/// Call on the main thread only.
pub unsafe fn activate(control: &NSControl) {
    // SAFETY: the caller guarantees main-thread execution; the sender may
    // be nil — `performClick:` accepts it.
    unsafe { control.performClick(None) };
}
