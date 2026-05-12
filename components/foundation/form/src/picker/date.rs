//! Date picker component.

use alloc::string::{String, ToString};
use core::ops::RangeInclusive;

pub use jiff::civil::{Date, DateTime, Time};
use nami::Binding;
use waterui_controls::label::Label;
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

/// Configuration for the `DatePicker` component.
#[derive(Debug)]
#[non_exhaustive]
pub struct DatePickerConfig {
    /// The label displayed for the date picker.
    pub label: Label,
    /// The binding to the selected value.
    pub value: Binding<DateTime>,
    /// The range of valid values.
    pub range: RangeInclusive<DateTime>,
    /// The type of date picker.
    pub ty: DatePickerType,
}

/// Enum representing the different types of date pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

/// Types that can drive a [`DatePicker`].
///
/// Implemented for `Date`, `Time`, and `DateTime`. A single
/// [`DatePicker::new`] constructor dispatches on this trait, so callers do not
/// have to remember separate constructor names per binding type.
pub trait DatePickable: Clone + 'static {
    /// Maps a `Binding<Self>` into the canonical `Binding<DateTime>` that
    /// `DatePicker` stores internally, preserving round-trip semantics.
    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime>;

    /// Lifts an inclusive range of `Self` into the canonical
    /// `RangeInclusive<DateTime>` used by the picker.
    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime>;

    /// Returns the default [`DatePickerType`] when the user has not specified
    /// one explicitly.
    fn default_picker_type() -> DatePickerType;

    /// Returns the canonical full range used when the user has not constrained
    /// the picker.
    fn full_range() -> RangeInclusive<DateTime>;
}

/// A control for selecting dates and times.
///
/// `DatePicker` stores a full date-time internally so backends can round-trip
/// hidden components without loss. Use [`DatePicker::new`] with any binding
/// whose value type implements [`DatePickable`] (`Date`, `Time`, or
/// `DateTime`).
#[derive(Debug)]
pub struct DatePicker(DatePickerConfig);

impl ConfigurableView for DatePicker {
    type Config = DatePickerConfig;

    fn config(self) -> Self::Config {
        self.0
    }
}

impl ViewConfiguration for DatePickerConfig {
    type View = DatePicker;

    fn render(self) -> Self::View {
        DatePicker(self)
    }
}

impl From<DatePickerConfig> for DatePicker {
    fn from(value: DatePickerConfig) -> Self {
        Self(value)
    }
}

impl NativeView for DatePickerConfig {
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl View for DatePicker {
    fn body(self, env: &Environment) -> impl View {
        let config = self.0;
        if let Some(hook) = env.get::<Hook<DatePickerConfig>>() {
            AnyView::new(hook.apply(env, config))
        } else {
            AnyView::new(Native::new(config))
        }
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl DatePicker {
    /// Creates a picker bound to `value` with the given semantic label,
    /// dispatching on the binding's type (`Date`, `Time`, or `DateTime`) via
    /// [`DatePickable`].
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new<T: DatePickable>(label: impl IntoLabel, value: &Binding<T>) -> Self {
        Self(DatePickerConfig {
            label: label.into_label(),
            value: T::into_datetime_binding(value),
            range: T::full_range(),
            ty: T::default_picker_type(),
        })
    }

    /// Sets the valid range for the picker.
    #[must_use]
    pub fn range<T: DatePickable>(mut self, range: RangeInclusive<T>) -> Self {
        let mapped = T::into_picker_range(range);
        self.0.value = self.0.value.clamp(mapped.clone());
        self.0.range = mapped;
        self
    }

    /// Sets the type of date picker.
    #[must_use]
    pub const fn ty(mut self, ty: DatePickerType) -> Self {
        self.0.ty = ty;
        self
    }
}

impl_label_style_methods!(DatePicker);

impl DatePickable for Date {
    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime> {
        map_date_binding(binding)
    }

    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime> {
        start_of_day(*range.start())..=end_of_day(*range.end())
    }

    fn default_picker_type() -> DatePickerType {
        DatePickerType::Date
    }

    fn full_range() -> RangeInclusive<DateTime> {
        Self::into_picker_range(Self::MIN..=Self::MAX)
    }
}

impl DatePickable for Time {
    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime> {
        map_time_binding(binding)
    }

    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime> {
        anchor_time(*range.start())..=anchor_time(*range.end())
    }

    fn default_picker_type() -> DatePickerType {
        DatePickerType::HourAndMinute
    }

    fn full_range() -> RangeInclusive<DateTime> {
        Self::into_picker_range(Self::MIN..=end_of_day_time())
    }
}

impl DatePickable for DateTime {
    fn into_datetime_binding(binding: &Binding<Self>) -> Binding<DateTime> {
        binding.clone()
    }

    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<DateTime> {
        range
    }

    fn default_picker_type() -> DatePickerType {
        DatePickerType::DateHourAndMinute
    }

    fn full_range() -> RangeInclusive<DateTime> {
        full_picker_range()
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

        assert_eq!(source.get(), Time::new(18, 45, 12, 0).unwrap());
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
}
