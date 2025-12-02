//! A numeric stepper control for incrementing or decrementing values.

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

configurable!(
    /// A control for incrementing or decrementing a value.
    ///
    /// Stepper displays +/- buttons with an optional label. It's ideal for
    /// adjusting small numeric values like quantities.
    ///
    /// # Layout Behavior
    ///
    /// Stepper sizes itself to fit its label and buttons, and never stretches
    /// to fill extra space. In a stack, it takes only the space it needs.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Basic stepper
    /// stepper(&quantity)
    ///
    /// // With label and range
    /// stepper(&count)
    ///     .label("Items")
    ///     .range(1..=10)
    ///     .step(1)
    ///
    /// // In a form row
    /// hstack((
    ///     text("Quantity"),
    ///     spacer(),
    ///     stepper(&quantity),
    /// ))
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Stretch Axis: `None` - Stepper never expands to fill available space.
    // Size: label_width + spacing + stepper_buttons (platform-determined)
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Stepper,
    StepperConfig
);

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
