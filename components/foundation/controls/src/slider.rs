//! Slider control for adjusting numeric values within a range.
//!
//! This module provides a `Slider` widget that allows users to select a value
//! within a specified numeric range by sliding a handle along a track.
//!
//! ![Slider](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/slider.svg)

use alloc::rc::Rc;
use core::fmt::{self, Debug, Formatter};
use core::marker::PhantomData;
use core::ops::RangeInclusive;

use crate::label::{ControlLabel, IntoLabel, Label, LabelKind, impl_label_style_methods};
use crate::size::ControlSize;
use nami::signal::IntoComputed;
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal, SignalExt};
use waterui_core::Str;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View, contract_violation};

/// Numeric value types that can drive a [`Slider`].
///
/// Implemented for standard floating-point and integer primitives so callers
/// can pass `&Binding<f32>`, `&Binding<f64>`, `&Binding<u32>`, and similar
/// directly without a hand-written mapping.
pub trait SliderValue: Copy + PartialOrd + 'static {
    /// Converts this value into the canonical `f64` representation.
    fn to_f64(self) -> f64;

    /// Converts a canonical `f64` value back into `Self`.
    fn from_f64(value: f64) -> Self;

    /// Maps a typed binding into the canonical `Binding<f64>`.
    #[must_use]
    fn into_f64_binding(binding: &Binding<Self>) -> Binding<f64> {
        Binding::mapping(binding, Self::to_f64, |binding, value| {
            binding.set(Self::from_f64(value));
        })
    }
}

impl SliderValue for f64 {
    fn to_f64(self) -> f64 {
        self
    }

    fn from_f64(value: f64) -> Self {
        value
    }

    fn into_f64_binding(binding: &Binding<Self>) -> Binding<f64> {
        binding.clone()
    }
}

impl SliderValue for f32 {
    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "f32 slider narrows canonical f64 back to the bound f32 precision"
    )]
    fn from_f64(value: f64) -> Self {
        value as Self
    }
}

macro_rules! impl_slider_small_signed_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SliderValue for $ty {
                fn to_f64(self) -> f64 {
                    f64::from(self)
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value is rounded and clamped to Self::MIN..=Self::MAX before casting"
                )]
                fn from_f64(value: f64) -> Self {
                    let min = f64::from(Self::MIN);
                    let max = f64::from(Self::MAX);
                    let clamped = if value.is_finite() {
                        value.round().clamp(min, max)
                    } else {
                        min
                    };
                    clamped as Self
                }
            }
        )+
    };
}

macro_rules! impl_slider_wide_signed_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SliderValue for $ty {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "canonical slider config erases all numeric bindings to f64"
                )]
                fn to_f64(self) -> f64 {
                    self as f64
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_precision_loss,
                    reason = "value is rounded and clamped to Self::MIN..=Self::MAX before casting"
                )]
                fn from_f64(value: f64) -> Self {
                    let min = Self::MIN as f64;
                    let max = Self::MAX as f64;
                    let clamped = if value.is_finite() {
                        value.round().clamp(min, max)
                    } else {
                        min
                    };
                    clamped as Self
                }
            }
        )+
    };
}

macro_rules! impl_slider_small_unsigned_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SliderValue for $ty {
                fn to_f64(self) -> f64 {
                    f64::from(self)
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value is rounded and clamped to 0..=Self::MAX before casting"
                )]
                fn from_f64(value: f64) -> Self {
                    let min = f64::from(Self::MIN);
                    let max = f64::from(Self::MAX);
                    let clamped = if value.is_finite() {
                        value.round().clamp(min, max)
                    } else {
                        min
                    };
                    clamped as Self
                }
            }
        )+
    };
}

macro_rules! impl_slider_wide_unsigned_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SliderValue for $ty {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "canonical slider config erases all numeric bindings to f64"
                )]
                fn to_f64(self) -> f64 {
                    self as f64
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "value is rounded and clamped to 0..=Self::MAX before casting"
                )]
                fn from_f64(value: f64) -> Self {
                    let min = Self::MIN as f64;
                    let max = Self::MAX as f64;
                    let clamped = if value.is_finite() {
                        value.round().clamp(min, max)
                    } else {
                        min
                    };
                    clamped as Self
                }
            }
        )+
    };
}

impl_slider_small_signed_value!(i8, i16, i32);
impl_slider_wide_signed_value!(i64, isize);
impl_slider_small_unsigned_value!(u8, u16, u32);
impl_slider_wide_unsigned_value!(u64, usize);

/// Validates `range` for a slider: bounds must be finite with `start <= end`.
///
/// # Contract violation
///
/// Reversed or non-finite bounds panic in debug builds and log and clamp in
/// release builds.
#[must_use]
pub fn sanitize_range(range: RangeInclusive<f64>) -> RangeInclusive<f64> {
    let (start, end) = (*range.start(), *range.end());
    if start.is_finite() && end.is_finite() && start <= end {
        return start..=end;
    }
    contract_violation!(
        "a slider range must have finite bounds with start <= end, got {start:?}..={end:?}; clamping the range"
    );
    let start = if start.is_finite() { start } else { 0.0 };
    let end = if end.is_finite() {
        end.max(start)
    } else {
        start.max(1.0)
    };
    start..=end
}

/// Validates a slider step: must be positive and finite.
///
/// # Contract violation
///
/// A non-positive or non-finite step panics in debug builds and logs and
/// returns `None` in release builds.
#[must_use]
pub fn sanitize_step(step: f64) -> Option<f64> {
    if step.is_finite() && step > 0.0 {
        Some(step)
    } else {
        contract_violation!(
            "a slider step must be positive and finite, got {step:?}; the step is ignored"
        );
        None
    }
}

/// Clamps `value` in place when it falls outside `range`.
pub fn clamp_value_to_range(value: &Binding<f64>, range: &RangeInclusive<f64>) {
    let range = sanitize_range(range.clone());
    let (start, end) = (*range.start(), *range.end());
    let current = value.snapshot();
    let clamped = if current.is_finite() {
        current.clamp(start, end)
    } else {
        start
    };
    if clamped.to_bits() != current.to_bits() {
        value.set(clamped);
    }
}

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
    range: Computed<RangeInclusive<f64>>,
    /// The binding to the current value of the slider.
    value: Binding<f64>,
    /// How large the slider is drawn.
    size: ControlSize,
    /// How the value indicator shown above the thumb while it is dragged
    /// formats the value; `None` shows no indicator.
    value_formatter: Option<ValueFormatter>,
    /// The increment the value moves in, anchored at the range's start;
    /// `None` moves it continuously.
    step: Computed<Option<f64>>,
}

/// `value` held in `range` and, given a `step`, rounded to the nearest
/// multiple of it counted from the range's start: the one rounding every
/// backend applies to a value the user moves.
#[must_use]
pub fn snap(value: f64, range: &RangeInclusive<f64>, step: Option<f64>) -> f64 {
    let range = sanitize_range(range.clone());
    let (start, end) = (*range.start(), *range.end());
    let value = if value.is_finite() {
        value.clamp(start, end)
    } else {
        start
    };
    let step = step.and_then(sanitize_step);
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
        Computed<RangeInclusive<f64>>,
        Binding<f64>,
        ControlSize,
        Option<ValueFormatter>,
        Computed<Option<f64>>,
    ) {
        (
            self.label,
            self.min_value_label,
            self.max_value_label,
            self.range,
            self.value,
            self.size,
            self.value_formatter,
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
        range: Computed<RangeInclusive<f64>>,
        value: Binding<f64>,
        size: ControlSize,
        value_formatter: Option<ValueFormatter>,
        step: Computed<Option<f64>>,
    ) -> Self {
        Self {
            label,
            min_value_label,
            max_value_label,
            range,
            value,
            size,
            value_formatter,
            step,
        }
    }

    /// `value` held in the current range and, when the slider has a step,
    /// rounded to the nearest multiple of it counted from the range's start.
    #[must_use]
    pub fn snap(&self, value: f64) -> f64 {
        snap(value, &self.range.snapshot(), self.step.snapshot())
    }

    /// Watches `range` and re-clamps `value` in place whenever the range
    /// updates, returning the subscription guard.
    pub fn watch_range_clamp(&self) -> BoxWatcherGuard {
        let value = self.value.clone();
        clamp_value_to_range(&value, &self.range.snapshot());
        self.range.watch(move |ctx| {
            clamp_value_to_range(&value, ctx.value());
        })
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
    pub const fn range(&self) -> &Computed<RangeInclusive<f64>> {
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
    pub const fn value_formatter(&self) -> Option<&ValueFormatter> {
        self.value_formatter.as_ref()
    }

    /// The increment the value moves in, anchored at the range's start;
    /// `None` moves it continuously.
    #[must_use]
    pub const fn step(&self) -> &Computed<Option<f64>> {
        &self.step
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
            value_formatter: self.value_formatter,
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

impl Debug for ValueFormatter {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
/// # fn labelled_track(brightness: Binding<f32>) -> impl View {
/// // Directly bound to f32 with end-of-track value labels
/// slider("Brightness", &brightness)
///     .range(0.0_f32..=1.0_f32)
///     .step(0.1_f32)
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
pub struct Slider<L, Min, Max, T = f64> {
    label: Label<L>,
    min_value_label: Min,
    max_value_label: Max,
    range: Computed<RangeInclusive<f64>>,
    value: Binding<f64>,
    size: ControlSize,
    value_formatter: Option<ValueFormatter>,
    step: Computed<Option<f64>>,
    _value: PhantomData<fn() -> T>,
}

impl<L, T: SliderValue> Slider<L, (), (), T> {
    /// Creates a new [`Slider`] with the given semantic label, bound to `value`.
    ///
    /// The default range is `0.0..=1.0`; use [`Slider::range`] to override.
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<T>) -> Self {
        Self {
            label: label.into_label(),
            min_value_label: (),
            max_value_label: (),
            range: Computed::constant(0.0..=1.0),
            value: T::into_f64_binding(value),
            size: ControlSize::ExtraSmall,
            value_formatter: None,
            step: Computed::constant(None),
            _value: PhantomData,
        }
    }
}

impl<L, Min, Max, T> Slider<L, Min, Max, T> {
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

    /// Sets how large the slider is drawn. Sliders default to
    /// [`ControlSize::ExtraSmall`], the thinnest track.
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Sets the label rendered next to the minimum end of the track.
    pub fn min_value_label<NewMin: IntoLabel>(
        self,
        label: NewMin,
    ) -> Slider<L, Label<NewMin::Kind>, Max, T> {
        Slider {
            label: self.label,
            min_value_label: label.into_label(),
            max_value_label: self.max_value_label,
            range: self.range,
            value: self.value,
            size: self.size,
            value_formatter: self.value_formatter,
            step: self.step,
            _value: PhantomData,
        }
    }

    /// Sets the label rendered next to the maximum end of the track.
    pub fn max_value_label<NewMax: IntoLabel>(
        self,
        label: NewMax,
    ) -> Slider<L, Min, Label<NewMax::Kind>, T> {
        Slider {
            label: self.label,
            min_value_label: self.min_value_label,
            max_value_label: label.into_label(),
            range: self.range,
            value: self.value,
            size: self.size,
            value_formatter: self.value_formatter,
            step: self.step,
            _value: PhantomData,
        }
    }
}

impl<L, Min, Max, T: SliderValue> Slider<L, Min, Max, T> {
    /// Moves the value in increments of `step`, anchored at the start of
    /// the range: a drag rounds to the nearest multiple.
    ///
    /// # Contract violation
    ///
    /// A `step` that is not positive and finite is reported: a debug build
    /// panics, a release build logs it and the slider keeps its current step.
    pub fn step(mut self, step: impl IntoComputed<T>) -> Self {
        let step_signal = step.into_computed();
        let previous = self.step;
        let combined = step_signal
            .zip(&previous)
            .map(|(candidate, fallback)| sanitize_step(T::to_f64(candidate)).or(fallback))
            .computed();
        // Validate the initial value eagerly so invalid static steps surface immediately.
        let _ = combined.snapshot();
        self.step = combined;
        self
    }

    /// Sets the inclusive value range for this slider.
    ///
    /// # Contract violation
    ///
    /// A reversed or non-finite range panics in debug builds and logs and
    /// clamps in release builds.
    pub fn range(mut self, range: impl IntoComputed<RangeInclusive<T>>) -> Self {
        let mapped = range
            .into_computed()
            .map(|range| sanitize_range(T::to_f64(*range.start())..=T::to_f64(*range.end())))
            .computed();
        let initial = mapped.snapshot();
        clamp_value_to_range(&self.value, &initial);
        self.range = mapped;
        self
    }

    /// Formats the value displayed in the slider's value indicator while the
    /// thumb is dragged.
    pub fn value_formatter<R: Into<Str>>(mut self, format: impl Fn(T) -> R + 'static) -> Self {
        self.value_formatter = Some(ValueFormatter::new(move |value| {
            format(T::from_f64(value)).into()
        }));
        self
    }
}

impl<L: LabelKind, Min: View, Max: View, T> Slider<L, Min, Max, T> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> SliderConfig<S> {
        SliderConfig {
            label: self.label.into_control_label::<S>(env),
            min_value_label: S::from_view(self.min_value_label),
            max_value_label: S::from_view(self.max_value_label),
            range: self.range,
            value: self.value,
            size: self.size,
            value_formatter: self.value_formatter,
            step: self.step,
        }
    }
}

impl<S: View> From<SliderConfig<S>> for Slider<ControlLabel<S>, S, S, f64> {
    fn from(value: SliderConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            min_value_label: value.min_value_label,
            max_value_label: value.max_value_label,
            range: value.range,
            value: value.value,
            size: value.size,
            value_formatter: value.value_formatter,
            step: value.step,
            _value: PhantomData,
        }
    }
}

impl<L: LabelKind, Min: View, Max: View, T: 'static> View for Slider<L, Min, Max, T> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(Slider<L, Min, Max, T>);

/// Convenience constructor for a [`Slider`] with the given label and value
/// binding, defaulting to the normalized range `0.0..=1.0`.
pub fn slider<L: IntoLabel, T: SliderValue>(
    label: L,
    value: &Binding<T>,
) -> Slider<L::Kind, (), (), T> {
    Slider::new(label, value)
}

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal, SignalExt};
    use waterui_core::{AnyView, Environment};
    use waterui_locale::locales;

    use super::{slider, snap};

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a slider step must be positive and finite")
    )]
    fn invalid_step_is_ignored() {
        let value = Binding::new(0.0_f64);
        let stepped = slider("Volume", &value).step(2.0).step(0.0);

        // Release outcome: the slider keeps the step it had.
        assert_eq!(stepped.step.snapshot(), Some(2.0));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a slider range must have finite bounds with start <= end")
    )]
    fn reversed_range_clamps_without_panicking_in_release() {
        let value = Binding::new(5.0_f64);
        let clamped = slider("Volume", &value).range(10.0..=2.0);
        assert_eq!(clamped.range.snapshot(), 10.0..=10.0);
        assert_eq!(snap(5.0, &(10.0..=2.0), None), 10.0);
    }

    #[test]
    fn snap_anchors_at_range_start_and_clamps_to_bounds() {
        assert_eq!(snap(0.0, &(0.0..=10.0), Some(2.5)), 0.0);
        assert_eq!(snap(3.6, &(0.0..=10.0), Some(2.5)), 2.5);
        assert_eq!(snap(3.8, &(0.0..=10.0), Some(2.5)), 5.0);
        assert_eq!(snap(15.0, &(0.0..=10.0), Some(2.5)), 10.0);
        assert_eq!(snap(-4.0, &(0.0..=10.0), Some(2.5)), 0.0);
    }

    #[test]
    fn generic_f32_and_u32_bindings_drive_slider_and_reclamp_on_range_change() {
        let mut env = Environment::new();
        env.insert(locales::EN);

        let opacity = Binding::new(0.8_f32);
        let max_opacity = Binding::new(1.0_f32);
        let step = Binding::new(0.1_f32);
        let cfg = slider("Opacity", &opacity)
            .range(max_opacity.map(|max| 0.0_f32..=max))
            .step(step.clone())
            .value_formatter(|v| alloc::format!("{v:.1}"))
            .into_config::<AnyView>(&env);

        let _guard = cfg.watch_range_clamp();
        assert!((opacity.snapshot() - 0.8).abs() < 1e-6);
        assert_eq!(cfg.step().snapshot().map(|s| (s * 10.0).round()), Some(1.0));

        max_opacity.set(0.5_f32);
        assert!((opacity.snapshot() - 0.5).abs() < 1e-6);

        step.set(0.25_f32);
        assert_eq!(
            cfg.step().snapshot().map(|s| (s * 100.0).round()),
            Some(25.0)
        );

        let rating = Binding::new(4_u32);
        let u32_cfg = slider("Rating", &rating)
            .range(1_u32..=5_u32)
            .step(1_u32)
            .into_config::<AnyView>(&env);
        u32_cfg.value().set(3.0);
        assert_eq!(rating.snapshot(), 3);
    }
}
