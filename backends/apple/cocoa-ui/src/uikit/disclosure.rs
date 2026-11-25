//! The `UIKit` disclosure control: a `UIButton` displaying the platform
//! chevron indicator (`chevron.right` / `chevron.down`) with keyboard activation
//! and expanded accessibility state.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIButton` subclass and calls `objc2` bindings
//! marked unsafe because `UIKit` views are main-thread only — which
//! `MainThreadOnly` and [`MainThreadMarker`] guarantee.

use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSObjectProtocol, NSString, ns_string};
use objc2_ui_kit::{
    NSObjectUIAccessibility, UIAccessibilityContentSizeCategoryImageAdjusting,
    UIAccessibilityTraitButton, UIButton, UIButtonType, UIColor, UIControl, UIControlEvents,
    UIControlState, UIFontTextStyleBody, UIImage, UIImageRenderingMode, UIImageSymbolConfiguration,
    UIKeyCommand, UIKeyModifierFlags, UIView,
};

use crate::action::{ActionTarget, ControlEvents};
use crate::callback::guarded;
use crate::geometry::Size;
use crate::gesture::{ButtonMask, GestureState};
use crate::uikit::gesture::{GestureAttachment, tap};

/// Ivars for [`DisclosureButton`].
#[derive(Default, Debug)]
pub struct DisclosureButtonIvars;

define_class!(
    // SAFETY: `UIButton`'s designated initializer is `initWithFrame:` /
    // `+buttonWithType:`, and no `Drop` is defined.
    #[unsafe(super(UIButton))]
    #[name = "CocoaUiDisclosureButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DisclosureButtonIvars]
    #[derive(Debug)]
    /// A `UIButton` displaying the disclosure chevron with Space/Return key commands.
    pub struct DisclosureButton;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIButton` subclass.
    unsafe impl NSObjectProtocol for DisclosureButton {}

    impl DisclosureButton {
        // SAFETY: `canBecomeFocused` returns whether focus can land on the button.
        #[unsafe(method(canBecomeFocused))]
        fn can_become_focused(&self) -> bool {
            self.isEnabled()
        }

        // SAFETY: returns Space and Return key commands bound to `cocoaUiDisclosureActivate:`.
        #[unsafe(method_id(keyCommands))]
        fn key_commands(&self) -> Option<Retained<NSArray<UIKeyCommand>>> {
            let mtm = self.mtm();
            let space = NSString::from_str(" ");
            let ret = NSString::from_str("\r");
            // SAFETY: binds valid selectors implemented on this class.
            let commands = unsafe {
                [
                    UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                        &space,
                        UIKeyModifierFlags(0),
                        sel!(cocoaUiDisclosureActivate:),
                        mtm,
                    ),
                    UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                        &ret,
                        UIKeyModifierFlags(0),
                        sel!(cocoaUiDisclosureActivate:),
                        mtm,
                    ),
                ]
            };
            Some(NSArray::from_retained_slice(&commands))
        }

        // SAFETY: handles keyboard activation for Space and Return.
        #[unsafe(method(cocoaUiDisclosureActivate:))]
        fn disclosure_activate(&self, _sender: Option<&UIKeyCommand>) {
            guarded("DisclosureButton cocoaUiDisclosureActivate:", || {
                if self.isEnabled() {
                    self.activate();
                }
            });
        }
    }
);

impl DisclosureButton {
    /// Creates a new `UIKit` disclosure chevron button initialized to `expanded`.
    ///
    /// # Panics
    ///
    /// When not called on the main thread, or if standard SF Symbols `chevron.right` /
    /// `chevron.down` are unavailable.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, expanded: bool) -> Retained<Self> {
        let _ = mtm;
        // SAFETY: invoked on the `DisclosureButton` class on the main thread.
        let button: Retained<Self> =
            unsafe { msg_send![Self::class(), buttonWithType: UIButtonType::Custom] };
        let collapsed_img = UIImage::systemImageNamed(ns_string!("chevron.right"))
            .map(|img| img.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate));
        let expanded_img = UIImage::systemImageNamed(ns_string!("chevron.down"))
            .map(|img| img.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate));
        let (Some(collapsed_img), Some(expanded_img)) = (collapsed_img, expanded_img) else {
            panic!("SF Symbols ships `chevron.right` and `chevron.down` on every supported iOS");
        };
        // SAFETY: `UIFontTextStyleBody` is an immutable UIKit shared text style.
        let configuration =
            unsafe { UIImageSymbolConfiguration::configurationWithTextStyle(UIFontTextStyleBody) };
        button.setPreferredSymbolConfiguration_forImageInState(
            Some(&configuration),
            UIControlState::Normal,
        );
        button.setPreferredSymbolConfiguration_forImageInState(
            Some(&configuration),
            UIControlState::Selected,
        );
        button.setImage_forState(Some(&collapsed_img), UIControlState::Normal);
        button.setImage_forState(Some(&expanded_img), UIControlState::Selected);
        button.setChangesSelectionAsPrimaryAction(true);
        button.setAdjustsImageSizeForAccessibilityContentSizeCategory(true);
        // SAFETY: `UIAccessibilityTraitButton` is a published UIKit constant.
        let button_trait = unsafe { UIAccessibilityTraitButton };
        button.setAccessibilityTraits(button.accessibilityTraits(mtm) | button_trait, mtm);
        button.set_expanded(expanded);
        button
    }

    /// The underlying `UIControl`.
    #[must_use]
    pub fn control(&self) -> &UIControl {
        self
    }

    /// The underlying `UIView`.
    #[must_use]
    pub fn as_view(&self) -> &UIView {
        self
    }

    /// Intrinsic size of the disclosure indicator button.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        self.intrinsicContentSize().into()
    }

    /// Whether the disclosure indicator is currently expanded.
    #[must_use]
    pub fn is_expanded(&self) -> bool {
        self.isSelected()
    }

    /// Sets the expanded state and accessibility value.
    pub fn set_expanded(&self, expanded: bool) {
        let mtm = self.mtm();
        if self.isSelected() != expanded {
            self.setSelected(expanded);
        }
        let state = if expanded { "Expanded" } else { "Collapsed" };
        self.setAccessibilityValue(Some(&NSString::from_str(state)), mtm);
    }

    /// Sets whether the control responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// Sets the chevron template tint colour.
    pub fn set_tint(&self, color: &UIColor) {
        // SAFETY: `setTintColor:` is called on the main thread.
        unsafe { self.setTintColor(Some(color)) };
    }

    /// Sets the accessibility label announced for the disclosure control.
    pub fn set_accessibility_label(&self, label: &str) {
        let mtm = self.mtm();
        self.setIsAccessibilityElement(true, mtm);
        self.setAccessibilityLabel(Some(&NSString::from_str(label)), mtm);
    }

    /// Toggles the expanded state and dispatches `PrimaryActionTriggered`.
    pub fn activate(&self) {
        let next = !self.isSelected();
        self.set_expanded(next);
        self.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
    }

    /// Registers a handler called whenever the user toggles the disclosure button.
    pub fn on_change(&self, handler: impl Fn(MainThreadMarker) + 'static) -> ActionTarget {
        ActionTarget::new(
            self.control(),
            ControlEvents::PRIMARY_ACTION_TRIGGERED,
            handler,
        )
    }
}

/// Attaches a tap recognizer to `label_view` that toggles `button` when tapped.
pub fn attach_label_click(
    label_view: &UIView,
    button: Retained<DisclosureButton>,
) -> GestureAttachment {
    tap(label_view, 1, ButtonMask::PRIMARY, move |state, _| {
        if state == GestureState::Ended && button.isEnabled() {
            button.activate();
        }
    })
}
