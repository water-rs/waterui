//! The `AppKit` toggle: an `NSSwitch` or an `NSButton` checkbox, together
//! with the labelled row a toggle lays out.
//!
//! `Toggle` owns a plain `NSView` container holding the control and an
//! optional label, arranged the way a Mac reads them — control first, then
//! its title — and carries the platform's animation and action plumbing.
//!
//! # Safety
//!
//! The `unsafe` here constructs framework controls through their designated
//! initializers and convenience constructors, installs the container's
//! Auto Layout constraints, and runs `NSAnimationContext` groups — all
//! documented main-thread APIs, which the [`MainThreadMarker`] `new`
//! requires guarantees.

use std::fmt;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAnimationContext, NSButton, NSControl, NSControlStateValueOff, NSControlStateValueOn,
    NSLayoutConstraint, NSLayoutConstraintOrientation, NSLayoutPriorityDefaultHigh,
    NSLayoutPriorityRequired, NSSwitch, NSView,
};
use objc2_foundation::{NSArray, ns_string};
use objc2_quartz_core::CAMediaTimingFunction;

use crate::action::ActionTarget;
use crate::geometry::Size;

/// The gap between the control and its label, in points.
pub const LABEL_SPACING: f64 = 8.0;

/// Which control a [`Toggle`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An `NSSwitch`.
    Switch,
    /// An `NSButton` checkbox.
    Checkbox,
}

/// The `NSAnimationContext` parameters a state change plays under — the
/// `withPlatformAnimation` table.
#[derive(Debug, Clone, Copy, Default)]
pub enum StateChange {
    /// Writes the new state directly.
    #[default]
    Immediate,
    /// Runs the write inside an implicit-animation group of `seconds`;
    /// `control_points` installs a cubic-bezier timing function when present.
    Implicit {
        /// The group's duration.
        seconds: f64,
        /// Cubic-bezier timing control points `(x1, y1, x2, y2)`.
        control_points: Option<(f32, f32, f32, f32)>,
    },
}

#[derive(Clone)]
enum Control {
    Switch(Retained<NSSwitch>),
    Checkbox(Retained<NSButton>),
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Switch(_) => f.write_str("Switch"),
            Self::Checkbox(_) => f.write_str("Checkbox"),
        }
    }
}

/// A labelled toggle row in an `NSView` container.
///
/// The container is what the leaf mounts and lays out; the control is what
/// user interaction and accessibility target.
#[derive(Debug, Clone)]
pub struct Toggle {
    container: Retained<NSView>,
    control: Control,
}

impl Toggle {
    /// A toggle row of `kind`, initially `is_on`, with no label yet —
    /// `set_label` installs one.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, kind: Kind, is_on: bool) -> Self {
        let state = if is_on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        let control = match kind {
            Kind::Switch => {
                let switch = NSSwitch::new(mtm);
                switch.setState(state);
                Control::Switch(switch)
            }
            Kind::Checkbox => {
                // SAFETY: the convenience constructor yields a configured
                // checkbox; `nil` target/action leave it unarmed until
                // `on_change` installs one.
                let button = unsafe {
                    NSButton::checkboxWithTitle_target_action(ns_string!(""), None, None, mtm)
                };
                button.setState(state);
                Control::Checkbox(button)
            }
        };
        Self {
            container: NSView::new(mtm),
            control,
        }
    }

    /// The container view — what a leaf mounts.
    #[must_use]
    pub fn container(&self) -> &NSView {
        &self.container
    }

    /// The control view — the target of user interaction, accessibility, and
    /// `set_enabled`.
    #[must_use]
    pub fn control(&self) -> &NSView {
        match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        }
    }

    /// Installs `label` as the container's second subview: the control pinned
    /// leading, the label after it, both vertically centered.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_label(&self, label: &NSView) {
        let control = self.control();
        label.setTranslatesAutoresizingMaskIntoConstraints(false);
        control.setTranslatesAutoresizingMaskIntoConstraints(false);
        self.container.addSubview(label);
        self.container.addSubview(control);

        label.setContentCompressionResistancePriority_forOrientation(
            NSLayoutPriorityRequired,
            NSLayoutConstraintOrientation::Horizontal,
        );
        label.setContentHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultHigh,
            NSLayoutConstraintOrientation::Horizontal,
        );

        let constraints = NSArray::from_slice(&[
            &*control
                .leadingAnchor()
                .constraintEqualToAnchor(&self.container.leadingAnchor()),
            &*control
                .centerYAnchor()
                .constraintEqualToAnchor(&self.container.centerYAnchor()),
            &*label.leadingAnchor().constraintEqualToAnchor_constant(
                &self.container.leadingAnchor(),
                self.label_leading(),
            ),
            &*label
                .centerYAnchor()
                .constraintEqualToAnchor(&self.container.centerYAnchor()),
            &*label
                .trailingAnchor()
                .constraintLessThanOrEqualToAnchor(&self.container.trailingAnchor()),
        ]);
        NSLayoutConstraint::activateConstraints(&constraints);
    }

    /// The control's intrinsic size — the row's share of a measure.
    #[must_use]
    pub fn control_size(&self) -> Size {
        self.control().intrinsicContentSize().into()
    }

    /// The label's leading offset inside the row: the control's intrinsic
    /// width plus `LABEL_SPACING`. `set_label` pins the label there, and a
    /// measure of the row reads the same number — the row's composition
    /// lives in one place.
    #[must_use]
    pub fn label_leading(&self) -> f64 {
        self.control_size().width + LABEL_SPACING
    }

    /// The row's composed size beside a label of `label_size`: control,
    /// `LABEL_SPACING`, label wide; the taller of the two high — the same
    /// arithmetic the `set_label` constraints place.
    #[must_use]
    pub fn row_size(&self, label_size: Size) -> Size {
        Size::new(
            self.label_leading() + label_size.width,
            self.control_size().height.max(label_size.height),
        )
    }

    /// Writes `on` into the control, under `change`'s animation terms.
    /// A state already equal to `on` is left alone.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_on(&self, on: bool, change: StateChange) {
        let state = if on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        match &self.control {
            Control::Switch(control) if control.state() == state => return,
            Control::Checkbox(control) if control.state() == state => return,
            _ => {}
        }
        match change {
            StateChange::Immediate => set_state(&self.control, state),
            StateChange::Implicit {
                seconds,
                control_points,
            } => {
                let control = self.control.clone();
                let changes =
                    RcBlock::new(move |context: core::ptr::NonNull<NSAnimationContext>| {
                        // SAFETY: the context is the live group AppKit hands
                        // the block.
                        let context = unsafe { context.as_ref() };
                        context.setDuration(seconds);
                        context.setAllowsImplicitAnimation(true);
                        if let Some((x1, y1, x2, y2)) = control_points {
                            let timing =
                                CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2);
                            context.setTimingFunction(Some(&timing));
                        }
                        set_state(&control, state);
                    });
                NSAnimationContext::runAnimationGroup(&changes);
            }
        }
    }

    /// The control's state after user interaction — what an action handler
    /// reports back.
    #[must_use]
    pub fn is_on(&self) -> bool {
        match &self.control {
            Control::Switch(control) => control.state() == NSControlStateValueOn,
            Control::Checkbox(control) => control.state() == NSControlStateValueOn,
        }
    }

    /// Whether the control responds to input.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn set_enabled(&self, enabled: bool) {
        let control: &NSControl = match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        };
        control.setEnabled(enabled);
    }

    /// Calls `handler` each time the user toggles the control. The returned
    /// [`ActionTarget`] owns the registration — keep it for as long as the
    /// control should respond.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    pub fn on_change(&self, handler: impl Fn(MainThreadMarker) + 'static) -> ActionTarget {
        let control: &NSControl = match &self.control {
            Control::Switch(control) => control,
            Control::Checkbox(control) => control,
        };
        ActionTarget::new(control, handler)
    }
}

/// Writes `state` — the property change an implicit-animation group picks
/// up.
fn set_state(control: &Control, state: objc2_app_kit::NSControlStateValue) {
    match control {
        Control::Switch(control) => control.setState(state),
        Control::Checkbox(control) => control.setState(state),
    }
}
