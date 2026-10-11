//! A numeric stepper control for incrementing or decrementing values.
//!
//! ![Stepper](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/stepper.svg)

use core::ops::{Bound, RangeBounds, RangeInclusive};

use crate::label::{IntoLabel, Label, LabelDisplayMode, impl_label_style_methods};
use alloc::rc::Rc;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_core::{Environment, configurable, layout::StretchAxis};
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
    /// A stepper with a visible label expands horizontally to fill the
    /// available width, placing the label at the leading edge and the buttons
    /// at the trailing edge with the free space between. A stepper with a
    /// hidden label is content-sized (just the buttons).
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::stepper;
    /// # fn basic(quantity: Binding<i32>) -> impl View {
    /// // Basic stepper: the label is required, so assistive technology always
    /// // has something to announce
    /// stepper("Quantity", &quantity)
    /// # }
    ///
    /// # fn bounded(count: Binding<i32>) -> impl View {
    /// // With a range and step
    /// stepper("Items", &count)
    ///     .range(1..=10)
    ///     .step(1)
    /// # }
    ///
    /// # fn row(quantity: Binding<i32>) -> impl View {
    /// // In a form row, where the row's own text carries the label visually
    /// hstack((
    ///     text("Quantity"),
    ///     spacer(),
    ///     stepper("Quantity", &quantity).hide_label(),
    /// ))
    /// # }
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // - Visible label: stretchAxis `.horizontal`; sizeThatFits answers the
    //   proposed width (never below the row's intrinsic width) and the
    //   intrinsic height; label leading, buttons trailing, flexible space
    //   between.
    // - Hidden label: stretchAxis `.none`; sizeThatFits answers the intrinsic
    //   size — the buttons alone.
    // - Read the axis from `NativeView::stretch_axis` on the payload; do not
    //   restate the rule in the backend.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Stepper,
    StepperConfig,
    |config| config.layout_stretch_axis(),
    resolve | config,
    env | config.resolve(env)
);

impl StepperConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        self
    }

    /// `docs/layout-spec.md` §3: a stepper is `Horizontal` when its label is
    /// visible, and `None` otherwise.
    ///
    /// Before resolution only the label's own display mode is known — an
    /// environment-wide [`LabelDisplayMode`] is not — so an unresolved label
    /// counts as visible; the resolved payload answers exactly.
    const fn layout_stretch_axis(&self) -> StretchAxis {
        if matches!(
            self.label.display_mode_preference(),
            LabelDisplayMode::Hidden
        ) {
            StretchAxis::None
        } else {
            StretchAxis::Horizontal
        }
    }
}

impl Stepper {
    /// Creates a new `Stepper` with the given semantic label and binding value.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(label: Label, value: &Binding<i32>) -> Self {
        Self(StepperConfig {
            value: value.clone(),
            step: 1i32.into_computed(),
            label,
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

    /// The binding this stepper reads and writes.
    ///
    /// Exposed for the same reason a text field exposes its own: a validator
    /// wraps the control by splicing a filtered binding in front of its value.
    pub const fn value_binding(&mut self) -> &mut Binding<i32> {
        &mut self.0.value
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
    Stepper::new(label.into_label(), value)
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::{Environment, NativeView, View};
    use waterui_locale::locales;

    use super::stepper;
    use crate::label::LabelDisplayMode;

    /// `docs/layout-spec.md` §3 over label visibility, both as the static
    /// answer a container reads before `body` and as the resolved payload a
    /// backend receives — an environment-wide hidden mode included.
    #[test]
    fn stretch_axis_follows_label_visibility() {
        let quantity = Binding::i32(0);
        let env = test_env();
        for (visible, expected) in [(true, StretchAxis::Horizontal), (false, StretchAxis::None)] {
            let make = || {
                let stepper = stepper("Quantity", &quantity);
                if visible {
                    stepper
                } else {
                    stepper.hide_label()
                }
            };
            assert_eq!(
                View::stretch_axis(&make()),
                expected,
                "label visible: {visible} (static)"
            );
            let resolved = make().0.resolve(&env);
            assert_eq!(
                NativeView::stretch_axis(&resolved),
                expected,
                "label visible: {visible} (resolved payload)"
            );
        }

        let mut env = test_env();
        env.insert(LabelDisplayMode::Hidden);
        let resolved = stepper("Quantity", &quantity).0.resolve(&env);
        assert_eq!(
            NativeView::stretch_axis(&resolved),
            StretchAxis::None,
            "an environment-hidden label stops the stretch"
        );
    }

    fn test_env() -> Environment {
        let mut env = Environment::new();
        env.insert(locales::EN);
        env
    }
}
