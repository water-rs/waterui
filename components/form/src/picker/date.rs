//! Date picker component.

use core::ops::{RangeBounds, RangeInclusive};

use nami::Binding;
use time::Date;
use waterui_core::{AnyView, View, configurable};
/// Configuration for the `DatePicker` component.
#[derive(Debug)]
#[non_exhaustive]
pub struct DatePickerConfig {
    /// The label to display for the date picker.
    pub label: AnyView,
    /// The binding to the `Date` value.
    pub value: Binding<Date>,
    /// The range of valid dates.
    pub range: RangeInclusive<Date>,
    /// The type of date picker.
    pub ty: DatePickerType,
}

/// Enum representing the different types of date pickers.
#[derive(Debug, Default)]
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

configurable!(
    /// A control for selecting dates and times.
    ///
    /// DatePicker provides various styles for date/time selection including
    /// date-only, time-only, or combined date-time pickers.
    ///
    /// # Layout Behavior
    ///
    /// DatePicker sizes itself to fit its content and never stretches to fill extra space.
    /// In a stack, it takes only the space it needs.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Stretch Axis: `None` - DatePicker never expands to fill available space.
    // Size: Determined by picker style and content (platform-determined)
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    DatePicker,
    DatePickerConfig
);

impl DatePicker {
    /// Creates a new `DatePicker` with the given date binding.
    #[must_use]
    pub fn new(date: &Binding<Date>) -> Self {
        Self(DatePickerConfig {
            label: AnyView::default(),
            value: date.clone(),
            range: Date::MIN..=Date::MAX,
            ty: DatePickerType::default(),
        })
    }

    /// Sets the range of valid dates.
    #[must_use]
    pub fn range(mut self, range: impl RangeBounds<Date> + Clone + 'static) -> Self {
        self.0.value = self.0.value.range(range);
        self
    }

    /// Sets the label for the date picker.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }

    /// Sets the type of date picker.
    #[must_use]
    pub const fn ty(mut self, ty: DatePickerType) -> Self {
        self.0.ty = ty;
        self
    }
}
