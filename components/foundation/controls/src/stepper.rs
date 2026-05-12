//! A numeric stepper control for incrementing or decrementing values.
//!
//! ![Stepper](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/stepper.svg)

use core::ops::{Bound, RangeBounds, RangeInclusive};

use crate::label::{IntoLabel, Label, impl_label_style_methods};
use alloc::rc::Rc;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_core::configurable;
use waterui_text::styled::StyledStr;

#[derive(Debug)]
#[non_exhaustive]
/// Configuration options for the [`Stepper`] component.
pub struct StepperConfig {
    /// The binding to the current value of the stepper.
    pub value: Binding<i32>,
    /// The step size for each increment or decrement.
    pub step: Computed<i32>,
    /// The label displayed alongside the stepper.
    pub label: Label,
    /// Optional formatter for the inline value display, layered on top of the
    /// label. When `None`, the stepper renders only the label; when `Some`, the
    /// formatted value is shown next to the buttons.
    pub value_formatter: Option<Computed<StyledStr>>,
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
    /// With a label: Stepper expands horizontally to fill available space,
    /// placing the label on the left and buttons on the right.
    /// Without a label: Stepper is content-sized (just buttons).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Basic stepper (has default label showing value)
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
    // - stretchAxis: .horizontal (stepper always has a label by default)
    // - sizeThatFits: Returns proposed width (or minimum), intrinsic height
    // - Layout: label on left, buttons on right, flexible space between
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Stepper,
    StepperConfig,
    waterui_core::layout::StretchAxis::Horizontal
);

impl Stepper {
    /// Creates a new `Stepper` with the given semantic label and binding value.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(label: impl IntoLabel, value: &Binding<i32>) -> Self {
        Self(StepperConfig {
            value: value.clone(),
            step: 1i32.into_computed(),
            label: label.into_label(),
            value_formatter: None,
            range: i32::MIN..=i32::MAX,
        })
    }
    /// Sets the step size for the stepper.
    #[must_use]
    pub fn step(mut self, step: impl IntoComputed<i32>) -> Self {
        self.0.step = step.into_computed();
        self
    }

    /// Sets a formatter for the inline value display. The semantic label is
    /// unaffected.
    #[must_use]
    pub fn value_formatter<T: Into<StyledStr>>(
        mut self,
        formatter: impl 'static + Fn(i32) -> T,
    ) -> Self {
        let formatter = Rc::new(formatter);
        self.0.value_formatter = Some(
            self.0
                .value
                .clone()
                .map(move |value| formatter(value).into())
                .computed(),
        );
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

impl_label_style_methods!(Stepper);

/// Creates a new Stepper with the given label and binding value.
///
/// See [`Stepper`] for more details.
#[must_use]
pub fn stepper(label: impl IntoLabel, value: &Binding<i32>) -> Stepper {
    Stepper::new(label, value)
}
