//! Locale-aware date and time formatting.

use icu_calendar::{Date, DateTime};
use icu_datetime::{
    DateFormatter, DateTimeFormatter, TimeFormatter,
    options::length::{self, Date as IcuDateStyle, Time as IcuTimeStyle},
};
use icu_provider::DataLocale;

use crate::locale::{Locale, locales};

/// Date formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateStyle {
    /// Short date: "12/31/2023" or "31.12.2023"
    Short,
    /// Medium date: "Dec 31, 2023" or "31. Dez. 2023"
    #[default]
    Medium,
    /// Long date: "December 31, 2023" or "31. Dezember 2023"
    Long,
    /// Full date: "Sunday, December 31, 2023"
    Full,
}

/// Time formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeStyle {
    /// Short time: "3:45 PM" or "15:45"
    #[default]
    Short,
    /// Medium time: "3:45:30 PM" or "15:45:30"
    Medium,
    /// Long time: "3:45:30 PM EST"
    Long,
    /// Full time: "3:45:30 PM Eastern Standard Time"
    Full,
}

/// A simple date structure for formatting.
#[derive(Debug, Clone, Copy)]
pub struct SimpleDate {
    /// Year (e.g., 2023)
    pub year: i32,
    /// Month (1-12)
    pub month: u8,
    /// Day (1-31)
    pub day: u8,
}

impl SimpleDate {
    /// Create a new date.
    #[must_use]
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }
}

/// A simple time structure for formatting.
#[derive(Debug, Clone, Copy)]
pub struct SimpleTime {
    /// Hour (0-23)
    pub hour: u8,
    /// Minute (0-59)
    pub minute: u8,
    /// Second (0-59)
    pub second: u8,
}

impl SimpleTime {
    /// Create a new time.
    #[must_use]
    pub const fn new(hour: u8, minute: u8, second: u8) -> Self {
        Self {
            hour,
            minute,
            second,
        }
    }
}

fn map_date_style(style: DateStyle) -> IcuDateStyle {
    match style {
        DateStyle::Short => IcuDateStyle::Short,
        DateStyle::Medium => IcuDateStyle::Medium,
        DateStyle::Long => IcuDateStyle::Long,
        DateStyle::Full => IcuDateStyle::Full,
    }
}

fn map_time_style(style: TimeStyle) -> IcuTimeStyle {
    match style {
        TimeStyle::Short => IcuTimeStyle::Short,
        TimeStyle::Medium => IcuTimeStyle::Medium,
        TimeStyle::Long => IcuTimeStyle::Long,
        TimeStyle::Full => IcuTimeStyle::Full,
    }
}

fn to_data_locale(locale: &Locale) -> DataLocale {
    locale.0.clone().into()
}

fn to_iso_date(date: &SimpleDate) -> Option<Date<icu_calendar::Iso>> {
    Date::try_new_iso_date(date.year, date.month, date.day).ok()
}

fn to_iso_datetime(date: &SimpleDate, time: &SimpleTime) -> Option<DateTime<icu_calendar::Iso>> {
    DateTime::try_new_iso_datetime(
        date.year,
        date.month,
        date.day,
        time.hour,
        time.minute,
        time.second,
    )
    .ok()
}

fn to_iso_time_only(time: &SimpleTime) -> Option<DateTime<icu_calendar::Iso>> {
    // Synthetic stable date for time-only formatting.
    DateTime::try_new_iso_datetime(2000, 1, 1, time.hour, time.minute, time.second).ok()
}

fn fallback_date_string(date: &SimpleDate) -> String {
    format!("{}-{:02}-{:02}", date.year, date.month, date.day)
}

fn fallback_time_string(time: &SimpleTime, with_seconds: bool) -> String {
    if with_seconds {
        format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second)
    } else {
        format!("{:02}:{:02}", time.hour, time.minute)
    }
}

/// Format a date according to locale conventions.
pub fn format_date(locale: &Locale, date: &SimpleDate, style: DateStyle) -> String {
    let Some(date_iso) = to_iso_date(date) else {
        return fallback_date_string(date);
    };

    let length = map_date_style(style);
    let data_locale = to_data_locale(locale);

    let formatter = DateFormatter::try_new_with_length(&data_locale, length)
        .or_else(|_| DateFormatter::try_new_with_length(&to_data_locale(&locales::EN), length));

    match formatter {
        Ok(formatter) => formatter
            .format(&date_iso.to_any())
            .map(|formatted| formatted.to_string())
            .unwrap_or_else(|_| fallback_date_string(date)),
        Err(_) => fallback_date_string(date),
    }
}

/// Format a time according to locale conventions.
pub fn format_time(locale: &Locale, time: &SimpleTime, style: TimeStyle) -> String {
    let Some(time_iso) = to_iso_time_only(time) else {
        let with_seconds = !matches!(style, TimeStyle::Short);
        return fallback_time_string(time, with_seconds);
    };

    let length = map_time_style(style);
    let data_locale = to_data_locale(locale);

    let formatter = TimeFormatter::try_new_with_length(&data_locale, length)
        .or_else(|_| TimeFormatter::try_new_with_length(&to_data_locale(&locales::EN), length));

    match formatter {
        Ok(formatter) => formatter.format(&time_iso).to_string(),
        Err(_) => {
            let with_seconds = !matches!(style, TimeStyle::Short);
            fallback_time_string(time, with_seconds)
        }
    }
}

/// Format a date and time according to locale conventions.
pub fn format_datetime(
    locale: &Locale,
    date: &SimpleDate,
    time: &SimpleTime,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> String {
    let Some(datetime_iso) = to_iso_datetime(date, time) else {
        let date_fallback = fallback_date_string(date);
        let time_fallback = fallback_time_string(time, !matches!(time_style, TimeStyle::Short));
        return format!("{date_fallback} {time_fallback}");
    };

    let options =
        length::Bag::from_date_time_style(map_date_style(date_style), map_time_style(time_style));
    let data_locale = to_data_locale(locale);

    let formatter =
        DateTimeFormatter::try_new(&data_locale, options.clone().into()).or_else(|_| {
            DateTimeFormatter::try_new(&to_data_locale(&locales::EN), options.clone().into())
        });

    match formatter {
        Ok(formatter) => formatter
            .format(&datetime_iso.to_any())
            .map(|formatted| formatted.to_string())
            .unwrap_or_else(|_| {
                let date_fallback = format_date(locale, date, date_style);
                let time_fallback = format_time(locale, time, time_style);
                format!("{date_fallback} {time_fallback}")
            }),
        Err(_) => {
            let date_fallback = format_date(locale, date, date_style);
            let time_fallback = format_time(locale, time, time_style);
            format!("{date_fallback} {time_fallback}")
        }
    }
}
