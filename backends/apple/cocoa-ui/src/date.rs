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
