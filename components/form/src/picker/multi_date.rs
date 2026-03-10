//! A multi-date picker rendered entirely with WaterUI primitives.

use alloc::{
    collections::BTreeSet,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::ops::RangeInclusive;

use nami::{Binding, SignalExt};
use time::{Date, Duration, Month, OffsetDateTime, Weekday};
use waterui_controls::button;
use waterui_core::dynamic::Dynamic;
use waterui_core::{AnyView, View};
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::spacer;
use waterui_layout::stack::{HStack, VStack, hstack, vstack};
use waterui_text::{Text, text};

#[derive(Debug)]
/// A calendar-style control for selecting multiple dates.
pub struct MultiDatePicker {
    label: AnyView,
    value: Binding<BTreeSet<Date>>,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
}

impl MultiDatePicker {
    /// Creates a new `MultiDatePicker` with the given binding for selected dates.
    #[must_use]
    pub fn new(date: &Binding<BTreeSet<Date>>) -> Self {
        let range = Date::MIN..=Date::MAX;
        let visible_month = Binding::container(initial_visible_month(date, &range));
        Self {
            label: AnyView::default(),
            value: date.clone(),
            range,
            visible_month,
        }
    }

    /// Sets the label for the multi-date picker.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.label = AnyView::new(label);
        self
    }

    /// Sets the valid date range for the picker.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        let initial_month = initial_visible_month(&self.value, &range);
        self.visible_month.set(initial_month);
        self.range = range;
        self
    }
}

impl View for MultiDatePicker {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let visible_month = self.visible_month;
        let range = self.range;

        vstack((
            label,
            Dynamic::watch(visible_month.zip(&selection), move |(month, selected_dates)| {
                AnyView::new(build_picker_body(
                    month,
                    selected_dates,
                    range.clone(),
                    visible_month.clone(),
                    selection.clone(),
                ))
            }),
        ))
        .spacing(10.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct VisibleMonth {
    year: i32,
    month: Month,
}

impl VisibleMonth {
    fn from_date(date: Date) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    fn first_day(self) -> Date {
        Date::from_calendar_date(self.year, self.month, 1)
            .expect("visible month must always be a valid calendar month")
    }

    fn previous(self) -> Self {
        let month_number = self.month as u8;
        if month_number == Month::January as u8 {
            Self {
                year: self.year - 1,
                month: Month::December,
            }
        } else {
            Self {
                year: self.year,
                month: Month::try_from(month_number - 1)
                    .expect("previous month number must be valid"),
            }
        }
    }

    fn next(self) -> Self {
        let month_number = self.month as u8;
        if month_number == Month::December as u8 {
            Self {
                year: self.year + 1,
                month: Month::January,
            }
        } else {
            Self {
                year: self.year,
                month: Month::try_from(month_number + 1).expect("next month number must be valid"),
            }
        }
    }

    fn contains(self, date: Date) -> bool {
        self.year == date.year() && self.month == date.month()
    }
}

#[derive(Debug, Clone, Copy)]
struct DayCell {
    date: Date,
    in_current_month: bool,
}

fn build_picker_body(
    month: VisibleMonth,
    selected_dates: BTreeSet<Date>,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    selection: Binding<BTreeSet<Date>>,
) -> impl View {
    let can_go_previous = month.previous().first_day() >= month_start(*range.start());
    let can_go_next = month.next().first_day() <= month_start(*range.end());

    Padding::new(
        EdgeInsets::all(8.0),
        vstack((
            build_month_header(month, visible_month, can_go_previous, can_go_next),
            weekday_header(),
            calendar_rows(month, &selected_dates, range, selection),
        ))
        .spacing(8.0),
    )
}

fn build_month_header(
    month: VisibleMonth,
    visible_month: Binding<VisibleMonth>,
    can_go_previous: bool,
    can_go_next: bool,
) -> impl View {
    let title = text(format!("{} {}", month_name(month.month), month.year)).headline();

    hstack((
        Frame::new(month_navigation_button("<", can_go_previous, visible_month.clone(), VisibleMonth::previous)).width(40.0),
        spacer(),
        title,
        spacer(),
        Frame::new(month_navigation_button(">", can_go_next, visible_month, VisibleMonth::next)).width(40.0),
    ))
    .spacing(8.0)
}

fn month_navigation_button(
    label: &'static str,
    enabled: bool,
    visible_month: Binding<VisibleMonth>,
    step: fn(VisibleMonth) -> VisibleMonth,
) -> impl View {
    if enabled {
        AnyView::new(
            button(label)
                .bordered()
                .with_state(&visible_month)
                .action(move |current| current.set(step(current.get()))),
        )
    } else {
        AnyView::new(button(label).borderless())
    }
}

fn weekday_header() -> impl View {
    let labels = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];

    labels
        .into_iter()
        .map(|weekday| {
            Frame::new(Text::new(weekday_label(weekday)).caption())
                .width(40.0)
                .height(24.0)
        })
        .collect::<HStack<_>>()
        .spacing(6.0)
}

fn calendar_rows(
    month: VisibleMonth,
    selected_dates: &BTreeSet<Date>,
    range: RangeInclusive<Date>,
    selection: Binding<BTreeSet<Date>>,
) -> impl View {
    month_cells(month)
        .chunks(7)
        .map(|week| {
            AnyView::new(
                week.iter()
                    .copied()
                    .map(|cell| day_cell_view(cell, month, selected_dates, range.clone(), selection.clone()))
                    .collect::<HStack<_>>()
                    .spacing(6.0),
            )
        })
        .collect::<VStack<_>>()
        .spacing(6.0)
}

fn day_cell_view(
    cell: DayCell,
    month: VisibleMonth,
    selected_dates: &BTreeSet<Date>,
    range: RangeInclusive<Date>,
    selection: Binding<BTreeSet<Date>>,
) -> AnyView {
    let is_selected = selected_dates.contains(&cell.date);
    let is_in_range = range.contains(&cell.date);
    let is_selectable = cell.in_current_month && is_in_range;
    let label = cell.date.day().to_string();

    let view = if is_selectable {
        let button = if is_selected {
            button(label).bordered_prominent()
        } else if month.contains(cell.date) {
            button(label).bordered()
        } else {
            button(label).borderless()
        };

        AnyView::new(
            Frame::new(
                button.with_state(&selection).action(move |selected| {
                    let mut dates = selected.get();
                    if !dates.insert(cell.date) {
                        dates.remove(&cell.date);
                    }
                    selected.set(dates);
                }),
            )
            .width(40.0)
            .height(36.0),
        )
    } else {
        let label = if cell.in_current_month {
            Text::new(label).caption()
        } else {
            Text::new(String::new()).caption()
        };

        AnyView::new(Frame::new(label).width(40.0).height(36.0))
    };

    view
}

fn month_cells(month: VisibleMonth) -> Vec<DayCell> {
    let first_day = month.first_day();
    let offset = first_day.weekday().number_days_from_monday() as i64;
    let grid_start = first_day - Duration::days(offset);

    (0_i64..42_i64)
        .map(|day_offset| {
            let date = grid_start + Duration::days(day_offset);
            DayCell {
                date,
                in_current_month: month.contains(date),
            }
        })
        .collect()
}

fn month_start(date: Date) -> Date {
    Date::from_calendar_date(date.year(), date.month(), 1)
        .expect("range month start must be a valid calendar date")
}

fn month_name(month: Month) -> &'static str {
    match month {
        Month::January => "January",
        Month::February => "February",
        Month::March => "March",
        Month::April => "April",
        Month::May => "May",
        Month::June => "June",
        Month::July => "July",
        Month::August => "August",
        Month::September => "September",
        Month::October => "October",
        Month::November => "November",
        Month::December => "December",
    }
}

fn weekday_label(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

fn initial_visible_month(
    selection: &Binding<BTreeSet<Date>>,
    range: &RangeInclusive<Date>,
) -> VisibleMonth {
    if let Some(first) = selection.get().iter().next().copied() {
        return VisibleMonth::from_date(first);
    }

    #[cfg(feature = "std")]
    {
        let today = OffsetDateTime::now_utc().date();
        if range.contains(&today) {
            return VisibleMonth::from_date(today);
        }
    }

    VisibleMonth::from_date(*range.start())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_grid_starts_on_monday() {
        let month = VisibleMonth {
            year: 2025,
            month: Month::June,
        };
        let cells = month_cells(month);

        assert_eq!(cells.len(), 42);
        assert_eq!(cells[0].date.weekday(), Weekday::Monday);
        assert!(cells.iter().any(|cell| cell.date.day() == 1 && cell.in_current_month));
    }

    #[test]
    fn visible_month_wraps_across_years() {
        let january = VisibleMonth {
            year: 2025,
            month: Month::January,
        };
        let december = january.previous();
        let next = december.next();

        assert_eq!(december.year, 2024);
        assert_eq!(december.month, Month::December);
        assert_eq!(next, january);
    }
}
