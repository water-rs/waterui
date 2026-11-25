//! A numeric stepper control for incrementing or decrementing values.
//!
//! ![Stepper](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/stepper.svg)

use core::ops::{Bound, RangeBounds, RangeInclusive};

use crate::label::{
    ControlLabel, IntoLabel, Label, LabelDisplayMode, LabelKind, impl_label_style_methods,
};
use alloc::rc::Rc;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_text::styled::StyledStr;

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Configuration options for the [`Stepper`] component.
pub struct StepperConfig<S> {
    /// The binding to the current value of the stepper.
    value: Binding<i32>,
    /// The step size for each increment or decrement.
    step: Computed<i32>,
    /// The label displayed alongside the stepper.
    label: ControlLabel<S>,
    /// Optional formatter for the inline value display, layered on top of the
    /// label. When `None`, the stepper renders only the label; when `Some`, the
    /// formatted value is shown next to the buttons.
    value_formatter: Option<Computed<StyledStr>>,
    /// The valid range of values for the stepper.
    range: RangeInclusive<i32>,
}

impl<S> StepperConfig<S> {
    /// The binding to the current value of the stepper.
    #[must_use]
    pub const fn value(&self) -> &Binding<i32> {
        &self.value
    }

    /// The step size for each increment or decrement.
    #[must_use]
    pub const fn step(&self) -> &Computed<i32> {
        &self.step
    }

    /// The label displayed alongside the stepper.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: ControlLabel<S>) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut ControlLabel<S> {
        &mut self.label
    }

    /// Optional formatter for the inline value display, layered on top of the
    /// label. When `None`, the stepper renders only the label; when `Some`, the
    /// formatted value is shown next to the buttons.
    #[must_use]
    pub const fn value_formatter(&self) -> Option<&Computed<StyledStr>> {
        self.value_formatter.as_ref()
    }

    /// The valid range of values for the stepper.
    #[must_use]
    pub const fn range(&self) -> &RangeInclusive<i32> {
        &self.range
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    #[expect(
        clippy::type_complexity,
        reason = "the tuple is the value's complete field list"
    )]
    pub fn into_parts(
        self,
    ) -> (
        Binding<i32>,
        Computed<i32>,
        ControlLabel<S>,
        Option<Computed<StyledStr>>,
        RangeInclusive<i32>,
    ) {
        (
            self.value,
            self.step,
            self.label,
            self.value_formatter,
            self.range,
        )
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

impl<S: 'static> NativeView for StepperConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout_stretch_axis()
    }
}

impl<S: 'static> MapSlots<S> for StepperConfig<S> {
    type With<T: 'static> = StepperConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> StepperConfig<T> {
        StepperConfig {
            value: self.value,
            step: self.step,
            label: self.label.map_slots(f),
            value_formatter: self.value_formatter,
            range: self.range,
        }
    }
}

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
#[derive(Debug, Clone)]
#[must_use]
pub struct Stepper<L> {
    value: Binding<i32>,
    step: Computed<i32>,
    label: Label<L>,
    value_formatter: Option<Computed<StyledStr>>,
    range: RangeInclusive<i32>,
}

impl<L> Stepper<L> {
    /// Creates a new `Stepper` with the given semantic label and binding value.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<i32>) -> Self {
        Self {
            value: value.clone(),
            step: 1i32.into_computed(),
            label: label.into_label(),
            value_formatter: None,
            range: i32::MIN..=i32::MAX,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the step size for the stepper.
    pub fn step(mut self, step: impl IntoComputed<i32>) -> Self {
        self.step = step.into_computed();
        self
    }

    /// The binding this stepper reads and writes.
    ///
    /// Exposed for the same reason a text field exposes its own: a validator
    /// wraps the control by splicing a filtered binding in front of its value.
    pub const fn value_binding(&mut self) -> &mut Binding<i32> {
        &mut self.value
    }

    /// Sets a formatter for the inline value display. The semantic label is
    /// unaffected.
    pub fn value_formatter<T: Into<StyledStr>>(
        mut self,
        formatter: impl 'static + Fn(i32) -> T,
    ) -> Self {
        let formatter = Rc::new(formatter);
        self.value_formatter = Some(
            self.value
                .clone()
                .map(move |value| formatter(value).into())
                .computed(),
        );
        self
    }

    /// Sets the valid range of values for the stepper.
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
        self.range = start..=end;
        self
    }
}

impl<L: LabelKind> Stepper<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> StepperConfig<S> {
        StepperConfig {
            value: self.value,
            step: self.step,
            label: self.label.into_control_label::<S>(env),
            value_formatter: self.value_formatter,
            range: self.range,
        }
    }
}

impl<L: LabelKind> Stepper<L> {
    /// Returns the configuration for this stepper.
    #[must_use]
    pub fn config(self) -> StepperConfig<Label<L>> {
        StepperConfig {
            value: self.value,
            step: self.step,
            label: self.label.into_unscoped_control_label(),
            value_formatter: self.value_formatter,
            range: self.range,
        }
    }
}

impl<S: View> From<StepperConfig<S>> for Stepper<ControlLabel<S>> {
    fn from(value: StepperConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            value: value.value,
            step: value.step,
            label: Label::from_control_label(value.label, mode),
            value_formatter: value.value_formatter,
            range: value.range,
        }
    }
}

impl<L: LabelKind> View for Stepper<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(Stepper<L>);

/// Creates a new Stepper with the given label and binding value.
///
/// See [`Stepper`] for more details.
pub fn stepper<L: IntoLabel>(label: L, value: &Binding<i32>) -> Stepper<L::Kind> {
    Stepper::new(label, value)
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::{AnyView, Environment, NativeView};
    use waterui_locale::locales;

    use super::stepper;
    use crate::label::LabelDisplayMode;

    /// `docs/layout-spec.md` §3 over label visibility, on the resolved payload
    /// a backend receives — an environment-wide hidden mode included.
    #[test]
    fn stretch_axis_follows_label_visibility() {
        let quantity = Binding::new(0_i32);
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
            let resolved = make().into_config::<AnyView>(&env);
            assert_eq!(
                NativeView::stretch_axis(&resolved),
                expected,
                "label visible: {visible} (resolved payload)"
            );
        }

        let mut env = test_env();
        env.insert(LabelDisplayMode::Hidden);
        let resolved = stepper("Quantity", &quantity).into_config::<AnyView>(&env);
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
