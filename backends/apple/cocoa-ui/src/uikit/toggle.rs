//! The `UIKit` toggle: a `UISwitch` or a `UIButton` checkbox, together with
//! the labelled row a toggle lays out.
//!
//! `Toggle` owns a plain `UIView` container holding the control and an
//! optional label, arranged as the phone's toggle row — label at the leading
//! end, control at the trailing end — and carries the platform's animation
//! and action plumbing.
//!
//! # Safety
//!
//! The `unsafe` here constructs framework controls through their designated
//! initializers and convenience constructors, installs the container's
//! Auto Layout constraints, and wraps state changes in `UIView` transition
//! blocks — all documented main-thread APIs, which the [`MainThreadMarker`]
//! `new` requires guarantees.

use std::fmt;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, ns_string};
use objc2_ui_kit::{
    UIAccessibilityContentSizeCategoryImageAdjusting, UIButton, UIButtonType, UIControl,
    UIControlState, UIFontTextStyleBody, UIImage, UIImageSymbolConfiguration,
    UILayoutPriorityDefaultHigh, UILayoutPriorityRequired, UISwitch, UIView,
};

use crate::action::{ActionTarget, ControlEvents};
use crate::core_animation;
use crate::geometry::Size;

/// The gap between the label and the control, in points.
pub const LABEL_SPACING: f64 = 8.0;

/// Which control a [`Toggle`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `UISwitch`.
    Switch,
    /// A `UIButton` checkbox — `square` / `checkmark.square.fill` symbols.
    Checkbox,
}

/// How a state change reaches the control.
#[derive(Debug, Clone, Copy, Default)]
pub enum StateChange {
    /// Writes the new state directly.
    #[default]
    Immediate,
    /// A switch plays its slide; a checkbox writes directly — it has no
    /// slide to play.
    Animated,
    /// A checkbox cross-fades the new state over `seconds`; a switch plays
    /// its slide.
    Dissolve {
        /// The cross-fade duration.
        seconds: f64,
    },
}

#[derive(Clone)]
enum Control {
    Switch(Retained<UISwitch>),
    Checkbox(Retained<UIButton>),
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Switch(_) => f.write_str("Switch"),
            Self::Checkbox(_) => f.write_str("Checkbox"),
        }
    }
}

/// A labelled toggle row in a `UIView` container.
///
/// The container is what the leaf mounts and lays out; the control is what
/// user interaction and accessibility target.
#[derive(Debug, Clone)]
pub struct Toggle {
    container: Retained<UIView>,
    control: Control,
}

impl Toggle {
    /// A toggle row of `kind`, initially `is_on`, with no label yet —
    /// `set_label` installs one.
    ///
    /// # Panics
    ///
    /// When not called on the main thread, or when the system `square` /
    /// `checkmark.square.fill` symbols a checkbox needs are absent — a
    /// fatal authoring error.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, kind: Kind, is_on: bool) -> Self {
        let control = match kind {
            Kind::Switch => {
                let switch = UISwitch::new(mtm);
                switch.setOn(is_on);
                Control::Switch(switch)
            }
            Kind::Checkbox => {
                let unchecked = UIImage::systemImageNamed(ns_string!("square"));
                let checked = UIImage::systemImageNamed(ns_string!("checkmark.square.fill"));
                let (Some(unchecked), Some(checked)) = (unchecked, checked) else {
                    panic!("cocoa-ui checkbox requires the system square symbols");
                };
                let button = UIButton::buttonWithType(UIButtonType::System, mtm);
                // SAFETY: `UIFontTextStyleBody` is a documented, immutable
                // shared text style.
                let configuration = unsafe {
                    UIImageSymbolConfiguration::configurationWithTextStyle(UIFontTextStyleBody)
                };
                button.setPreferredSymbolConfiguration_forImageInState(
                    Some(&configuration),
                    UIControlState::Normal,
                );
                button.setImage_forState(Some(&unchecked), UIControlState::Normal);
                button.setImage_forState(Some(&checked), UIControlState::Selected);
                button.setSelected(is_on);
                button.setChangesSelectionAsPrimaryAction(true);
                button.setAdjustsImageSizeForAccessibilityContentSizeCategory(true);
                Control::Checkbox(button)
            }
        };
        Self {
            container: UIView::new(mtm),
            control,
        }
    }

    /// The container view — what a leaf mounts.
    #[must_use]
    pub fn container(&self) -> &UIView {
        &self.container
    }

    /// The control view — the target of user interaction, accessibility, and
    /// `set_enabled`.
    #[must_use]
    pub fn control(&self) -> &UIView {
        match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        }
    }

    /// Installs `label` as the container's leading subview: the label pinned
    /// leading, the control pinned trailing, both vertically centered.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_label(&self, label: &UIView) {
        let control = self.control();
        label.setTranslatesAutoresizingMaskIntoConstraints(false);
        control.setTranslatesAutoresizingMaskIntoConstraints(false);
        self.container.addSubview(label);
        self.container.addSubview(control);

        label.setContentCompressionResistancePriority_forAxis(
            UILayoutPriorityRequired,
            objc2_ui_kit::UILayoutConstraintAxis::Horizontal,
        );
        label.setContentHuggingPriority_forAxis(
            UILayoutPriorityDefaultHigh,
            objc2_ui_kit::UILayoutConstraintAxis::Horizontal,
        );

        let constraints = NSArray::from_slice(&[
            &*label
                .leadingAnchor()
                .constraintEqualToAnchor(&self.container.leadingAnchor()),
            &*label
                .centerYAnchor()
                .constraintEqualToAnchor(&self.container.centerYAnchor()),
            &*control
                .trailingAnchor()
                .constraintEqualToAnchor(&self.container.trailingAnchor()),
            &*control
                .centerYAnchor()
                .constraintEqualToAnchor(&self.container.centerYAnchor()),
            &*label
                .trailingAnchor()
                .constraintLessThanOrEqualToAnchor_constant(
                    &control.leadingAnchor(),
                    -LABEL_SPACING,
                ),
        ]);
        objc2_ui_kit::NSLayoutConstraint::activateConstraints(
            &constraints,
            MainThreadMarker::from(&*self.container),
        );
    }

    /// The control's intrinsic size — the row's share of a measure.
    #[must_use]
    pub fn control_size(&self) -> Size {
        self.control().intrinsicContentSize().into()
    }

    /// The row's composed size beside a label of `label_size`: label,
    /// `LABEL_SPACING`, control wide; the taller of the two high — the same
    /// arithmetic the `set_label` constraints place.
    #[must_use]
    pub fn row_size(&self, label_size: Size) -> Size {
        Size::new(
            label_size.width + LABEL_SPACING + self.control_size().width,
            label_size.height.max(self.control_size().height),
        )
    }

    /// Writes `on` into the control under `change`'s terms. A state already
    /// equal to `on` is left alone.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_on(&self, on: bool, change: StateChange) {
        match &self.control {
            Control::Switch(switch) => {
                if switch.isOn() == on {
                    return;
                }
                match change {
                    StateChange::Immediate => switch.setOn(on),
                    StateChange::Animated | StateChange::Dissolve { .. } => {
                        switch.setOn_animated(on, true);
                    }
                }
            }
            Control::Checkbox(button) => {
                if button.isSelected() == on {
                    return;
                }
                match change {
                    StateChange::Immediate | StateChange::Animated => button.setSelected(on),
                    StateChange::Dissolve { seconds } => {
                        let button = Retained::clone(button);
                        let view = Retained::clone(&button);
                        core_animation::cross_dissolve(&view, seconds, move || {
                            button.setSelected(on);
                        });
                    }
                }
            }
        }
    }

    /// The control's state after user interaction — what an action handler
    /// reports back.
    #[must_use]
    pub fn is_on(&self) -> bool {
        match &self.control {
            Control::Switch(switch) => switch.isOn(),
            Control::Checkbox(button) => button.isSelected(),
        }
    }

    /// Whether the control responds to input.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_enabled(&self, enabled: bool) {
        let control: &UIControl = match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        };
        control.setEnabled(enabled);
    }

    /// Calls `handler` each time the user toggles the control — `.valueChanged`
    /// on a switch, `.primaryActionTriggered` on a checkbox. The returned
    /// [`ActionTarget`] owns the registration — keep it for as long as the
    /// control should respond.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn on_change(&self, handler: impl Fn(MainThreadMarker) + 'static) -> ActionTarget {
        match &self.control {
            Control::Switch(switch) => {
                ActionTarget::new(switch, ControlEvents::VALUE_CHANGED, handler)
            }
            Control::Checkbox(button) => {
                ActionTarget::new(button, ControlEvents::PRIMARY_ACTION_TRIGGERED, handler)
            }
        }
    }
}
