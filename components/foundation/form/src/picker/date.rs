//! Date picker component.

use alloc::string::{String, ToString};
use core::{marker::PhantomData, ops::RangeInclusive};

pub use jiff::civil::{Date, DateTime, Time};
use nami::signal::IntoComputed;
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal, SignalExt};
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::{AnyView, Environment, Native, NativeView, View, contract_violation};

/// Validates a date-picker range: `start <= end`.
///
/// # Contract violation
///
/// A reversed range panics in debug builds and logs and clamps to
/// `start..=start` in release builds.
#[must_use]
pub fn sanitize_range(range: RangeInclusive<DateTime>) -> RangeInclusive<DateTime> {
    let (start, end) = (*range.start(), *range.end());
    if start <= end {
        return start..=end;
    }
    contract_violation!(
        "a date picker range must have start <= end, got {start}..={end}; clamping the range"
    );
    start..=start
}

/// Clamps `value` in place when it falls outside `range`.
pub fn clamp_value_to_range(value: &Binding<DateTime>, range: &RangeInclusive<DateTime>) {
    let range = sanitize_range(range.clone());
    let current = value.snapshot();
    let clamped = current.clamp(*range.start(), *range.end());
    if clamped != current {
        value.set(clamped);
    }
}

/// Configuration for the `DatePicker` component.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatePickerConfig<S> {
    /// The label displayed for the date picker.
    label: ControlLabel<S>,
    /// The binding to the selected value.
    value: Binding<DateTime>,
    /// The range of valid values.
    range: Computed<RangeInclusive<DateTime>>,
    /// The type of date picker.
    ty: DatePickerType,
}

impl<S> DatePickerConfig<S> {
    /// The label displayed for the date picker.
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

    /// The binding to the selected value.
    #[must_use]
    pub const fn value(&self) -> &Binding<DateTime> {
        &self.value
    }

    /// The range of valid values.
    #[must_use]
    pub const fn range(&self) -> &Computed<RangeInclusive<DateTime>> {
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

    /// The type of date picker.
    #[must_use]
    pub const fn ty(&self) -> DatePickerType {
        self.ty
    }
}

/// Enum representing the different types of date pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each date picker type variant by variant; a new date picker type must fail their build instead of falling into a wildcard arm"
)]
pub enum DatePickerType {
    /// Date only.
    Date,
    /// Hour and minute.
    HourAndMinute,
    /// Hour, minute, and second.
    HourMinuteAndSecond,
    /// Date, hour, and minute.
    #[default]
    DateHourAndMinute,
    /// Date, hour, minute, and second.
    DateHourMinuteAndSecond,
}

impl DatePickerType {
    /// Returns the `strftime`/`strptime` format used to present this picker.
    #[must_use]
    pub const fn format_string(self) -> &'static str {
        match self {
            Self::Date => "%F",
            Self::HourAndMinute => "%R",
            Self::HourMinuteAndSecond => "%T",
            Self::DateHourAndMinute => "%F %R",
            Self::DateHourMinuteAndSecond => "%F %T",
        }
    }

    /// Formats a picker value exactly as `WaterUI` presents it to the user.
    #[must_use]
    pub fn format_value(self, value: DateTime) -> String {
        match self {
            Self::Date => value.date().strftime(self.format_string()).to_string(),
            Self::HourAndMinute | Self::HourMinuteAndSecond => {
                value.time().strftime(self.format_string()).to_string()
            }
            Self::DateHourAndMinute | Self::DateHourMinuteAndSecond => {
                value.strftime(self.format_string()).to_string()
            }
        }
    }

    /// Parses a user-provided picker value into the canonical internal datetime.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` does not match this picker's format.
    pub fn parse_value(self, value: &str) -> Result<DateTime, jiff::Error> {
        match self {
            Self::Date => Date::strptime(self.format_string(), value).map(start_of_day),
            Self::HourAndMinute | Self::HourMinuteAndSecond => {
                Time::strptime(self.format_string(), value).map(anchor_time)
            }
            Self::DateHourAndMinute | Self::DateHourMinuteAndSecond => {
                DateTime::strptime(self.format_string(), value)
            }
        }
    }
}

/// Presentation components supported by [`DatePicker`] when bound to [`Date`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DatePickerComponents {
    /// Year, month, and day.
    #[default]
    Date,
}

/// Presentation components supported by [`DatePicker`] when bound to [`Time`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TimePickerComponents {
    /// Hour and minute.
    #[default]
    HourAndMinute,
    /// Hour, minute, and second.
    HourMinuteAndSecond,
}

/// Presentation components supported by [`DatePicker`] when bound to [`DateTime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DateTimePickerComponents {
    /// Date only.
    Date,
    /// Hour and minute.
    HourAndMinute,
    /// Hour, minute, and second.
    HourMinuteAndSecond,
    /// Date, hour, and minute.
    #[default]
    DateHourAndMinute,
    /// Date, hour, minute, and second.
    DateHourMinuteAndSecond,
}

/// Types that can drive a [`DatePicker`].
///
/// Implemented for `Date`, `Time`, and `DateTime`. A single
/// [`DatePicker::new`] constructor dispatches on this trait, so callers do not
/// have to remember separate constructor names per binding type.
pub trait DatePickable: Clone + 'static {
    /// Typed component selection valid for values of `Self`.
    type Components: Copy + Eq + Default + 'static;

    /// Maps a `Binding<Self>` into the canonical `Binding<DateTime>` that
    /// `DatePicker` stores internally, preserving round-trip semantics.
    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime>;

    /// Lifts an inclusive range of `Self` into the canonical
    /// `RangeInclusive<DateTime>` used by the picker.
    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime>;

    /// Resolves a typed component selection for `Self` into the backend [`DatePickerType`].
    fn resolve_picker_type(components: Self::Components) -> DatePickerType;

    /// Returns the default [`DatePickerType`] when the user has not specified
    /// one explicitly.
    #[must_use]
    fn default_picker_type() -> DatePickerType {
        Self::resolve_picker_type(Self::Components::default())
    }

    /// Returns the canonical full range used when the user has not constrained
    /// the picker.
    fn full_range() -> RangeInclusive<DateTime>;
}

/// Bound types whose [`DatePicker`] presentation supports seconds (`Time` and `DateTime`).
pub trait TimePickable: DatePickable {
    /// Returns the [`DatePickerType`] corresponding to `ty` with seconds shown.
    fn with_seconds(ty: DatePickerType) -> DatePickerType;
}

/// A control for selecting dates and times.
///
/// `DatePicker` is generic over the bound value type `T` (`Date`, `Time`, or
/// `DateTime`) so invalid ranges and unrepresentable time/date components fail
/// to compile. Internally it maps onto a [`DateTime`] binding so backends can
/// round-trip hidden components without loss.
///
/// ```compile_fail
/// use waterui_core::binding;
/// use waterui_form::picker::date::{Date, DatePicker, Time};
///
/// let date = binding(Date::constant(2026, 3, 20));
/// let start = Time::constant(8, 0, 0, 0);
/// let end = Time::constant(17, 0, 0, 0);
/// let _ = DatePicker::new("Due", &date).range(start..=end);
/// ```
///
/// ```compile_fail
/// use waterui_core::binding;
/// use waterui_form::picker::date::{Date, DatePicker};
///
/// let date = binding(Date::constant(2026, 3, 20));
/// let _ = DatePicker::new("Due", &date).seconds();
/// ```
///
/// ```compile_fail
/// use waterui_core::binding;
/// use waterui_form::picker::date::{Date, DatePicker, TimePickerComponents};
///
/// let date = binding(Date::constant(2026, 3, 20));
/// let _ = DatePicker::new("Due", &date).components(TimePickerComponents::HourAndMinute);
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct DatePicker<L, T = DateTime> {
    label: Label<L>,
    value: Binding<DateTime>,
    range: Computed<RangeInclusive<DateTime>>,
    ty: DatePickerType,
    _bound: PhantomData<fn() -> T>,
}

impl<L: LabelKind, T> DatePicker<L, T> {
    /// Returns the configuration for this date picker.
    #[must_use]
    pub fn config(self) -> DatePickerConfig<Label<L>> {
        DatePickerConfig {
            label: self.label.into_unscoped_control_label(),
            value: self.value,
            range: self.range,
            ty: self.ty,
        }
    }
}

impl<S: View> From<DatePickerConfig<S>> for DatePicker<ControlLabel<S>, DateTime> {
    fn from(value: DatePickerConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            value: value.value,
            range: value.range,
            ty: value.ty,
            _bound: PhantomData,
        }
    }
}

impl<S: 'static> NativeView for DatePickerConfig<S> {
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl<S: 'static> MapSlots<S> for DatePickerConfig<S> {
    type With<T: 'static> = DatePickerConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> DatePickerConfig<T> {
        DatePickerConfig {
            label: self.label.map_slots(f),
            value: self.value,
            range: self.range,
            ty: self.ty,
        }
    }
}

impl<L: LabelKind, T> DatePicker<L, T> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> DatePickerConfig<S> {
        DatePickerConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
            range: self.range,
            ty: self.ty,
        }
    }
}

impl<L: LabelKind, T: 'static> View for DatePicker<L, T> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl<L, T> DatePicker<L, T> {
    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }
}

impl<L, T: DatePickable> DatePicker<L, T> {
    /// Creates a picker bound to `value` with the given semantic label,
    /// dispatching on the binding's type (`Date`, `Time`, or `DateTime`) via
    /// [`DatePickable`].
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<T>) -> Self {
        Self {
            label: label.into_label(),
            value: T::into_datetime_binding(value),
            range: Computed::constant(T::full_range()),
            ty: T::default_picker_type(),
            _bound: PhantomData,
        }
    }

    /// Sets the valid range for the picker, constrained to the bound value type `T`.
    ///
    /// # Contract violation
    ///
    /// A reversed range panics in debug builds and logs and clamps in release
    /// builds.
    pub fn range(mut self, range: impl IntoComputed<RangeInclusive<T>>) -> Self {
        let mapped = range
            .into_computed()
            .map(|range| sanitize_range(T::into_picker_range(range)))
            .computed();
        let initial = mapped.snapshot();
        clamp_value_to_range(&self.value, &initial);
        self.range = mapped;
        self
    }

    /// Sets which components of `T` are presented by the picker.
    pub fn components(mut self, components: T::Components) -> Self {
        self.ty = T::resolve_picker_type(components);
        self
    }
}

impl<L, T: TimePickable> DatePicker<L, T> {
    /// Displays seconds in the time portion of the picker.
    pub fn seconds(mut self) -> Self {
        self.ty = T::with_seconds(self.ty);
        self
    }
}

impl<L> DatePicker<L, DateTime> {
    /// Presents only the date components while preserving the time on round-trip.
    pub const fn date_only(mut self) -> Self {
        self.ty = DatePickerType::Date;
        self
    }

    /// Presents only the time components while preserving the date on round-trip.
    pub const fn time_only(mut self) -> Self {
        self.ty = match self.ty {
            DatePickerType::HourMinuteAndSecond | DatePickerType::DateHourMinuteAndSecond => {
                DatePickerType::HourMinuteAndSecond
            }
            DatePickerType::Date
            | DatePickerType::HourAndMinute
            | DatePickerType::DateHourAndMinute => DatePickerType::HourAndMinute,
        };
        self
    }
}

impl_label_style_methods!(DatePicker<L, T>);

impl DatePickable for Date {
    type Components = DatePickerComponents;

    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime> {
        map_date_binding(binding)
    }

    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime> {
        start_of_day(*range.start())..=end_of_day(*range.end())
    }

    fn resolve_picker_type(components: Self::Components) -> DatePickerType {
        match components {
            DatePickerComponents::Date => DatePickerType::Date,
        }
    }

    fn full_range() -> RangeInclusive<DateTime> {
        Self::into_picker_range(Self::MIN..=Self::MAX)
    }
}

impl DatePickable for Time {
    type Components = TimePickerComponents;

    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime> {
        map_time_binding(binding)
    }

    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime> {
        anchor_time(*range.start())..=anchor_time(*range.end())
    }

    fn resolve_picker_type(components: Self::Components) -> DatePickerType {
        match components {
            TimePickerComponents::HourAndMinute => DatePickerType::HourAndMinute,
            TimePickerComponents::HourMinuteAndSecond => DatePickerType::HourMinuteAndSecond,
        }
    }

    fn full_range() -> RangeInclusive<DateTime> {
        Self::into_picker_range(Self::MIN..=end_of_day_time())
    }
}

impl TimePickable for Time {
    fn with_seconds(_ty: DatePickerType) -> DatePickerType {
        DatePickerType::HourMinuteAndSecond
    }
}

impl DatePickable for DateTime {
    type Components = DateTimePickerComponents;

    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime> {
        binding.clone()
    }

    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime> {
        range
    }

    fn resolve_picker_type(components: Self::Components) -> DatePickerType {
        match components {
            DateTimePickerComponents::Date => DatePickerType::Date,
            DateTimePickerComponents::HourAndMinute => DatePickerType::HourAndMinute,
            DateTimePickerComponents::HourMinuteAndSecond => DatePickerType::HourMinuteAndSecond,
            DateTimePickerComponents::DateHourAndMinute => DatePickerType::DateHourAndMinute,
            DateTimePickerComponents::DateHourMinuteAndSecond => {
                DatePickerType::DateHourMinuteAndSecond
            }
        }
    }

    fn full_range() -> RangeInclusive<DateTime> {
        full_picker_range()
    }
}

impl TimePickable for DateTime {
    fn with_seconds(ty: DatePickerType) -> DatePickerType {
        match ty {
            DatePickerType::HourAndMinute | DatePickerType::HourMinuteAndSecond => {
                DatePickerType::HourMinuteAndSecond
            }
            DatePickerType::Date
            | DatePickerType::DateHourAndMinute
            | DatePickerType::DateHourMinuteAndSecond => DatePickerType::DateHourMinuteAndSecond,
        }
    }
}

fn map_date_binding(date: &Binding<Date>) -> Binding<DateTime> {
    Binding::mapping(date, start_of_day, |binding, value| {
        binding.set(value.date());
    })
}

fn map_time_binding(time: &Binding<Time>) -> Binding<DateTime> {
    Binding::mapping(time, anchor_time, |binding, value| {
        binding.set(value.time());
    })
}

const fn full_picker_range() -> RangeInclusive<DateTime> {
    start_of_day(Date::MIN)..=end_of_day(Date::MAX)
}

const fn start_of_day(date: Date) -> DateTime {
    date.at(0, 0, 0, 0)
}

const fn end_of_day(date: Date) -> DateTime {
    date.at(23, 59, 59, 0)
}

fn time_anchor_date() -> Date {
    Date::new(2000, 1, 1).expect("time-only picker anchor date must be valid")
}

fn anchor_time(time: Time) -> DateTime {
    time_anchor_date().at(time.hour(), time.minute(), time.second(), 0)
}

fn end_of_day_time() -> Time {
    Time::new(23, 59, 59, 0).expect("end-of-day time must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::{Signal, binding};

    #[test]
    fn date_range_maps_full_days() {
        let start = Date::new(2025, 1, 10).unwrap();
        let end = Date::new(2025, 1, 12).unwrap();
        let mapped = Date::into_picker_range(start..=end);

        assert_eq!(mapped.start().date(), start);
        assert_eq!(mapped.start().time(), Time::MIN);
        assert_eq!(mapped.end().date(), end);
        assert_eq!(mapped.end().time(), end_of_day_time());
    }

    #[test]
    fn time_binding_preserves_time_value() {
        let source = Binding::container(Time::new(8, 30, 0, 0).unwrap());
        let mapped = map_time_binding(&source);

        mapped.set(anchor_time(Time::new(18, 45, 12, 0).unwrap()));

        assert_eq!(source.snapshot(), Time::new(18, 45, 12, 0).unwrap());
    }

    #[test]
    fn date_picker_type_round_trips_display_values() {
        let date = Date::new(2025, 1, 10).unwrap();
        let time = Time::new(18, 45, 12, 0).unwrap();
        let date_time = date.at(time.hour(), time.minute(), time.second(), 0);

        let cases = [
            (DatePickerType::Date, start_of_day(date), start_of_day(date)),
            (
                DatePickerType::HourAndMinute,
                anchor_time(Time::new(18, 45, 0, 0).unwrap()),
                anchor_time(Time::new(18, 45, 0, 0).unwrap()),
            ),
            (
                DatePickerType::HourMinuteAndSecond,
                anchor_time(time),
                anchor_time(time),
            ),
            (
                DatePickerType::DateHourAndMinute,
                date.at(time.hour(), time.minute(), 0, 0),
                date.at(time.hour(), time.minute(), 0, 0),
            ),
            (
                DatePickerType::DateHourMinuteAndSecond,
                date_time,
                date_time,
            ),
        ];

        for (picker_type, value, expected) in cases {
            let formatted = picker_type.format_value(value);
            let parsed = picker_type.parse_value(&formatted).unwrap();
            assert_eq!(parsed, expected, "round trip failed for {picker_type:?}");
        }
    }

    #[test]
    fn date_picker_config_defaults_and_builders_match_bound_type() {
        let date = Binding::container(Date::constant(2025, 6, 15));
        let date_cfg = DatePicker::new("Date", &date)
            .range(Date::constant(2025, 1, 1)..=Date::constant(2025, 12, 31))
            .config();
        assert_eq!(date_cfg.ty(), DatePickerType::Date);

        let time = Binding::container(Time::constant(8, 30, 0, 0));
        assert_eq!(
            DatePicker::new("Time", &time).config().ty(),
            DatePickerType::HourAndMinute
        );
        assert_eq!(
            DatePicker::new("Time", &time).seconds().config().ty(),
            DatePickerType::HourMinuteAndSecond
        );

        let inferred_time: Binding<Time> = binding(Time::constant(8, 30, 0, 0));
        assert_eq!(
            DatePicker::new("Inferred", &inferred_time)
                .seconds()
                .config()
                .ty(),
            DatePickerType::HourMinuteAndSecond
        );

        let dt = Binding::container(Date::constant(2025, 6, 15).at(8, 30, 0, 0));
        assert_eq!(
            DatePicker::new("DT", &dt).seconds().config().ty(),
            DatePickerType::DateHourMinuteAndSecond
        );
        assert_eq!(
            DatePicker::new("DT", &dt).date_only().config().ty(),
            DatePickerType::Date
        );
        assert_eq!(
            DatePicker::new("DT", &dt)
                .time_only()
                .seconds()
                .config()
                .ty(),
            DatePickerType::HourMinuteAndSecond
        );
    }

    #[test]
    fn reactive_range_reclamps_bound_date_in_place() {
        let start = Binding::container(Date::constant(2025, 6, 10));
        let end = Binding::container(Date::constant(2025, 6, 12));
        let far_future = Date::constant(2025, 12, 31);
        let cfg = DatePicker::new("End", &end)
            .range(start.map(move |s| s..=far_future))
            .config();

        let _guard = cfg.watch_range_clamp();
        assert_eq!(end.snapshot(), Date::constant(2025, 6, 12));

        start.set(Date::constant(2025, 6, 20));
        assert_eq!(end.snapshot(), Date::constant(2025, 6, 20));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a date picker range must have start <= end")
    )]
    fn reversed_date_picker_range_clamps_in_release() {
        let date = Binding::container(Date::constant(2025, 6, 15));
        let cfg = DatePicker::new("Due", &date)
            .range(Date::constant(2025, 6, 20)..=Date::constant(2025, 6, 10))
            .config();
        assert_eq!(date.snapshot(), Date::constant(2025, 6, 20));
        assert_eq!(
            cfg.range().snapshot().start().date(),
            Date::constant(2025, 6, 20)
        );
    }
}
