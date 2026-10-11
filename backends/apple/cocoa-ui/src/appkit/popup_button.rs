//! The `AppKit` pull-down button: an `NSPopUpButton` that shows a menu.
//!
//! `pullsDown` is the Mac menu-button presentation — the label describes
//! the action rather than the current value. The button's `Menu` items are
//! rebuilt on `set_menu`; the label view is the owner's content, laid out by
//! the owner at `label_offer`.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSPopUpButton` subclass and calls `objc2`/
//! `AppKit` bindings marked unsafe because `AppKit` control APIs are
//! main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSPopUpButton, NSView};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;

/// The pull-down button's state.
pub struct PopUpButtonIvars {}

impl fmt::Debug for PopUpButtonIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PopUpButtonIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSPopUpButton`'s designated initializer is
    // `initWithFrame:pullsDown:`, which `PopUpButton::new` calls, and the
    // class does not implement `Drop`.
    #[unsafe(super(NSPopUpButton))]
    #[name = "CocoaUiPopUpButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PopUpButtonIvars]
    #[derive(Debug)]
    /// A pull-down `NSPopUpButton` with a hosted label view.
    pub struct PopUpButton;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSPopUpButton`.
    unsafe impl NSObjectProtocol for PopUpButton {}
);

impl PopUpButton {
    /// A pull-down button — its title describes the action, not a value.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PopUpButtonIvars {});
        // SAFETY: `initWithFrame:pullsDown:` is `NSPopUpButton`'s designated
        // initializer; `pullsDown` is the menu-button presentation.
        unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO, pullsDown: true] }
    }

    /// Replaces the button's menu.
    pub fn set_menu_items(&self, menu: &objc2_app_kit::NSMenu) {
        self.setMenu(Some(menu));
    }

    /// Hosts `label` inside the button's face; the owner lays it out within
    /// the button's bounds, reserving the pull-down arrow's trailing width.
    pub fn set_label_view(&self, label: &NSView) {
        self.addSubview(label);
    }

    /// Clears the button's title so only the hosted label shows.
    pub fn clear_title(&self) {
        self.setTitle(&objc2_foundation::NSString::new());
    }
}
