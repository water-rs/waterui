//! Date picker component.

use core::ops::RangeInclusive;

use nami::Binding;
use waterui_controls::IntoLabel;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

// Re-export essential time types for FFI and external use.
pub use time::{Date, Month, PrimitiveDateTime, Time};

/// Configuration for the `DatePicker` component.
#[derive(Debug)]
#[non_exhaustive]
pub struct DatePickerConfig {
    /// The label to display for the date picker.
    pub label: AnyView,
    /// The binding to the selected value.
    pub value: Binding<PrimitiveDateTime>,
    /// The range of valid values.
    pub range: RangeInclusive<PrimitiveDateTime>,
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

/// Values accepted by [`DatePicker::range`].
#[doc(hidden)]
pub trait DatePickerRangeValue: Clone + 'static {
    #[doc(hidden)]
    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<PrimitiveDateTime>;
}

/// A control for selecting dates and times.
///
/// `DatePicker` stores a full date-time internally so backends can round-trip
/// hidden components without loss. Use the constructor that matches your
/// binding type:
///
/// - [`DatePicker::new`] for `Binding<Date>`
/// - [`DatePicker::time`] for `Binding<Time>`
/// - [`DatePicker::datetime`] for `Binding<PrimitiveDateTime>`
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
    /// Creates a date-only picker bound to a `Date`.
    #[must_use]
    pub fn new(date: &Binding<Date>) -> Self {
        Self(DatePickerConfig {
            label: AnyView::default(),
            value: map_date_binding(date),
            range: Date::into_picker_range(Date::MIN..=Date::MAX),
            ty: DatePickerType::Date,
        })
    }

    /// Creates a time-only picker bound to a `Time`.
    #[must_use]
    pub fn time(time: &Binding<Time>) -> Self {
        Self(DatePickerConfig {
            label: AnyView::default(),
            value: map_time_binding(time),
            range: Time::into_picker_range(Time::MIDNIGHT..=end_of_day_time()),
            ty: DatePickerType::HourAndMinute,
        })
    }

    /// Creates a date-time picker bound to a `PrimitiveDateTime`.
    #[must_use]
    pub fn datetime(value: &Binding<PrimitiveDateTime>) -> Self {
        Self(DatePickerConfig {
            label: AnyView::default(),
            value: value.clone(),
            range: PrimitiveDateTime::into_picker_range(full_picker_range()),
            ty: DatePickerType::DateHourAndMinute,
        })
    }

    /// Sets the valid range for the picker.
    #[must_use]
    pub fn range<T: DatePickerRangeValue>(mut self, range: RangeInclusive<T>) -> Self {
        let mapped = T::into_picker_range(range);
        self.0.value = self.0.value.clamp(mapped.clone());
        self.0.range = mapped;
        self
    }

    /// Sets the label for the date picker.
    #[must_use]
    pub fn label(mut self, label: impl IntoLabel) -> Self {
        self.0.label = AnyView::new(label.into_label());
        self
    }

    /// Sets the type of date picker.
    #[must_use]
    pub const fn ty(mut self, ty: DatePickerType) -> Self {
        self.0.ty = ty;
        self
    }
}

impl DatePickerRangeValue for Date {
    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<PrimitiveDateTime> {
        start_of_day(*range.start())..=end_of_day(*range.end())
    }
}

impl DatePickerRangeValue for Time {
    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<PrimitiveDateTime> {
        anchor_time(*range.start())..=anchor_time(*range.end())
    }
}

impl DatePickerRangeValue for PrimitiveDateTime {
    fn into_picker_range(range: RangeInclusive<Self>) -> RangeInclusive<PrimitiveDateTime> {
        range
    }
}

fn map_date_binding(date: &Binding<Date>) -> Binding<PrimitiveDateTime> {
    Binding::mapping(date, start_of_day, |binding, value| {
        binding.set(value.date());
    })
}

fn map_time_binding(time: &Binding<Time>) -> Binding<PrimitiveDateTime> {
    Binding::mapping(time, anchor_time, |binding, value| {
        binding.set(value.time());
    })
}

fn full_picker_range() -> RangeInclusive<PrimitiveDateTime> {
    start_of_day(Date::MIN)..=end_of_day(Date::MAX)
}

fn start_of_day(date: Date) -> PrimitiveDateTime {
    PrimitiveDateTime::new(date, Time::MIDNIGHT)
}

fn end_of_day(date: Date) -> PrimitiveDateTime {
    PrimitiveDateTime::new(date, end_of_day_time())
}

fn time_anchor_date() -> Date {
    Date::from_calendar_date(2000, Month::January, 1)
        .expect("time-only picker anchor date must be valid")
}

fn anchor_time(time: Time) -> PrimitiveDateTime {
    PrimitiveDateTime::new(time_anchor_date(), time)
}

fn end_of_day_time() -> Time {
    Time::from_hms(23, 59, 59).expect("end-of-day time must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_range_maps_full_days() {
        let start = Date::from_calendar_date(2025, Month::January, 10).unwrap();
        let end = Date::from_calendar_date(2025, Month::January, 12).unwrap();
        let mapped = Date::into_picker_range(start..=end);

        assert_eq!(mapped.start().date(), start);
        assert_eq!(mapped.start().time(), Time::MIDNIGHT);
        assert_eq!(mapped.end().date(), end);
        assert_eq!(mapped.end().time(), end_of_day_time());
    }

    #[test]
    fn time_binding_preserves_time_value() {
        let source = Binding::container(Time::from_hms(8, 30, 0).unwrap());
        let mapped = map_time_binding(&source);

        mapped.set(anchor_time(Time::from_hms(18, 45, 12).unwrap()));

        assert_eq!(source.get(), Time::from_hms(18, 45, 12).unwrap());
    }
}
