//! Locale-aware relative time formatting (e.g., `"in 3 days"`, `"2 hours ago"`).

use core::fmt;

use fixed_decimal::Decimal;
use icu_experimental::relativetime::{
    RelativeTimeFormatter, RelativeTimeFormatterOptions, options::Numeric,
};
use jiff::{SignedDuration, Timestamp, Zoned, civil::Date};
use nami::impl_constant;

use crate::format::LocalizedDisplay;
use crate::locale::{Locale, locales};

/// Calendar unit for relative time formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RelativeTimeUnit {
    /// Seconds (e.g., `"in 5 seconds"`, `"10 seconds ago"`).
    Second,
    /// Minutes (e.g., `"in 3 minutes"`, `"5 minutes ago"`).
    Minute,
    /// Hours (e.g., `"in 2 hours"`, `"1 hour ago"`).
    Hour,
    /// Days (e.g., `"in 3 days"`, `"yesterday"`).
    Day,
    /// Weeks (e.g., `"in 2 weeks"`, `"last week"`).
    Week,
    /// Months (e.g., `"in 4 months"`, `"2 months ago"`).
    Month,
    /// Quarters (e.g., `"in 1 quarter"`, `"2 quarters ago"`).
    Quarter,
    /// Years (e.g., `"in 2 years"`, `"last year"`).
    Year,
}

/// Width style for relative time formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RelativeTimeStyle {
    /// Full unit names: `"in 3 days"`, `"2 hours ago"`.
    #[default]
    Long,
    /// Abbreviated unit names: `"in 3 days"`, `"2 hr. ago"`.
    Short,
    /// Compact unit names: `"in 3d"`, `"2h ago"`.
    Narrow,
}

/// Controls whether special non-numeric phrases like `"yesterday"` or `"tomorrow"` are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RelativeTimeNumeric {
    /// Always format numerically (`"in 1 day"`, `"1 day ago"`).
    #[default]
    Always,
    /// Use locale-specific words when available (`"tomorrow"`, `"yesterday"`, `"today"`).
    Auto,
}

impl From<RelativeTimeNumeric> for Numeric {
    fn from(value: RelativeTimeNumeric) -> Self {
        match value {
            RelativeTimeNumeric::Always => Self::Always,
            RelativeTimeNumeric::Auto => Self::Auto,
        }
    }
}

/// A signed relative time offset formatted according to the active locale.
///
/// Positive values represent future times (`"in 3 days"`), negative values
/// represent past times (`"2 hours ago"`). Implements [`LocalizedDisplay`] and
/// [`nami::Signal`] so it interpolates reactively in `text!`.
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{LocalizedDisplay, RelativeTime, locales};
///
/// assert_eq!(RelativeTime::days(3).to_localized_string(&locales::EN), "in 3 days");
/// assert_eq!(RelativeTime::hours(-2).to_localized_string(&locales::EN), "2 hours ago");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelativeTime {
    value: i64,
    unit: RelativeTimeUnit,
    style: RelativeTimeStyle,
    numeric: RelativeTimeNumeric,
}

const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;
const SECONDS_PER_WEEK: i64 = 7 * SECONDS_PER_DAY;
/// Nominal 30-day month threshold used when selecting a unit from a duration.
const SECONDS_PER_MONTH: i64 = 30 * SECONDS_PER_DAY;
/// Nominal 365-day year threshold used when selecting a unit from a duration.
const SECONDS_PER_YEAR: i64 = 365 * SECONDS_PER_DAY;

impl RelativeTime {
    /// Creates a relative time offset of `value` in `unit`.
    ///
    /// Positive values format in the future (`"in 3 days"`); negative values
    /// format in the past (`"3 days ago"`).
    #[must_use]
    pub const fn new(value: i64, unit: RelativeTimeUnit) -> Self {
        Self {
            value,
            unit,
            style: RelativeTimeStyle::Long,
            numeric: RelativeTimeNumeric::Always,
        }
    }

    /// Creates a relative time offset in seconds.
    #[must_use]
    pub const fn seconds(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Second)
    }

    /// Creates a relative time offset in minutes.
    #[must_use]
    pub const fn minutes(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Minute)
    }

    /// Creates a relative time offset in hours.
    #[must_use]
    pub const fn hours(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Hour)
    }

    /// Creates a relative time offset in days.
    #[must_use]
    pub const fn days(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Day)
    }

    /// Creates a relative time offset in weeks.
    #[must_use]
    pub const fn weeks(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Week)
    }

    /// Creates a relative time offset in months.
    #[must_use]
    pub const fn months(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Month)
    }

    /// Creates a relative time offset in quarters.
    #[must_use]
    pub const fn quarters(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Quarter)
    }

    /// Creates a relative time offset in years.
    #[must_use]
    pub const fn years(value: i64) -> Self {
        Self::new(value, RelativeTimeUnit::Year)
    }

    /// Selects the largest natural unit for `duration` (positive = future, negative = past).
    #[must_use]
    pub const fn from_signed_duration(duration: SignedDuration) -> Self {
        let secs = duration.as_secs();
        let abs_secs = secs.unsigned_abs();
        if abs_secs >= SECONDS_PER_YEAR as u64 {
            Self::years(secs / SECONDS_PER_YEAR)
        } else if abs_secs >= SECONDS_PER_MONTH as u64 {
            Self::months(secs / SECONDS_PER_MONTH)
        } else if abs_secs >= SECONDS_PER_WEEK as u64 {
            Self::weeks(secs / SECONDS_PER_WEEK)
        } else if abs_secs >= SECONDS_PER_DAY as u64 {
            Self::days(secs / SECONDS_PER_DAY)
        } else if abs_secs >= SECONDS_PER_HOUR as u64 {
            Self::hours(secs / SECONDS_PER_HOUR)
        } else if abs_secs >= SECONDS_PER_MINUTE as u64 {
            Self::minutes(secs / SECONDS_PER_MINUTE)
        } else {
            Self::seconds(secs)
        }
    }

    /// Computes the relative offset of `target` relative to `reference`
    /// (`target - reference`: future targets are positive).
    #[must_use]
    pub fn between(target: Timestamp, reference: Timestamp) -> Self {
        Self::from_signed_duration(target.duration_since(reference))
    }

    /// Computes the relative offset of `target` relative to `reference` in
    /// whole-day or larger units (`Day`, `Week`, `Month`, `Year`).
    #[must_use]
    pub fn between_dates(target: Date, reference: Date) -> Self {
        let days = target.duration_since(reference).as_secs() / SECONDS_PER_DAY;
        let abs_days = days.unsigned_abs();
        if abs_days >= 365 {
            Self::years(days / 365)
        } else if abs_days >= 30 {
            Self::months(days / 30)
        } else if abs_days >= 7 {
            Self::weeks(days / 7)
        } else {
            Self::days(days)
        }
    }

    /// Computes the relative offset of `target` relative to `reference`.
    #[must_use]
    pub fn between_zoned(target: &Zoned, reference: &Zoned) -> Self {
        Self::from_signed_duration(target.duration_since(reference))
    }

    /// Sets the formatting width style.
    #[must_use]
    pub const fn style(mut self, style: RelativeTimeStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets whether numeric formatting is always used or special words like `"tomorrow"` are allowed.
    #[must_use]
    pub const fn numeric(mut self, numeric: RelativeTimeNumeric) -> Self {
        self.numeric = numeric;
        self
    }

    /// Returns the signed offset value.
    #[must_use]
    pub const fn value(&self) -> i64 {
        self.value
    }

    /// Returns the unit.
    #[must_use]
    pub const fn unit(&self) -> RelativeTimeUnit {
        self.unit
    }
}

macro_rules! relative_formatter_with_fallback {
    ($ctor:ident, $locale:expr, $options:expr) => {{
        RelativeTimeFormatter::$ctor($locale.0.clone().into(), $options)
            .or_else(|_| RelativeTimeFormatter::$ctor(locales::EN.0.clone().into(), $options))
            .expect("compiled English ICU relative time data is present")
    }};
}

fn build_formatter(
    locale: &Locale,
    unit: RelativeTimeUnit,
    style: RelativeTimeStyle,
    numeric: RelativeTimeNumeric,
) -> RelativeTimeFormatter {
    let options = RelativeTimeFormatterOptions::default().with_numeric(numeric.into());
    match (style, unit) {
        (RelativeTimeStyle::Long, RelativeTimeUnit::Second) => {
            relative_formatter_with_fallback!(try_new_long_second, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Minute) => {
            relative_formatter_with_fallback!(try_new_long_minute, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Hour) => {
            relative_formatter_with_fallback!(try_new_long_hour, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Day) => {
            relative_formatter_with_fallback!(try_new_long_day, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Week) => {
            relative_formatter_with_fallback!(try_new_long_week, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Month) => {
            relative_formatter_with_fallback!(try_new_long_month, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Quarter) => {
            relative_formatter_with_fallback!(try_new_long_quarter, locale, options)
        }
        (RelativeTimeStyle::Long, RelativeTimeUnit::Year) => {
            relative_formatter_with_fallback!(try_new_long_year, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Second) => {
            relative_formatter_with_fallback!(try_new_short_second, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Minute) => {
            relative_formatter_with_fallback!(try_new_short_minute, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Hour) => {
            relative_formatter_with_fallback!(try_new_short_hour, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Day) => {
            relative_formatter_with_fallback!(try_new_short_day, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Week) => {
            relative_formatter_with_fallback!(try_new_short_week, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Month) => {
            relative_formatter_with_fallback!(try_new_short_month, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Quarter) => {
            relative_formatter_with_fallback!(try_new_short_quarter, locale, options)
        }
        (RelativeTimeStyle::Short, RelativeTimeUnit::Year) => {
            relative_formatter_with_fallback!(try_new_short_year, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Second) => {
            relative_formatter_with_fallback!(try_new_narrow_second, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Minute) => {
            relative_formatter_with_fallback!(try_new_narrow_minute, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Hour) => {
            relative_formatter_with_fallback!(try_new_narrow_hour, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Day) => {
            relative_formatter_with_fallback!(try_new_narrow_day, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Week) => {
            relative_formatter_with_fallback!(try_new_narrow_week, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Month) => {
            relative_formatter_with_fallback!(try_new_narrow_month, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Quarter) => {
            relative_formatter_with_fallback!(try_new_narrow_quarter, locale, options)
        }
        (RelativeTimeStyle::Narrow, RelativeTimeUnit::Year) => {
            relative_formatter_with_fallback!(try_new_narrow_year, locale, options)
        }
    }
}

impl LocalizedDisplay for RelativeTime {
    fn fmt(&self, locale: &Locale, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let formatter = build_formatter(locale, self.unit, self.style, self.numeric);
        write!(f, "{}", formatter.format(Decimal::from(self.value)))
    }
}

impl_constant!(RelativeTime);

/// Format a relative time according to locale conventions.
#[must_use]
pub fn format_relative_time(locale: &Locale, relative: RelativeTime) -> String {
    relative.to_localized_string(locale)
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, civil};

    use super::*;
    use crate::locale::locales;

    #[test]
    fn formats_relative_time_across_locales_including_cjk() {
        let in_three_days = RelativeTime::days(3);
        assert_eq!(
            format_relative_time(&locales::EN, in_three_days),
            "in 3 days"
        );
        assert_eq!(
            format_relative_time(&locales::DE, in_three_days),
            "in 3 Tagen"
        );
        assert_eq!(
            format_relative_time(&locales::FR, in_three_days),
            "dans 3 jours"
        );
        assert_eq!(
            format_relative_time(&locales::ZH_CN, in_three_days),
            "3天后"
        );
        assert_eq!(format_relative_time(&locales::JA, in_three_days), "3 日後");
        assert_eq!(format_relative_time(&locales::KO, in_three_days), "3일 후");

        let two_hours_ago = RelativeTime::hours(-2);
        assert_eq!(
            format_relative_time(&locales::EN, two_hours_ago),
            "2 hours ago"
        );
        assert_eq!(
            format_relative_time(&locales::DE, two_hours_ago),
            "vor 2 Stunden"
        );
        assert_eq!(
            format_relative_time(&locales::ZH_CN, two_hours_ago),
            "2小时前"
        );
        assert_eq!(
            format_relative_time(&locales::JA, two_hours_ago),
            "2 時間前"
        );
        assert_eq!(
            format_relative_time(&locales::KO, two_hours_ago),
            "2시간 전"
        );
    }

    #[test]
    fn auto_numeric_uses_locale_words_for_adjacent_days() {
        let tomorrow = RelativeTime::days(1).numeric(RelativeTimeNumeric::Auto);
        let yesterday = RelativeTime::days(-1).numeric(RelativeTimeNumeric::Auto);

        assert_eq!(tomorrow.to_localized_string(&locales::EN), "tomorrow");
        assert_eq!(yesterday.to_localized_string(&locales::EN), "yesterday");
        assert_eq!(tomorrow.to_localized_string(&locales::ZH_CN), "明天");
        assert_eq!(yesterday.to_localized_string(&locales::ZH_CN), "昨天");
        assert_eq!(tomorrow.to_localized_string(&locales::JA), "明日");
        assert_eq!(yesterday.to_localized_string(&locales::JA), "昨日");
        assert_eq!(tomorrow.to_localized_string(&locales::KO), "내일");
        assert_eq!(yesterday.to_localized_string(&locales::KO), "어제");
    }

    #[test]
    fn between_dates_and_signed_duration_pick_natural_units() {
        let today = civil::date(2026, 3, 20);
        assert_eq!(
            RelativeTime::between_dates(civil::date(2026, 3, 23), today),
            RelativeTime::days(3)
        );
        assert_eq!(
            RelativeTime::between_dates(civil::date(2026, 3, 6), today),
            RelativeTime::weeks(-2)
        );
        assert_eq!(
            RelativeTime::from_signed_duration(SignedDuration::from_hours(-5)),
            RelativeTime::hours(-5)
        );
        assert_eq!(
            RelativeTime::from_signed_duration(SignedDuration::from_mins(45)),
            RelativeTime::minutes(45)
        );
    }

    #[test]
    fn styles_produce_distinct_compact_forms() {
        let value = RelativeTime::minutes(5);
        assert_eq!(value.to_localized_string(&locales::EN), "in 5 minutes");
        assert_eq!(
            value
                .style(RelativeTimeStyle::Short)
                .to_localized_string(&locales::EN),
            "in 5 min."
        );
        assert_eq!(
            value
                .style(RelativeTimeStyle::Narrow)
                .to_localized_string(&locales::EN),
            "in 5m"
        );
    }
}
