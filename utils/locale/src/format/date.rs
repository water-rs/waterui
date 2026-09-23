//! Locale-aware date and time formatting.

use icu_calendar::{Date as IcuDate, Gregorian, Iso};
use icu_datetime::{
    DateTimeFormatter, DateTimeFormatterLoadError, NoCalendarFormatter,
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
    ToSpan,
    civil::{Date as CivilDate, DateTime as JiffDateTime, Weekday},
};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::locale::{Locale, locales};
use crate::regional::RegionalContext;

static DATE_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
static TIME_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
static DATETIME_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);

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

fn to_iso_date(date: SimpleDate) -> Option<IcuDate<Iso>> {
    IcuDate::try_new_iso(date.year, date.month, date.day).ok()
}

fn to_iso_datetime(date: SimpleDate, time: SimpleTime) -> Option<IcuDateTime<Iso>> {
    Some(IcuDateTime {
        date: to_iso_date(date)?,
        time: to_icu_time(time)?,
    })
}

fn to_icu_time(time: SimpleTime) -> Option<IcuTime> {
    IcuTime::try_new(time.hour, time.minute, time.second, 0).ok()
}

fn to_iso_civil_date(date: CivilDate) -> Option<IcuDate<Iso>> {
    IcuDate::try_new_iso(
        i32::from(date.year()),
        date.month().try_into().ok()?,
        date.day().try_into().ok()?,
    )
    .ok()
}

fn to_jiff_datetime(date: SimpleDate, time: SimpleTime) -> Option<JiffDateTime> {
    let year = i16::try_from(date.year).ok()?;
    let month = i8::try_from(date.month).ok()?;
    let day = i8::try_from(date.day).ok()?;
    let hour = i8::try_from(time.hour).ok()?;
    let minute = i8::try_from(time.minute).ok()?;
    let second = i8::try_from(time.second).ok()?;
    JiffDateTime::new(year, month, day, hour, minute, second, 0).ok()
}

fn build_regional_time_zone(
    context: &RegionalContext,
    date: SimpleDate,
    time: SimpleTime,
) -> Option<ZonedDateTime<Gregorian, TimeZoneInfo<AtTime>>> {
    let local_datetime = to_jiff_datetime(date, time)?;
    let zoned = local_datetime.in_tz(context.timezone()).ok()?;
    Some((&zoned).into())
}

macro_rules! formatter_with_fallback {
    ($locale:expr, $field_set:expr, $target:ty) => {{
        let field_set = $field_set;
        DateTimeFormatter::try_new($locale.0.clone().into(), field_set)
            .or_else(|_| DateTimeFormatter::try_new(locales::EN.0.clone().into(), field_set))
            .map(|formatter| formatter.cast_into_fset::<$target>())
    }};
}

macro_rules! time_formatter_with_fallback {
    ($locale:expr, $field_set:expr) => {{
        let field_set = $field_set;
        NoCalendarFormatter::try_new($locale.0.clone().into(), field_set)
            .or_else(|_| NoCalendarFormatter::try_new(locales::EN.0.clone().into(), field_set))
            .map(|formatter| formatter.cast_into_fset::<TimeFieldSet>())
    }};
}

fn date_formatter(
    locale: &Locale,
    style: DateStyle,
) -> Result<DateTimeFormatter<DateFieldSet>, DateTimeFormatterLoadError> {
    match style {
        DateStyle::Short => formatter_with_fallback!(locale, YMD::short(), DateFieldSet),
        DateStyle::Medium => formatter_with_fallback!(locale, YMD::medium(), DateFieldSet),
        DateStyle::Long => formatter_with_fallback!(locale, YMD::long(), DateFieldSet),
        DateStyle::Full => formatter_with_fallback!(locale, YMDE::long(), DateFieldSet),
    }
}

fn calendar_month_formatter(
    locale: &Locale,
) -> Result<DateTimeFormatter<CalendarPeriodFieldSet>, DateTimeFormatterLoadError> {
    formatter_with_fallback!(locale, YM::long(), CalendarPeriodFieldSet)
}

fn calendar_weekday_formatter(
    locale: &Locale,
) -> Result<DateTimeFormatter<DateFieldSet>, DateTimeFormatterLoadError> {
    formatter_with_fallback!(locale, E::short(), DateFieldSet)
}

fn time_formatter(
    locale: &Locale,
    style: TimeStyle,
) -> Result<NoCalendarFormatter<TimeFieldSet>, DateTimeFormatterLoadError> {
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
) -> Result<DateTimeFormatter<DateAndTimeFieldSet>, DateTimeFormatterLoadError> {
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
) -> Result<DateTimeFormatter<ZonedDateAndTimeFieldSet>, DateTimeFormatterLoadError> {
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

fn fallback_date_string(date: SimpleDate) -> String {
    format!("{}-{:02}-{:02}", date.year, date.month, date.day)
}

fn fallback_time_string(time: SimpleTime, with_seconds: bool) -> String {
    if with_seconds {
        format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second)
    } else {
        format!("{:02}:{:02}", time.hour, time.minute)
    }
}

fn report_fallback_once(flag: &AtomicBool, kind: &str, locale: &Locale, reason: &str) {
    if flag
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        tracing::warn!(
            kind,
            locale = locale.canonical_tag(),
            reason,
            "waterui-locale formatting fallback"
        );
    }
}

/// Format a calendar month header using locale conventions.
#[must_use]
pub fn format_calendar_month_year(locale: &Locale, date: &CivilDate) -> String {
    let Some(date_iso) = to_iso_civil_date(*date) else {
        report_fallback_once(
            &DATE_FALLBACK_LOGGED,
            "calendar-month-year",
            locale,
            "invalid calendar month/year date",
        );
        return format!("{}-{:02}", date.year(), date.month());
    };

    match calendar_month_formatter(locale) {
        Ok(formatter) => formatter.format(&date_iso).to_string(),
        Err(err) => {
            report_fallback_once(
                &DATE_FALLBACK_LOGGED,
                "calendar-month-year",
                locale,
                &format!("ICU month/year formatter init failed: {err}"),
            );
            format!("{}-{:02}", date.year(), date.month())
        }
    }
}

/// Format a short localized weekday label for calendar headers.
///
/// # Panics
///
/// Panics if the fixed probe date `2024-01-01` becomes invalid.
#[must_use]
pub fn format_calendar_weekday(locale: &Locale, weekday: Weekday) -> String {
    let probe_date = CivilDate::new(2024, 1, 1).expect("calendar weekday probe date must be valid")
        + i32::from(weekday.to_monday_zero_offset()).days();
    let Some(date_iso) = to_iso_civil_date(probe_date) else {
        report_fallback_once(
            &DATE_FALLBACK_LOGGED,
            "calendar-weekday",
            locale,
            "invalid calendar weekday probe date",
        );
        return weekday.to_monday_one_offset().to_string();
    };

    match calendar_weekday_formatter(locale) {
        Ok(formatter) => formatter.format(&date_iso).to_string(),
        Err(err) => {
            report_fallback_once(
                &DATE_FALLBACK_LOGGED,
                "calendar-weekday",
                locale,
                &format!("ICU weekday formatter init failed: {err}"),
            );
            weekday.to_monday_one_offset().to_string()
        }
    }
}

/// Format a date according to locale conventions.
#[must_use]
pub fn format_date(locale: &Locale, date: &SimpleDate, style: DateStyle) -> String {
    let Some(date_iso) = to_iso_date(*date) else {
        report_fallback_once(
            &DATE_FALLBACK_LOGGED,
            "date",
            locale,
            "invalid date components",
        );
        return fallback_date_string(*date);
    };

    match date_formatter(locale, style) {
        Ok(formatter) => formatter.format(&date_iso).to_string(),
        Err(err) => {
            report_fallback_once(
                &DATE_FALLBACK_LOGGED,
                "date",
                locale,
                &format!("ICU formatter init failed: {err}"),
            );
            fallback_date_string(*date)
        }
    }
}

/// Format a time according to locale conventions.
#[must_use]
pub fn format_time(locale: &Locale, time: &SimpleTime, style: TimeStyle) -> String {
    let Some(time_iso) = to_icu_time(*time) else {
        report_fallback_once(
            &TIME_FALLBACK_LOGGED,
            "time",
            locale,
            "invalid time components",
        );
        let with_seconds = !matches!(style, TimeStyle::Short);
        return fallback_time_string(*time, with_seconds);
    };

    match time_formatter(locale, style) {
        Ok(formatter) => formatter.format(&time_iso).to_string(),
        Err(err) => {
            report_fallback_once(
                &TIME_FALLBACK_LOGGED,
                "time",
                locale,
                &format!("ICU formatter init failed: {err}"),
            );
            let with_seconds = !matches!(style, TimeStyle::Short);
            fallback_time_string(*time, with_seconds)
        }
    }
}

/// Format a date and time according to locale conventions.
#[must_use]
pub fn format_datetime(
    locale: &Locale,
    date: &SimpleDate,
    time: &SimpleTime,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> String {
    let Some(datetime_iso) = to_iso_datetime(*date, *time) else {
        report_fallback_once(
            &DATETIME_FALLBACK_LOGGED,
            "datetime",
            locale,
            "invalid date/time components",
        );
        let date_fallback = fallback_date_string(*date);
        let time_fallback = fallback_time_string(*time, !matches!(time_style, TimeStyle::Short));
        return format!("{date_fallback} {time_fallback}");
    };

    match datetime_formatter(locale, date_style, time_style) {
        Ok(formatter) => formatter.format(&datetime_iso).to_string(),
        Err(err) => {
            report_fallback_once(
                &DATETIME_FALLBACK_LOGGED,
                "datetime",
                locale,
                &format!("ICU formatter init failed: {err}"),
            );
            let date_fallback = format_date(locale, date, date_style);
            let time_fallback = format_time(locale, time, time_style);
            format!("{date_fallback} {time_fallback}")
        }
    }
}

/// Format a date and time using regional timezone + language settings.
///
/// This API consumes runtime regional settings (locale, preferred language order,
/// region, and timezone) and formats the datetime with timezone awareness.
#[must_use]
pub fn format_datetime_with_regional_context(
    context: &RegionalContext,
    date: &SimpleDate,
    time: &SimpleTime,
    date_style: DateStyle,
    time_style: TimeStyle,
) -> String {
    let locale = context.locale();

    let Some(zoned_datetime) = build_regional_time_zone(context, *date, *time) else {
        report_fallback_once(
            &DATETIME_FALLBACK_LOGGED,
            "datetime",
            locale,
            "failed to resolve timezone context",
        );
        return format_datetime(locale, date, time, date_style, time_style);
    };

    match zoned_datetime_formatter(locale, date_style, time_style) {
        Ok(formatter) => formatter.format(&zoned_datetime).to_string(),
        Err(err) => {
            report_fallback_once(
                &DATETIME_FALLBACK_LOGGED,
                "datetime",
                locale,
                &format!("ICU zoned formatter init failed: {err}"),
            );
            format_datetime(locale, date, time, date_style, time_style)
        }
    }
}
