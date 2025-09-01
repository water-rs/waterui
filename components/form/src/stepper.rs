use core::ops::{Bound, RangeBounds, RangeInclusive};

use nami::{Binding, Computed, signal::IntoComputed};
use waterui_core::{AnyView, View, configurable};

#[derive(Debug)]
#[non_exhaustive]
/// Configuration options for the [`Stepper`] component.
pub struct StepperConfig {
    /// The binding to the current value of the stepper.
    pub value: Binding<i32>,
    /// The step size for each increment or decrement.
    pub step: Computed<i32>,
    /// The label displayed alongside the stepper.
    pub label: AnyView,
    /// The valid range of values for the stepper.
    pub range: RangeInclusive<i32>,
}

configurable!(Stepper, StepperConfig);

impl Stepper {
    /// Creates a new `Stepper` with the given binding value.
    #[must_use]
    pub fn new(value: &Binding<i32>) -> Self {
        Self(StepperConfig {
            value: value.clone(),
            step: 1i32.into_computed(),
            label: AnyView::default(),
            range: i32::MIN..=i32::MAX,
        })
    }
    /// Sets the step size for the stepper.
    #[must_use]
    pub fn step(mut self, step: impl IntoComputed<i32>) -> Self {
        self.0.step = step.into_computed();
        self
    }
    /// Sets the label for the stepper.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }

    /// Sets the valid range of values for the stepper.
    #[must_use]
    pub fn range(mut self, range: impl RangeBounds<i32>) -> Self {
        let start = match range.start_bound() {
            Bound::Included(&s) => s,
            Bound::Excluded(&s) => s.saturating_add(1),
            Bound::Unbounded => i32::MIN,
        };
        let end = match range.end_bound() {
            Bound::Included(&e) => e,
            Bound::Excluded(&e) => e.saturating_sub(1),
            Bound::Unbounded => i32::MAX,
        };
        self.0.range = start..=end;
        self
    }
}

/// Creates a new Stepper with the given binding value.
///
/// See [`Stepper`] for more details.
#[must_use]
pub fn stepper(value: &Binding<i32>) -> Stepper {
    Stepper::new(value)
}
