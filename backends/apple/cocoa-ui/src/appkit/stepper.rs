//! The `AppKit` stepper: an `NSStepper` driven in whole steps.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSStepper` subclass and calls `objc2`
//! bindings marked unsafe because `AppKit` control APIs are main-thread
//! only — which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::fmt;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAccessibility, NSControl, NSStepper};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObjectProtocol, NSString};

use crate::ActionTarget;
use crate::geometry::Size;

/// An `NSStepper` used as a value leaf.
pub struct StepperIvars {}

impl fmt::Debug for StepperIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StepperIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSStepper`'s designated initializer is `initWithFrame:`, which
    // `Stepper::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSStepper))]
    #[name = "CocoaUiStepper"]
    #[thread_kind = MainThreadOnly]
    #[ivars = StepperIvars]
    #[derive(Debug)]
    /// An `NSStepper` with an installable value-change action.
    pub struct Stepper;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSStepper` subclass.
    unsafe impl NSObjectProtocol for Stepper {}
);

impl Stepper {
    /// A stepper on the default range `0..=100` with a step of `1`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(StepperIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] }
    }

    /// The value the stepper currently reports.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.doubleValue()
    }

    /// Sets the value the stepper reports.
    pub fn set_value(&self, value: f64) {
        self.setDoubleValue(value);
    }

    /// Sets the inclusive range the value travels over.
    pub fn set_range(&self, start: f64, end: f64) {
        self.setMinValue(start);
        self.setMaxValue(end);
    }

    /// Sets the amount each press adds or subtracts.
    pub fn set_step(&self, step: f64) {
        self.setIncrement(step);
    }

    /// Whether the value wraps around at the range's ends.
    pub fn set_wraps(&self, wraps: bool) {
        self.setValueWraps(wraps);
    }

    /// Whether holding a button repeats the step.
    pub fn set_autorepeat(&self, autorepeat: bool) {
        self.setAutorepeat(autorepeat);
    }

    /// Whether the stepper responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// Calls `handler` with the new value each time the user steps. The
    /// returned target must be kept for as long as the control should
    /// respond; dropping it detaches the target.
    pub fn install_action(&self, handler: impl Fn(f64) + 'static) -> ActionTarget {
        let control: &NSControl = self;
        let this = Weak::new(self);
        ActionTarget::new(control, move |_mtm| {
            if let Some(stepper) = this.load() {
                handler(stepper.value());
            }
        })
    }

    /// The control's intrinsic size — what a measure pass reports.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.intrinsicContentSize();
        Size::new(size.width, size.height)
    }

    /// Names the control to a screen reader; `None` leaves it unnamed.
    /// Setting a label marks the view an accessibility element.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        self.setAccessibilityElement(label.is_some());
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
    }

    /// Speaks `value` as the control's accessibility value — the formatted
    /// or raw number a screen reader announces.
    pub fn set_accessibility_value(&self, value: &str) {
        // SAFETY: setting a string property on a live control.
        unsafe { self.setAccessibilityValue(Some(&NSString::from_str(value))) };
    }
}
