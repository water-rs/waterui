//! The `AppKit` disclosure control: a native `NSButton` with
//! [`NSBezelStyle::Disclosure`] and [`NSButtonType::PushOnPushOff`].
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSButton` subclass forwarding to `NSButton`'s
//! own implementation and calls `objc2` bindings marked unsafe because `AppKit`
//! views are main-thread only — which `MainThreadOnly` and [`MainThreadMarker`]
//! guarantee.

use keyboard_types::{Key, NamedKey};
use objc2::rc::{Allocated, Retained};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSBezelStyle, NSButton, NSButtonType, NSControl, NSControlStateValueOff,
    NSControlStateValueOn, NSEvent, NSView,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObjectProtocol, NSString, ns_string};

use crate::action::ActionTarget;
use crate::appkit::control::activate;
use crate::appkit::gesture::{GestureAttachment, click};
use crate::callback::guarded;
use crate::geometry::Size;
use crate::gesture::{ButtonMask, GestureState};
use crate::keys::key_event;

/// Ivars for [`DisclosureButton`].
#[derive(Default, Debug)]
pub struct DisclosureButtonIvars;

define_class!(
    // SAFETY: `NSButton`'s designated initializer is `initWithFrame:`, which
    // `DisclosureButton::new` invokes through `super`, and no `Drop` is defined.
    #[unsafe(super(NSButton))]
    #[name = "CocoaUiDisclosureButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DisclosureButtonIvars]
    #[derive(Debug)]
    /// An `NSButton` configured with `NSBezelStyle::Disclosure` and keyboard activation
    /// for both Space and Return/Enter.
    pub struct DisclosureButton;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSButton` subclass.
    unsafe impl NSObjectProtocol for DisclosureButton {}

    impl DisclosureButton {
        // SAFETY: designated initializer initializes ivars then chains to super.
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: CGRect) -> Retained<Self> {
            let this = this.set_ivars(DisclosureButtonIvars);
            // SAFETY: `NSButton::initWithFrame:` is the designated initializer.
            let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
            this.setButtonType(NSButtonType::PushOnPushOff);
            this.setBezelStyle(NSBezelStyle::Disclosure);
            this.setTitle(ns_string!(""));
            this
        }

        // SAFETY: `acceptsFirstResponder` answers a `bool` matching `isEnabled`.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            self.isEnabled()
        }

        // SAFETY: handles Space and Return/Enter by activating the disclosure button,
        // forwarding all other keys to `NSButton`.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let handled = guarded("DisclosureButton keyDown:", || {
                if !self.isEnabled() {
                    return false;
                }
                let key = key_event(event).key;
                let activate_key = match key {
                    Key::Named(NamedKey::Enter) => true,
                    Key::Character(ref ch) if ch == " " => true,
                    _ => false,
                };
                if activate_key {
                    // SAFETY: `self` is an enabled main-thread DisclosureButton handling keyDown.
                    unsafe { activate(self) };
                    return true;
                }
                false
            });
            if !handled {
                // SAFETY: forwarding unhandled key events preserves responder chain semantics.
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }
    }
);

impl DisclosureButton {
    /// Creates a new `AppKit` disclosure triangle button initialized to `expanded`.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, expanded: bool) -> Retained<Self> {
        // SAFETY: `initWithFrame:` is called on the main thread via `MainThreadMarker`.
        let button: Retained<Self> =
            unsafe { msg_send![Self::alloc(mtm), initWithFrame: CGRect::ZERO] };
        button.set_expanded(expanded);
        button
    }

    /// The underlying `NSControl` for registering [`ActionTarget`]s.
    #[must_use]
    pub fn control(&self) -> &NSControl {
        self
    }

    /// The underlying `NSView` for mounting and frame placement.
    #[must_use]
    pub fn as_view(&self) -> &NSView {
        self
    }

    /// The button's intrinsic `AppKit` layout size.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        self.intrinsicContentSize().into()
    }

    /// Whether the disclosure button is currently expanded.
    #[must_use]
    pub fn is_expanded(&self) -> bool {
        self.state() == NSControlStateValueOn
    }

    /// Sets the button's expanded visual and accessibility state.
    pub fn set_expanded(&self, expanded: bool) {
        let target = if expanded {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        if self.state() != target {
            self.setState(target);
        }
        self.setAccessibilityExpanded(expanded);
    }

    /// Enables or disables interaction.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// Sets the accessibility label announced for the disclosure control.
    pub fn set_accessibility_label(&self, label: &str) {
        self.setAccessibilityElement(true);
        self.setAccessibilityLabel(Some(&NSString::from_str(label)));
    }

    /// Triggers a click action (toggling the state and notifying targets).
    pub fn activate(&self) {
        // SAFETY: `self` is a main-thread DisclosureButton triggering its own control action.
        unsafe { activate(self) };
    }

    /// Registers a handler called whenever the user toggles the disclosure button.
    pub fn on_change(&self, handler: impl Fn(MainThreadMarker) + 'static) -> ActionTarget {
        ActionTarget::new(self.control(), handler)
    }
}

/// Attaches a click recognizer to `label_view` that activates `button` when clicked.
pub fn attach_label_click(
    label_view: &NSView,
    button: Retained<DisclosureButton>,
) -> GestureAttachment {
    click(label_view, 1, ButtonMask::PRIMARY, move |state, _| {
        if state == GestureState::Ended && button.isEnabled() {
            button.activate();
        }
    })
}
