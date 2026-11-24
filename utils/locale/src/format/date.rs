//! Locale-aware date and time formatting over `jiff` values.

use core::fmt;

use icu_calendar::{Date as IcuDate, Gregorian};
use icu_datetime::{
    DateTimeFormatter, NoCalendarFormatter,
    fieldsets::{
        E, T, YM, YMD, YMDE,
        enums::{
            CalendarPeriodFieldSet, DateAndTimeFieldSet, DateFieldSet, TimeFieldSet,
            ZonedDateAndTimeFieldSet,
        },
        zone,
    },
};
use icu_time::{
    DateTime as IcuDateTime, Time as IcuTime, TimeZoneInfo, ZonedDateTime, zone::models::AtTime,
};
use jiff::{
    ToSpan, Zoned,
    civil::{Date, DateTime, Time, Weekday},
};
use nami::impl_constant;

use crate::format::LocalizedDisplay;
use crate::locale::{Locale, locales};
use crate::regional::RegionalContext;

/// Date formatting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
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
#[non_exhaustive]
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

/// Locale-aware display wrapper for a [`jiff::civil::Date`].
///
/// Implements [`LocalizedDisplay`] and [`nami::Signal`] so `text!` re-formats
/// the date whenever either the date signal or the active locale changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormattedDate {
    date: Date,
    style: DateStyle,
}

impl FormattedDate {
    /// Wraps `date` with `style` for locale-aware display.
    #[must_use]
    pub const fn new(date: Date, style: DateStyle) -> Self {
        Self { date, style }
    }

    /// Returns the wrapped date.
    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    /// Returns the formatting style.
    #[must_use]
    pub const fn style(&self) -> DateStyle {
        self.style
    }
}

impl LocalizedDisplay for FormattedDate {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icu_date = IcuDate::<Gregorian>::from(self.date);
        let formatter = date_formatter(locale, self.style);
        write!(f, "{}", formatter.format(&icu_date))
    }
}

/// Locale-aware display wrapper for a [`jiff::civil::Time`].
///
/// Implements [`LocalizedDisplay`] and [`nami::Signal`] so `text!` re-formats
/// the time whenever either the time signal or the active locale changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormattedTime {
    time: Time,
    style: TimeStyle,
}

impl FormattedTime {
    /// Wraps `time` with `style` for locale-aware display.
    #[must_use]
    pub const fn new(time: Time, style: TimeStyle) -> Self {
        Self { time, style }
    }

    /// Returns the wrapped time.
    #[must_use]
    pub const fn time(&self) -> Time {
        self.time
    }

    /// Returns the formatting style.
    #[must_use]
    pub const fn style(&self) -> TimeStyle {
        self.style
    }
}

impl LocalizedDisplay for FormattedTime {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icu_time = IcuTime::from(self.time);
        let formatter = time_formatter(locale, self.style);
        write!(f, "{}", formatter.format(&icu_time))
    }
}

/// Locale-aware display wrapper for a [`jiff::civil::DateTime`].
///
/// Implements [`LocalizedDisplay`] and [`nami::Signal`] so `text!` re-formats
/// the date-time whenever either the value signal or the active locale changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormattedDateTime {
    datetime: DateTime,
    date_style: DateStyle,
    time_style: TimeStyle,
}

impl FormattedDateTime {
    /// Wraps `datetime` with `date_style` and `time_style` for locale-aware display.
    #[must_use]
    pub const fn new(datetime: DateTime, date_style: DateStyle, time_style: TimeStyle) -> Self {
        Self {
            datetime,
            date_style,
            time_style,
        }
    }

    /// Returns the wrapped civil date-time.
    #[must_use]
    pub const fn datetime(&self) -> DateTime {
        self.datetime
    }

    /// Returns the date formatting style.
    #[must_use]
    pub const fn date_style(&self) -> DateStyle {
        self.date_style
    }

    /// Returns the time formatting style.
    #[must_use]
    pub const fn time_style(&self) -> TimeStyle {
        self.time_style
    }
}

impl LocalizedDisplay for FormattedDateTime {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icu_datetime = IcuDateTime::<Gregorian>::from(self.datetime);
        let formatter = datetime_formatter(locale, self.date_style, self.time_style);
        write!(f, "{}", formatter.format(&icu_datetime))
    }
}

/// Locale-aware display wrapper for a [`jiff::Zoned`] timestamp.
///
/// Implements [`LocalizedDisplay`] and [`nami::Signal`] so `text!` re-formats
/// the zoned date-time whenever either the value signal or the active locale
/// changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedZoned {
    zoned: Zoned,
    date_style: DateStyle,
    time_style: TimeStyle,
}

impl FormattedZoned {
    /// Wraps `zoned` with `date_style` and `time_style` for locale-aware display.
    #[must_use]
    pub const fn new(zoned: Zoned, date_style: DateStyle, time_style: TimeStyle) -> Self {
        Self {
            zoned,
            date_style,
            time_style,
        }
    }

    /// Returns the wrapped zoned date-time.
    #[must_use]
    pub const fn zoned(&self) -> &Zoned {
        &self.zoned
    }

    /// Returns the date formatting style.
    #[must_use]
    pub const fn date_style(&self) -> DateStyle {
        self.date_style
    }

    /// Returns the time formatting style.
    #[must_use]
    pub const fn time_style(&self) -> TimeStyle {
        self.time_style
    }
}

impl LocalizedDisplay for FormattedZoned {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icu_zoned = ZonedDateTime::<Gregorian, TimeZoneInfo<AtTime>>::from(&self.zoned);
        let formatter = zoned_datetime_formatter(locale, self.date_style, self.time_style);
        write!(f, "{}", formatter.format(&icu_zoned))
    }
}

impl_constant!(
    FormattedDate,
    FormattedTime,
    FormattedDateTime,
    FormattedZoned
);

/// Extension trait for wrapping `jiff` date and time values in a locale-aware
/// [`LocalizedDisplay`] adapter that interpolates reactively in `text!`.
///
/// # Examples
///
/// ```rust
/// use jiff::civil;
/// use waterui_locale::{DateStyle, LocalizedDateTimeExt, LocalizedDisplay, locales};
///
/// let due = civil::date(2026, 3, 20).formatted(DateStyle::Long);
/// assert_eq!(due.to_localized_string(&locales::EN), "March 20, 2026");
/// assert_eq!(due.to_localized_string(&locales::JA), "2026年3月20日");
/// ```
pub trait LocalizedDateTimeExt {
    /// Style accepted by [`Self::formatted`].
    type Style;
    /// Locale-aware wrapper produced by [`Self::formatted`].
    type Output: LocalizedDisplay;

    /// Wraps `self` in a locale-aware formatter.
    #[must_use]
    fn formatted(&self, style: Self::Style) -> Self::Output;
}

impl LocalizedDateTimeExt for Date {
    type Style = DateStyle;
    type Output = FormattedDate;

    fn formatted(&self, style: DateStyle) -> FormattedDate {
        FormattedDate::new(*self, style)
    }
}

impl LocalizedDateTimeExt for Time {
    type Style = TimeStyle;
    type Output = FormattedTime;

    fn formatted(&self, style: TimeStyle) -> FormattedTime {
        FormattedTime::new(*self, style)
    }
}

impl LocalizedDateTimeExt for DateTime {
    type Style = (DateStyle, TimeStyle);
    type Output = FormattedDateTime;

    fn formatted(&self, (date_style, time_style): (DateStyle, TimeStyle)) -> FormattedDateTime {
        FormattedDateTime::new(*self, date_style, time_style)
    }
}

impl LocalizedDateTimeExt for Zoned {
    type Style = (DateStyle, TimeStyle);
    type Output = FormattedZoned;

    fn formatted(&self, (date_style, time_style): (DateStyle, TimeStyle)) -> FormattedZoned {
        FormattedZoned::new(self.clone(), date_style, time_style)
    }
}

macro_rules! formatter_with_fallback {
    ($locale:expr, $field_set:expr, $target:ty) => {{
        let field_set = $field_set;
        DateTimeFormatter::try_new($locale.0.clone().into(), field_set)
            .or_else(|_| DateTimeFormatter::try_new(locales::EN.0.clone().into(), field_set))
            .expect("compiled English ICU datetime data is present")
            .cast_into_fset::<$target>()
    }};
}

macro_rules! time_formatter_with_fallback {
    ($locale:expr, $field_set:expr) => {{
        let field_set = $field_set;
        NoCalendarFormatter::try_new($locale.0.clone().into(), field_set)
            .or_else(|_| NoCalendarFormatter::try_new(locales::EN.0.clone().into(), field_set))
            .expect("compiled English ICU time data is present")
            .cast_into_fset::<TimeFieldSet>()
    }};
}

fn date_formatter(locale: &Locale, style: DateStyle) -> DateTimeFormatter<DateFieldSet> {
    match style {
        DateStyle::Short => formatter_with_fallback!(locale, YMD::short(), DateFieldSet),
        DateStyle::Medium => formatter_with_fallback!(locale, YMD::medium(), DateFieldSet),
        DateStyle::Long => formatter_with_fallback!(locale, YMD::long(), DateFieldSet),
        DateStyle::Full => formatter_with_fallback!(locale, YMDE::long(), DateFieldSet),
    }
}

fn calendar_month_formatter(locale: &Locale) -> DateTimeFormatter<CalendarPeriodFieldSet> {
    formatter_with_fallback!(locale, YM::long(), CalendarPeriodFieldSet)
}

fn calendar_weekday_formatter(locale: &Locale) -> DateTimeFormatter<DateFieldSet> {
    formatter_with_fallback!(locale, E::short(), DateFieldSet)
}

fn time_formatter(locale: &Locale, style: TimeStyle) -> NoCalendarFormatter<TimeFieldSet> {
    match style {
        TimeStyle::Short => time_formatter_with_fallback!(locale, T::hm()),
        TimeStyle::Medium | TimeStyle::Long | TimeStyle::Full => {
            time_formatter_with_fallback!(locale, T::hms())
        }
    }
}

fn datetime_formatter(
    locale: &Locale,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> DateTimeFormatter<DateAndTimeFieldSet> {
    macro_rules! with_time_style {
        ($field_set:expr) => {
            match time_style {
                TimeStyle::Short => {
                    formatter_with_fallback!(locale, $field_set.with_time_hm(), DateAndTimeFieldSet)
                }
                TimeStyle::Medium | TimeStyle::Long | TimeStyle::Full => formatter_with_fallback!(
                    locale,
                    $field_set.with_time_hms(),
                    DateAndTimeFieldSet
                ),
            }
        };
    }

    match date_style {
        DateStyle::Short => with_time_style!(YMD::short()),
        DateStyle::Medium => with_time_style!(YMD::medium()),
        DateStyle::Long => with_time_style!(YMD::long()),
        DateStyle::Full => with_time_style!(YMDE::long()),
    }
}

fn zoned_datetime_formatter(
    locale: &Locale,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> DateTimeFormatter<ZonedDateAndTimeFieldSet> {
    macro_rules! with_time_and_zone_style {
        ($field_set:expr) => {
            match time_style {
                TimeStyle::Short => formatter_with_fallback!(
                    locale,
                    $field_set.with_time_hm().with_zone(zone::SpecificShort),
                    ZonedDateAndTimeFieldSet
                ),
                TimeStyle::Medium | TimeStyle::Long => formatter_with_fallback!(
                    locale,
                    $field_set.with_time_hms().with_zone(zone::SpecificShort),
                    ZonedDateAndTimeFieldSet
                ),
                TimeStyle::Full => formatter_with_fallback!(
                    locale,
                    $field_set.with_time_hms().with_zone(zone::SpecificLong),
                    ZonedDateAndTimeFieldSet
                ),
            }
        };
    }

    match date_style {
        DateStyle::Short => with_time_and_zone_style!(YMD::short()),
        DateStyle::Medium => with_time_and_zone_style!(YMD::medium()),
        DateStyle::Long => with_time_and_zone_style!(YMD::long()),
        DateStyle::Full => with_time_and_zone_style!(YMDE::long()),
    }
}

/// Format a calendar month header using locale conventions.
#[must_use]
pub fn format_calendar_month_year(locale: &Locale, date: &Date) -> String {
    let icu_date = IcuDate::<Gregorian>::from(*date);
    calendar_month_formatter(locale)
        .format(&icu_date)
        .to_string()
}

/// Format a short localized weekday label for calendar headers.
///
/// # Panics
///
/// Panics if the fixed probe date `2024-01-01` becomes invalid.
#[must_use]
pub fn format_calendar_weekday(locale: &Locale, weekday: Weekday) -> String {
    let probe_date = Date::new(2024, 1, 1).expect("calendar weekday probe date must be valid")
        + i32::from(weekday.to_monday_zero_offset()).days();
    let icu_date = IcuDate::<Gregorian>::from(probe_date);
    calendar_weekday_formatter(locale)
        .format(&icu_date)
        .to_string()
}

/// Format a civil date according to locale conventions.
#[must_use]
pub fn format_date(locale: &Locale, date: Date, style: DateStyle) -> String {
    FormattedDate::new(date, style).to_localized_string(locale)
}

/// Format a civil time according to locale conventions.
#[must_use]
pub fn format_time(locale: &Locale, time: Time, style: TimeStyle) -> String {
    FormattedTime::new(time, style).to_localized_string(locale)
}

/// Format a civil date and time according to locale conventions.
#[must_use]
pub fn format_datetime(
    locale: &Locale,
    datetime: DateTime,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> String {
    FormattedDateTime::new(datetime, date_style, time_style).to_localized_string(locale)
}

/// Format a zoned date and time according to locale conventions.
#[must_use]
pub fn format_zoned(
    locale: &Locale,
    zoned: &Zoned,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> String {
    let icu_zoned = ZonedDateTime::<Gregorian, TimeZoneInfo<AtTime>>::from(zoned);
    zoned_datetime_formatter(locale, date_style, time_style)
        .format(&icu_zoned)
        .to_string()
}

/// Format a civil date and time using regional timezone + language settings.
///
/// Resolves `datetime` in `context.timezone()` and formats it with the
/// timezone name; falls back to civil datetime formatting when the configured
/// timezone name cannot be resolved.
#[must_use]
pub fn format_datetime_with_regional_context(
    context: &RegionalContext,
    datetime: DateTime,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> String {
    let locale = context.locale();
    match datetime.in_tz(context.timezone()) {
        Ok(zoned) => format_zoned(locale, &zoned, date_style, time_style),
        Err(error) => {
            tracing::warn!(
                timezone = context.timezone(),
                locale = locale.canonical_tag(),
                %error,
                "failed to resolve regional timezone; formatting civil datetime without zone"
            );
            format_datetime(locale, datetime, date_style, time_style)
        }
    }
}
