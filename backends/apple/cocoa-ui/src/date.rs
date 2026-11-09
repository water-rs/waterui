//! Calendar dates: `NSDate`, `NSDateComponents`, and `NSCalendar` as plain
//! values.
//!
//! Both frameworks model a wall-clock date the same way — a year, month and
//! day plus an optional time — so the conversions live once at the crate
//! root. The calendar is always proleptic Gregorian: the components a caller
//! produces and consumes are calendar fields, not instants, so no time zone
//! is involved.
//!
//! # Safety
//!
//! The `unsafe` here calls `objc2` bindings on `NSDate`, `NSDateComponents`
//! and `NSCalendar` — `NSDateComponents` is only mutated immediately after
//! construction, and the calendar is read-only, matching the documented
//! contracts.

use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_foundation::{
    NSCalendar, NSCalendarIdentifierGregorian, NSCalendarUnit, NSDate, NSDateComponents,
    NSDateFormatter, NSDateFormatterStyle, NSDateInterval,
};

/// The year, month and day of a calendar date.
///
/// Field widths follow the frameworks: `NSInteger` is a pointer-sized
/// integer, but calendar values never approach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateParts {
    /// The calendar year.
    pub year: i32,
    /// The month, `1..=12`.
    pub month: i32,
    /// The day of the month.
    pub day: i32,
}

/// The hour, minute and second of a wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TimeParts {
    /// The hour, `0..=23`.
    pub hour: i32,
    /// The minute, `0..=59`.
    pub minute: i32,
    /// The second, `0..=59`.
    pub second: i32,
}

/// A calendar date plus a wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTimeParts {
    /// The date fields.
    pub date: DateParts,
    /// The time fields.
    pub time: TimeParts,
}

/// The proleptic Gregorian calendar, created per call — the frameworks
/// treat calendars as read-only values, so a fresh one is cheap.
///
/// # Panics
///
/// Never in practice: the Gregorian identifier always resolves.
#[must_use]
pub fn gregorian() -> Retained<NSCalendar> {
    // SAFETY: the identifier is a framework-provided static string; passing
    // it to `calendarWithIdentifier:` is the documented way to get the
    // Gregorian calendar.
    let calendar = unsafe { NSCalendar::calendarWithIdentifier(NSCalendarIdentifierGregorian) };
    calendar.expect("the Gregorian calendar identifier always resolves")
}

/// `components` carrying `parts`' year, month and day — the shape
/// `UICalendarView` and `UICalendarSelectionMultiDate` speak.
#[must_use]
pub fn components(parts: DateParts) -> Retained<NSDateComponents> {
    let components = NSDateComponents::new();
    components.setYear(parts.year as isize);
    components.setMonth(parts.month as isize);
    components.setDay(parts.day as isize);
    components
}

/// The year, month and day `components` carries; `None` when any of the
/// three is unset — the same completeness check the frameworks perform.
#[must_use]
pub fn parts(components: &NSDateComponents) -> Option<DateParts> {
    // NSDateComponents reports NSDateComponentUndefined (NSIntegerMax) for
    // fields that were never set; the generated getters cannot express that,
    // so validity comes from `isValidDate` semantics — read the raw values
    // and reject the undefined marker.
    const UNDEFINED: isize = isize::MAX;
    let year = components.year();
    let month = components.month();
    let day = components.day();
    if year == UNDEFINED || month == UNDEFINED || day == UNDEFINED {
        return None;
    }
    Some(DateParts {
        year: i32::try_from(year).ok()?,
        month: i32::try_from(month).ok()?,
        day: i32::try_from(day).ok()?,
    })
}

/// The calendar date `date` names under the Gregorian calendar, with its
/// wall-clock time — `nil` components fatal in the caller's terms become
/// `None` here.
#[must_use]
pub fn date_time_parts(date: &NSDate) -> Option<DateTimeParts> {
    let units = NSCalendarUnit::Year
        | NSCalendarUnit::Month
        | NSCalendarUnit::Day
        | NSCalendarUnit::Hour
        | NSCalendarUnit::Minute
        | NSCalendarUnit::Second;
    let components = gregorian().components_fromDate(units, date);
    let date_parts = parts(&components)?;
    let time_parts = TimeParts {
        hour: i32::try_from(components.hour()).ok()?,
        minute: i32::try_from(components.minute()).ok()?,
        second: i32::try_from(components.second()).ok()?,
    };
    Some(DateTimeParts {
        date: date_parts,
        time: time_parts,
    })
}

/// The calendar date `date` names under the Gregorian calendar.
#[must_use]
pub fn date_parts(date: &NSDate) -> Option<DateParts> {
    date_time_parts(date).map(|parts| parts.date)
}

/// The `NSDate` `parts` names under the Gregorian calendar; `None` when the
/// fields do not resolve to a real date.
#[must_use]
pub fn ns_date(parts: &DateTimeParts) -> Option<Retained<NSDate>> {
    let components = components(parts.date);
    components.setHour(parts.time.hour as isize);
    components.setMinute(parts.time.minute as isize);
    components.setSecond(parts.time.second as isize);
    gregorian().dateFromComponents(&components)
}

/// The `NSDate` `parts` names under the Gregorian calendar at midnight;
/// `None` when the fields do not resolve to a real date.
#[must_use]
pub fn ns_date_from_date_parts(parts: DateParts) -> Option<Retained<NSDate>> {
    gregorian().dateFromComponents(&components(parts))
}

/// The half-open interval `[start, end)` an availability range takes.
#[must_use]
pub fn interval(start: &NSDate, end: &NSDate) -> Retained<NSDateInterval> {
    NSDateInterval::initWithStartDate_endDate(NSDateInterval::alloc(), start, end)

    // `alloc` is `AllocAnyThread::alloc`: `NSDateInterval` is `Send + Sync`,
    // so no main-thread proof is needed.
}

/// A calendar month: the year and the month, `1..=12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct YearMonth {
    /// The calendar year.
    pub year: i32,
    /// The month, `1..=12`.
    pub month: i32,
}

impl YearMonth {
    /// The month `day` falls in.
    #[must_use]
    pub const fn of(day: DateParts) -> Self {
        Self {
            year: day.year,
            month: day.month,
        }
    }

    /// The month's first day.
    #[must_use]
    pub const fn first_day(self) -> DateParts {
        DateParts {
            year: self.year,
            month: self.month,
            day: 1,
        }
    }

    /// The month before this one.
    #[must_use]
    pub const fn previous(self) -> Self {
        if self.month == 1 {
            Self {
                year: self.year - 1,
                month: 12,
            }
        } else {
            Self {
                year: self.year,
                month: self.month - 1,
            }
        }
    }

    /// The month after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        if self.month == 12 {
            Self {
                year: self.year + 1,
                month: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    /// The number of days in the month under the Gregorian calendar.
    ///
    /// # Panics
    ///
    /// When the month does not name a real Gregorian month.
    #[must_use]
    pub fn day_count(self) -> i32 {
        let calendar = gregorian();
        let first = calendar
            .dateFromComponents(&components(self.first_day()))
            .expect("a calendar month resolves to a date");
        let range =
            calendar.rangeOfUnit_inUnit_forDate(NSCalendarUnit::Day, NSCalendarUnit::Month, &first);
        i32::try_from(range.length).expect("a month has fewer than 2^31 days")
    }

    /// The weekday the month's first day falls on, `1` (Sunday) through `7`
    /// (Saturday) — `NSCalendar`'s numbering.
    ///
    /// # Panics
    ///
    /// When the month does not name a real Gregorian month.
    #[must_use]
    pub fn first_weekday(self) -> i32 {
        let calendar = gregorian();
        let first = calendar
            .dateFromComponents(&components(self.first_day()))
            .expect("a calendar month resolves to a date");
        i32::try_from(calendar.component_fromDate(NSCalendarUnit::Weekday, &first))
            .expect("weekday is 1 through 7")
    }
}

/// `day` moved by `days` calendar days, crossing month and year boundaries.
#[must_use]
pub fn add_days(day: DateParts, days: i32) -> DateParts {
    let mut month = YearMonth::of(day);
    let mut value = day.day + days;
    while value < 1 {
        month = month.previous();
        value += month.day_count();
    }
    loop {
        let count = month.day_count();
        if value <= count {
            break;
        }
        value -= count;
        month = month.next();
    }
    DateParts {
        year: month.year,
        month: month.month,
        day: value,
    }
}

/// The current calendar day in the user's time zone.
///
/// # Panics
///
/// Never in practice: the current instant always resolves to a date.
#[must_use]
pub fn today() -> DateParts {
    date_parts(&NSDate::now()).expect("the current instant resolves to a calendar date")
}

/// `date` formatted in the user's locale at medium date style with no time —
/// the format a selection list row shows.
#[must_use]
pub fn format_medium(date: &NSDate) -> String {
    NSDateFormatter::localizedStringFromDate_dateStyle_timeStyle(
        date,
        NSDateFormatterStyle::MediumStyle,
        NSDateFormatterStyle::NoStyle,
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_round_trips_through_components() {
        let parts = DateParts {
            year: 2026,
            month: 9,
            day: 29,
        };
        assert_eq!(super::parts(&components(parts)), Some(parts));
    }

    #[test]
    fn parts_rejects_unset_components() {
        assert_eq!(super::parts(&NSDateComponents::new()), None);
    }

    #[test]
    fn month_lengths_and_weekdays_follow_the_gregorian_calendar() {
        let february_leap = YearMonth {
            year: 2024,
            month: 2,
        };
        assert_eq!(february_leap.day_count(), 29);
        assert_eq!(
            YearMonth {
                year: 2100,
                month: 2
            }
            .day_count(),
            28
        );
        assert_eq!(
            YearMonth {
                year: 2026,
                month: 11
            }
            .day_count(),
            30
        );
        // 1 November 2026 is a Sunday.
        assert_eq!(
            YearMonth {
                year: 2026,
                month: 11
            }
            .first_weekday(),
            1
        );
        assert_eq!(
            february_leap.previous().previous().next(),
            YearMonth {
                year: 2024,
                month: 1,
            }
        );
        assert_eq!(
            YearMonth {
                year: 2026,
                month: 12
            }
            .next(),
            YearMonth {
                year: 2027,
                month: 1,
            }
        );
    }

    #[test]
    fn add_days_crosses_month_and_year_boundaries() {
        let day = |year, month, day| DateParts { year, month, day };
        assert_eq!(add_days(day(2026, 12, 30), 7), day(2027, 1, 6));
        assert_eq!(add_days(day(2027, 1, 3), -7), day(2026, 12, 27));
        assert_eq!(add_days(day(2024, 2, 28), 1), day(2024, 2, 29));
        assert_eq!(add_days(day(2024, 3, 1), -1), day(2024, 2, 29));
        assert_eq!(add_days(day(2026, 11, 18), 0), day(2026, 11, 18));
    }

    #[test]
    fn date_parts_ordering_is_lexicographic() {
        let a = DateParts {
            year: 2026,
            month: 9,
            day: 29,
        };
        let b = DateParts {
            year: 2026,
            month: 10,
            day: 1,
        };
        assert!(a < b);
    }
}
