//! Slider control for adjusting numeric values within a range.
//!
//! This module provides a `Slider` widget that allows users to select a value
//! within a specified numeric range by sliding a handle along a track.
//!
//! ![Slider](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/slider.svg)

use core::ops::RangeInclusive;

use alloc::rc::Rc;

use crate::label::{ControlLabel, IntoLabel, Label, LabelKind, impl_label_style_methods};
use crate::size::ControlSize;
use nami::Binding;
use waterui_core::Str;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::view::ViewConfiguration;
use waterui_core::{AnyView, Environment, Native, NativeView, View, contract_violation};

/// Configuration for the [`Slider`] widget.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SliderConfig<S> {
    /// The label displayed for the slider.
    label: ControlLabel<S>,
    /// The label for the minimum value of the slider.
    min_value_label: S,
    /// The label for the maximum value of the slider.
    max_value_label: S,
    /// The range of values the slider can take.
    range: RangeInclusive<f64>,
    /// The binding to the current value of the slider.
    value: Binding<f64>,
    /// How large the slider is drawn.
    size: ControlSize,
    /// How the value indicator shown above the thumb while it is dragged
    /// formats the value; `None` shows no indicator.
    value_indicator: Option<ValueFormatter>,
    /// The increment the value moves in, anchored at the range's start;
    /// `None` moves it continuously.
    step: Option<f64>,
}

/// `value` held in `range` and, given a `step`, rounded to the nearest
/// multiple of it counted from the range's start: the one rounding every
/// backend applies to a value the user moves.
#[must_use]
pub fn snap(value: f64, range: &RangeInclusive<f64>, step: Option<f64>) -> f64 {
    let (start, end) = (*range.start(), *range.end());
    let value = value.clamp(start, end);
    step.map_or(value, |step| {
        ((value - start) / step)
            .round()
            .mul_add(step, start)
            .clamp(start, end)
    })
}

impl<S> SliderConfig<S> {
    /// Takes the value apart into its fields, in declaration order.
    #[expect(
        clippy::type_complexity,
        reason = "the tuple is the config's fields in declaration order"
    )]
    pub fn into_parts(
        self,
    ) -> (
        ControlLabel<S>,
        S,
        S,
        RangeInclusive<f64>,
        Binding<f64>,
        ControlSize,
        Option<ValueFormatter>,
        Option<f64>,
    ) {
        (
            self.label,
            self.min_value_label,
            self.max_value_label,
            self.range,
            self.value,
            self.size,
            self.value_indicator,
            self.step,
        )
    }

    /// Creates a `SliderConfig` from its parts.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor takes the config's fields in declaration order"
    )]
    pub const fn new(
        label: ControlLabel<S>,
        min_value_label: S,
        max_value_label: S,
        range: RangeInclusive<f64>,
        value: Binding<f64>,
        size: ControlSize,
        value_indicator: Option<ValueFormatter>,
        step: Option<f64>,
    ) -> Self {
        Self {
            label,
            min_value_label,
            max_value_label,
            range,
            value,
            size,
            value_indicator,
            step,
        }
    }

    /// `value` held in the range and, when the slider has a step, rounded to
    /// the nearest multiple of it counted from the range's start.
    #[must_use]
    pub fn snap(&self, value: f64) -> f64 {
        snap(value, &self.range, self.step)
    }

    /// The label displayed for the slider.
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

    /// The label for the minimum value of the slider.
    #[must_use]
    pub const fn min_value_label(&self) -> &S {
        &self.min_value_label
    }

    /// The label for the maximum value of the slider.
    #[must_use]
    pub const fn max_value_label(&self) -> &S {
        &self.max_value_label
    }

    /// The range of values the slider can take.
    #[must_use]
    pub const fn range(&self) -> &RangeInclusive<f64> {
        &self.range
    }

    /// The binding to the current value of the slider.
    #[must_use]
    pub const fn value(&self) -> &Binding<f64> {
        &self.value
    }

    /// How large the slider is drawn.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// How the value indicator shown above the thumb while it is dragged
    /// formats the value; `None` shows no indicator.
    #[must_use]
    pub const fn value_indicator(&self) -> Option<&ValueFormatter> {
        self.value_indicator.as_ref()
    }

    /// The increment the value moves in, anchored at the range's start;
    /// `None` moves it continuously.
    #[must_use]
    pub const fn step(&self) -> Option<f64> {
        self.step
    }
}

impl<S: 'static> NativeView for SliderConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

impl<S: 'static> MapSlots<S> for SliderConfig<S> {
    type With<T: 'static> = SliderConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> SliderConfig<T> {
        SliderConfig {
            label: self.label.map_slots(f.clone()),
            min_value_label: f(self.min_value_label),
            max_value_label: f(self.max_value_label),
            range: self.range,
            value: self.value,
            size: self.size,
            value_indicator: self.value_indicator,
            step: self.step,
        }
    }
}

/// Formats a slider's value for its value indicator.
#[derive(Clone)]
pub struct ValueFormatter(Rc<dyn Fn(f64) -> Str>);

impl ValueFormatter {
    /// Creates a formatter from a function of the value.
    pub fn new(format: impl Fn(f64) -> Str + 'static) -> Self {
        Self(Rc::new(format))
    }

    /// Formats `value`.
    #[must_use]
    pub fn format(&self, value: f64) -> Str {
        (self.0)(value)
    }
}

impl core::fmt::Debug for ValueFormatter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ValueFormatter")
    }
}

/// A control for selecting a value from a continuous range.
///
/// Slider lets users select a value by dragging a thumb along a track.
///
/// # Layout Behavior
///
/// Slider **expands horizontally** to fill available space, but has a fixed height.
/// In an `HStack`, it will take up all remaining width after other views are sized.
///
/// # Examples
///
/// ```rust
/// # use waterui::prelude::*;
/// # use waterui_controls::slider;
/// # fn basic(volume: Binding<f64>) -> impl View {
/// // Basic slider (label required for accessibility)
/// slider("Volume", &volume).range(0.0..=100.0)
/// # }
///
/// # fn labelled_track(brightness: Binding<f64>) -> impl View {
/// // With end-of-track value labels
/// slider("Brightness", &brightness)
///     .min_value_label("Dark")
///     .max_value_label("Bright")
/// # }
///
/// # fn hidden(volume: Binding<f64>) -> impl View {
/// // Visually hidden label, still announced by VoiceOver / TalkBack
/// slider("Volume", &volume).hide_label()
/// # }
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct Slider<L, Min, Max> {
    label: Label<L>,
    min_value_label: Min,
    max_value_label: Max,
    range: RangeInclusive<f64>,
    value: Binding<f64>,
    size: ControlSize,
    value_indicator: Option<ValueFormatter>,
    step: Option<f64>,
}

impl<L> Slider<L, (), ()> {
    /// Creates a new [`Slider`] with the given semantic label, bound to `value`.
    ///
    /// The default range is `0.0..=1.0`; use [`Slider::range`] to override.
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new(label: Label<L>, value: &Binding<f64>) -> Self {
        Self {
            label,
            min_value_label: (),
            max_value_label: (),
            range: 0.0..=1.0,
            value: value.clone(),
            size: ControlSize::ExtraSmall,
            value_indicator: None,
            step: None,
        }
    }
}

impl<L, Min, Max> Slider<L, Min, Max> {
    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// The binding this slider reads and writes.
    ///
    /// Exposed so a validator can wrap the control by splicing a filtered
    /// binding in front of its value, the way a text field allows.
    pub const fn value_binding(&mut self) -> &mut Binding<f64> {
        &mut self.value
    }

    /// Moves the value in increments of `step`, anchored at the start of
    /// the range: a drag rounds to the nearest multiple.
    ///
    /// # Contract violation
    ///
    /// A `step` that is not positive and finite is reported: a debug build
    /// panics, a release build logs it and the slider keeps its current step.
    pub fn step(mut self, step: f64) -> Self {
        if step.is_finite() && step > 0.0 {
            self.step = Some(step);
        } else {
            contract_violation!(
                "a slider step must be positive and finite; the step is ignored and the slider keeps its current step"
            );
        }
        self
    }

    /// Sets the inclusive value range for this slider.
    pub const fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.range = range;
        self
    }

    /// Sets how large the slider is drawn. Sliders default to
    /// [`ControlSize::ExtraSmall`], the thinnest track.
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Shows the value above the thumb while it is dragged, formatted by
    /// `format`.
    pub fn value_indicator(mut self, format: impl Fn(f64) -> Str + 'static) -> Self {
        self.value_indicator = Some(ValueFormatter::new(format));
        self
    }

    /// Sets the label rendered next to the minimum end of the track.
    pub fn min_value_label<NewMin: IntoLabel>(
        self,
        label: NewMin,
    ) -> Slider<L, Label<NewMin::Kind>, Max> {
        Slider {
            label: self.label,
            min_value_label: label.into_label(),
            max_value_label: self.max_value_label,
            range: self.range,
            value: self.value,
            size: self.size,
            value_indicator: self.value_indicator,
            step: self.step,
        }
    }

    /// Sets the label rendered next to the maximum end of the track.
    pub fn max_value_label<NewMax: IntoLabel>(
        self,
        label: NewMax,
    ) -> Slider<L, Min, Label<NewMax::Kind>> {
        Slider {
            label: self.label,
            min_value_label: self.min_value_label,
            max_value_label: label.into_label(),
            range: self.range,
            value: self.value,
            size: self.size,
            value_indicator: self.value_indicator,
            step: self.step,
        }
    }
}

impl<L: LabelKind, Min: View, Max: View> Slider<L, Min, Max> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> SliderConfig<S> {
        SliderConfig {
            label: self.label.into_control_label::<S>(env),
            min_value_label: S::from_view(self.min_value_label),
            max_value_label: S::from_view(self.max_value_label),
            range: self.range,
            value: self.value,
            size: self.size,
            value_indicator: self.value_indicator,
            step: self.step,
        }
    }
}

impl<S: View> ViewConfiguration for SliderConfig<S> {
    type View = Slider<ControlLabel<S>, S, S>;

    fn render(self) -> Self::View {
        let mode = self.label.display_mode_preference();
        Slider {
            label: Label::from_control_label(self.label, mode),
            min_value_label: self.min_value_label,
            max_value_label: self.max_value_label,
            range: self.range,
            value: self.value,
            size: self.size,
            value_indicator: self.value_indicator,
            step: self.step,
        }
    }
}

impl<S: View> From<SliderConfig<S>> for Slider<ControlLabel<S>, S, S> {
    fn from(value: SliderConfig<S>) -> Self {
        value.render()
    }
}

impl<L: LabelKind, Min: View, Max: View> View for Slider<L, Min, Max> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(Slider<L, Min, Max>);

/// Convenience constructor for a [`Slider`] with the given label and value
/// binding, defaulting to the normalized range `0.0..=1.0`.
pub fn slider<L: IntoLabel>(label: L, value: &Binding<f64>) -> Slider<L::Kind, (), ()> {
    Slider::new(label.into_label(), value)
}

#[cfg(test)]
mod tests {
    use nami::Binding;

    use super::{slider, snap};

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a slider step must be positive and finite")
    )]
    fn invalid_step_is_ignored() {
        let value = Binding::container(0.0);
        let stepped = slider("Volume", &value).step(2.0).step(0.0);

        // Release outcome: the slider keeps the step it had.
        assert_eq!(stepped.step, Some(2.0));
    }

    #[test]
    fn snap_rounds_from_the_range_start_and_stays_in_range() {
        let range = 1.0..=10.0;
        assert_eq!(snap(4.4, &range, Some(2.0)).to_bits(), 5.0_f64.to_bits());
        assert_eq!(snap(4.4, &range, None).to_bits(), 4.4_f64.to_bits());
        // A step that overshoots the end is held to it.
        assert_eq!(snap(10.0, &range, Some(2.0)).to_bits(), 10.0_f64.to_bits());
        assert_eq!(snap(-3.0, &range, Some(2.0)).to_bits(), 1.0_f64.to_bits());
    }
}
