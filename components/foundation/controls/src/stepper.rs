//! A numeric stepper control for incrementing or decrementing values.
//!
//! ![Stepper](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/stepper.svg)

use alloc::rc::Rc;
use core::marker::PhantomData;
use core::ops::RangeInclusive;

use crate::label::{
    ControlLabel, IntoLabel, Label, LabelDisplayMode, LabelKind, impl_label_style_methods,
};
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal, SignalExt, signal::IntoComputed};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View, contract_violation};
use waterui_text::styled::StyledStr;

/// Integer value types that can drive a [`Stepper`].
///
/// Implemented for standard signed and unsigned integer primitives so callers
/// can pass `&Binding<i32>`, `&Binding<u32>`, `&Binding<usize>`, and similar
/// directly without a hand-written mapping.
pub trait StepperValue: Copy + Ord + 'static {
    /// The default inclusive range representable by both `Self` and `i64`.
    #[must_use]
    fn default_range() -> RangeInclusive<i64>;

    /// Converts this value into the canonical `i64` representation.
    fn to_i64(self) -> i64;

    /// Converts a canonical `i64` value back into `Self`, clamping to `Self`'s bounds.
    fn from_i64(value: i64) -> Self;

    /// Maps a typed binding into the canonical `Binding<i64>`.
    #[must_use]
    fn into_i64_binding(binding: &Binding<Self>) -> Binding<i64> {
        Binding::mapping(binding, Self::to_i64, |binding, value| {
            binding.set(Self::from_i64(value));
        })
    }
}

impl StepperValue for i64 {
    fn default_range() -> RangeInclusive<i64> {
        Self::MIN..=Self::MAX
    }

    fn to_i64(self) -> i64 {
        self
    }

    fn from_i64(value: i64) -> Self {
        value
    }

    fn into_i64_binding(binding: &Binding<Self>) -> Binding<i64> {
        binding.clone()
    }
}

macro_rules! impl_stepper_signed_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl StepperValue for $ty {
                fn default_range() -> RangeInclusive<i64> {
                    i64::from(Self::MIN)..=i64::from(Self::MAX)
                }

                fn to_i64(self) -> i64 {
                    i64::from(self)
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "value is clamped to Self::MIN..=Self::MAX before narrowing"
                )]
                fn from_i64(value: i64) -> Self {
                    value.clamp(i64::from(Self::MIN), i64::from(Self::MAX)) as Self
                }
            }
        )+
    };
}

impl_stepper_signed_value!(i8, i16, i32);

impl StepperValue for isize {
    fn default_range() -> RangeInclusive<i64> {
        (Self::MIN as i64)..=(Self::MAX as i64)
    }

    fn to_i64(self) -> i64 {
        self as i64
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "value is clamped to isize::MIN..=isize::MAX before casting"
    )]
    fn from_i64(value: i64) -> Self {
        value.clamp(Self::MIN as i64, Self::MAX as i64) as Self
    }
}

macro_rules! impl_stepper_small_unsigned_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl StepperValue for $ty {
                fn default_range() -> RangeInclusive<i64> {
                    0..=i64::from(Self::MAX)
                }

                fn to_i64(self) -> i64 {
                    i64::from(self)
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value is clamped to 0..=Self::MAX before casting"
                )]
                fn from_i64(value: i64) -> Self {
                    value.clamp(0, i64::from(Self::MAX)) as Self
                }
            }
        )+
    };
}

impl_stepper_small_unsigned_value!(u8, u16, u32);

impl StepperValue for u64 {
    fn default_range() -> RangeInclusive<i64> {
        0..=i64::MAX
    }

    fn to_i64(self) -> i64 {
        i64::try_from(self).unwrap_or(i64::MAX)
    }

    #[expect(
        clippy::cast_sign_loss,
        reason = "negative values are clamped to 0 before casting to u64"
    )]
    fn from_i64(value: i64) -> Self {
        value.max(0) as Self
    }
}

impl StepperValue for usize {
    fn default_range() -> RangeInclusive<i64> {
        let max = i64::try_from(Self::MAX).unwrap_or(i64::MAX);
        0..=max
    }

    fn to_i64(self) -> i64 {
        i64::try_from(self).unwrap_or(i64::MAX)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is clamped to 0..=min(usize::MAX, i64::MAX) before casting"
    )]
    fn from_i64(value: i64) -> Self {
        let max = i64::try_from(Self::MAX).unwrap_or(i64::MAX);
        value.clamp(0, max) as Self
    }
}

/// Validates a stepper range: `start <= end`.
///
/// # Contract violation
///
/// A reversed range panics in debug builds and logs and clamps to
/// `start..=start` in release builds.
#[must_use]
pub fn sanitize_range(range: RangeInclusive<i64>) -> RangeInclusive<i64> {
    let (start, end) = (*range.start(), *range.end());
    if start <= end {
        return start..=end;
    }
    contract_violation!(
        "a stepper range must have start <= end, got {start}..={end}; clamping the range"
    );
    start..=start
}

/// Validates a stepper step: must be strictly positive (`> 0`).
///
/// # Contract violation
///
/// A non-positive step panics in debug builds and logs and keeps `fallback`
/// (clamped to at least `1`) in release builds.
#[must_use]
pub fn sanitize_step(step: i64, fallback: i64) -> i64 {
    if step > 0 {
        return step;
    }
    contract_violation!(
        "a stepper step must be positive, got {step}; the step is ignored and clamped"
    );
    fallback.max(1)
}

/// Clamps `value` in place when it falls outside `range`.
pub fn clamp_value_to_range(value: &Binding<i64>, range: &RangeInclusive<i64>) {
    let range = sanitize_range(range.clone());
    let current = value.snapshot();
    let clamped = current.clamp(*range.start(), *range.end());
    if clamped != current {
        value.set(clamped);
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Configuration options for the [`Stepper`] component.
pub struct StepperConfig<S> {
    /// The binding to the current value of the stepper.
    value: Binding<i64>,
    /// The step size for each increment or decrement.
    step: Computed<i64>,
    /// The label displayed alongside the stepper.
    label: ControlLabel<S>,
    /// Optional formatter for the inline value display, layered on top of the
    /// label. When `None`, the stepper renders only the label; when `Some`, the
    /// formatted value is shown next to the buttons.
    value_formatter: Option<Computed<StyledStr>>,
    /// The valid range of values for the stepper.
    range: Computed<RangeInclusive<i64>>,
}

impl<S> StepperConfig<S> {
    /// The binding to the current value of the stepper.
    #[must_use]
    pub const fn value(&self) -> &Binding<i64> {
        &self.value
    }

    /// The step size for each increment or decrement.
    #[must_use]
    pub const fn step(&self) -> &Computed<i64> {
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
    pub const fn range(&self) -> &Computed<RangeInclusive<i64>> {
        &self.range
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

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    #[expect(
        clippy::type_complexity,
        reason = "the tuple is the value's complete field list"
    )]
    pub fn into_parts(
        self,
    ) -> (
        Binding<i64>,
        Computed<i64>,
        ControlLabel<S>,
        Option<Computed<StyledStr>>,
        Computed<RangeInclusive<i64>>,
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
/// # fn basic(quantity: Binding<u32>) -> impl View {
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
pub struct Stepper<L, T = i64> {
    value: Binding<i64>,
    step: Computed<i64>,
    label: Label<L>,
    value_formatter: Option<Computed<StyledStr>>,
    range: Computed<RangeInclusive<i64>>,
    _value: PhantomData<fn() -> T>,
}

impl<L, T> Stepper<L, T> {
    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// The binding this stepper reads and writes.
    ///
    /// Exposed for the same reason a text field exposes its own: a validator
    /// wraps the control by splicing a filtered binding in front of its value.
    pub const fn value_binding(&mut self) -> &mut Binding<i64> {
        &mut self.value
    }
}

impl<L, T: StepperValue> Stepper<L, T> {
    /// Creates a new `Stepper` with the given semantic label and binding value.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<T>) -> Self {
        Self {
            value: T::into_i64_binding(value),
            step: Computed::constant(1),
            label: label.into_label(),
            value_formatter: None,
            range: Computed::constant(T::default_range()),
            _value: PhantomData,
        }
    }

    /// Sets the step size for the stepper.
    ///
    /// # Contract violation
    ///
    /// A non-positive step panics in debug builds and logs and keeps the
    /// current step in release builds.
    pub fn step(mut self, step: impl IntoComputed<T>) -> Self {
        let previous = self.step;
        let combined = step
            .into_computed()
            .zip(&previous)
            .map(|(candidate, fallback)| sanitize_step(T::to_i64(candidate), fallback))
            .computed();
        let _ = combined.snapshot();
        self.step = combined;
        self
    }

    /// Sets a formatter for the inline value display. The semantic label is
    /// unaffected.
    pub fn value_formatter<R: Into<StyledStr>>(
        mut self,
        formatter: impl 'static + Fn(T) -> R,
    ) -> Self {
        let formatter = Rc::new(formatter);
        self.value_formatter = Some(
            self.value
                .clone()
                .map(move |value| formatter(T::from_i64(value)).into())
                .computed(),
        );
        self
    }

    /// Sets the valid inclusive range of values for the stepper.
    ///
    /// # Contract violation
    ///
    /// A reversed range panics in debug builds and logs and clamps in release
    /// builds.
    pub fn range(mut self, range: impl IntoComputed<RangeInclusive<T>>) -> Self {
        let mapped = range
            .into_computed()
            .map(|range| sanitize_range(T::to_i64(*range.start())..=T::to_i64(*range.end())))
            .computed();
        let initial = mapped.snapshot();
        clamp_value_to_range(&self.value, &initial);
        self.range = mapped;
        self
    }
}

impl<L: LabelKind, T> Stepper<L, T> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> StepperConfig<S> {
        StepperConfig {
            value: self.value,
            step: self.step,
            label: self.label.into_control_label::<S>(env),
            value_formatter: self.value_formatter,
            range: self.range,
        }
    }

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

impl<S: View> From<StepperConfig<S>> for Stepper<ControlLabel<S>, i64> {
    fn from(value: StepperConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            value: value.value,
            step: value.step,
            label: Label::from_control_label(value.label, mode),
            value_formatter: value.value_formatter,
            range: value.range,
            _value: PhantomData,
        }
    }
}

impl<L: LabelKind, T: 'static> View for Stepper<L, T> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(Stepper<L, T>);

/// Creates a new Stepper with the given label and binding value.
///
/// See [`Stepper`] for more details.
pub fn stepper<L: IntoLabel, T: StepperValue>(label: L, value: &Binding<T>) -> Stepper<L::Kind, T> {
    Stepper::new(label, value)
}

#[cfg(test)]
mod tests {
    use core::ops::RangeInclusive;

    use nami::{Binding, Signal, SignalExt};
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

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a stepper step must be positive")
    )]
    fn invalid_step_is_ignored_in_release() {
        let count = Binding::new(0_i32);
        let cfg = stepper("Count", &count).step(3).step(0).config();
        assert_eq!(cfg.step().snapshot(), 3);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a stepper range must have start <= end")
    )]
    fn reversed_range_clamps_in_release() {
        let count = Binding::new(5_i32);
        let cfg = stepper("Count", &count)
            .range(RangeInclusive::new(10, 2))
            .config();
        assert_eq!(cfg.range().snapshot(), 10..=10);
        assert_eq!(count.snapshot(), 10);
    }

    #[test]
    fn generic_u32_stepper_reclamps_in_place_when_range_signal_changes() {
        let count = Binding::new(8_u32);
        let upper = Binding::new(10_u32);
        let cfg = stepper("Items", &count)
            .range(upper.map(|max| 1_u32..=max))
            .step(2_u32)
            .value_formatter(|n| alloc::format!("{n} items"))
            .config();

        let _guard = cfg.watch_range_clamp();
        assert_eq!(count.snapshot(), 8);
        assert_eq!(
            cfg.value_formatter()
                .expect("formatter")
                .snapshot()
                .to_plain(),
            "8 items"
        );

        upper.set(4_u32);
        assert_eq!(count.snapshot(), 4);
        assert_eq!(
            cfg.value_formatter()
                .expect("formatter")
                .snapshot()
                .to_plain(),
            "4 items"
        );
    }

    fn test_env() -> Environment {
        let mut env = Environment::new();
        env.insert(locales::EN);
        env
    }
}
