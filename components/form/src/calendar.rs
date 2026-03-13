//! A WaterUI-composed month-grid calendar view.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};
use core::ops::RangeInclusive;

use jiff::{
    Timestamp, ToSpan,
    civil::{Date, Weekday},
};
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_controls::button;
use waterui_core::dynamic::Dynamic;
use waterui_core::extract::Extractor;
use waterui_core::{AnyView, Environment, View};
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::spacer;
use waterui_layout::stack::{HStack, VStack, hstack, vstack};
use waterui_locale::format::date::{format_calendar_month_year, format_calendar_weekday};
use waterui_locale::{
    Locale,
    regional::{self, RegionalContext},
};
use waterui_text::{Text, text};

#[derive(Debug)]
/// A calendar-style control for selecting a single date.
pub struct Calendar {
    label: AnyView,
    value: Binding<Date>,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    decorated: Computed<BTreeSet<Date>>,
}

impl Calendar {
    /// Creates a new `Calendar` with the given selected date binding.
    #[must_use]
    pub fn new(date: &Binding<Date>) -> Self {
        let range = Date::MIN..=Date::MAX;
        let visible_month = Binding::container(initial_visible_month(Some(date.get()), &range));
        Self {
            label: AnyView::default(),
            value: date.clone(),
            range,
            visible_month,
            decorated: Computed::constant(BTreeSet::new()),
        }
    }

    /// Sets the label displayed above the calendar.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.label = AnyView::new(label);
        self
    }

    /// Sets the valid date range.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        let visible_month = initial_visible_month(Some(self.value.get()), &range);
        self.visible_month.set(visible_month);
        self.range = range;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    #[must_use]
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.decorated = decorated.into_computed();
        self
    }
}

impl View for Calendar {
    fn body(self, env: &Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let visible_month = self.visible_month;
        let range = self.range;
        let decorated = self.decorated;
        let locale = resolve_locale(env);

        vstack((
            label,
            Dynamic::watch(
                visible_month.zip(&selection).zip(&decorated),
                move |((month, selected_date), decorated_dates)| {
                    AnyView::new(build_calendar_body(
                        &locale,
                        month,
                        &range,
                        visible_month.clone(),
                        calendar_rows(month, |cell| {
                            single_day_cell_view(
                                cell,
                                selected_date,
                                &range,
                                selection.clone(),
                                &decorated_dates,
                            )
                        }),
                    ))
                },
            ),
        ))
        .spacing(10.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct VisibleMonth {
    year: i16,
    month: i8,
}

impl VisibleMonth {
    pub(crate) fn from_date(date: Date) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    pub(crate) fn first_day(self) -> Date {
        Date::new(self.year, self.month, 1)
            .expect("visible month must always be a valid calendar month")
    }

    pub(crate) fn previous(self) -> Self {
        if self.month == 1 {
            Self {
                year: self.year.checked_sub(1).expect("calendar year underflow"),
                month: 12,
            }
        } else {
            Self {
                year: self.year,
                month: self.month - 1,
            }
        }
    }

    pub(crate) fn next(self) -> Self {
        if self.month == 12 {
            Self {
                year: self.year.checked_add(1).expect("calendar year overflow"),
                month: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    pub(crate) fn contains(self, date: Date) -> bool {
        self.year == date.year() && self.month == date.month()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DayCell {
    pub(crate) date: Date,
    pub(crate) in_current_month: bool,
}

pub(crate) fn build_calendar_body(
    locale: &Locale,
    month: VisibleMonth,
    range: &RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    content: impl View,
) -> impl View {
    let can_go_previous = month.previous().first_day() >= month_start(*range.start());
    let can_go_next = month.next().first_day() <= month_start(*range.end());

    Padding::new(
        EdgeInsets::all(8.0),
        vstack((
            build_month_header(locale, month, visible_month, can_go_previous, can_go_next),
            weekday_header(locale),
            content,
        ))
        .spacing(8.0),
    )
}

pub(crate) fn calendar_rows(
    month: VisibleMonth,
    mut cell_view: impl FnMut(DayCell) -> AnyView,
) -> impl View {
    let cells = month_cells(month);
    let mut rows = Vec::new();

    for week in cells.chunks(7) {
        let row = week
            .iter()
            .copied()
            .map(&mut cell_view)
            .collect::<HStack<_>>()
            .spacing(6.0);
        rows.push(AnyView::new(row));
    }

    rows.into_iter().collect::<VStack<_>>().spacing(6.0)
}

pub(crate) fn initial_visible_month(
    selected: Option<Date>,
    range: &RangeInclusive<Date>,
) -> VisibleMonth {
    if let Some(date) = selected {
        return VisibleMonth::from_date(date);
    }

    #[cfg(feature = "std")]
    {
        let today = Timestamp::now()
            .in_tz("UTC")
            .expect("UTC time zone must be valid")
            .date();
        if range.contains(&today) {
            return VisibleMonth::from_date(today);
        }
    }

    VisibleMonth::from_date(*range.start())
}

pub(crate) fn resolve_locale(env: &Environment) -> Locale {
    if let Ok(context) = RegionalContext::extract(env) {
        return context.locale().clone();
    }
    regional::current_settings().locale().clone()
}

fn build_month_header(
    locale: &Locale,
    month: VisibleMonth,
    visible_month: Binding<VisibleMonth>,
    can_go_previous: bool,
    can_go_next: bool,
) -> impl View {
    let title = text(format_calendar_month_year(locale, &month.first_day())).headline();

    hstack((
        Frame::new(month_navigation_button(
            "<",
            can_go_previous,
            visible_month.clone(),
            VisibleMonth::previous,
        ))
        .width(40.0),
        spacer(),
        title,
        spacer(),
        Frame::new(month_navigation_button(
            ">",
            can_go_next,
            visible_month,
            VisibleMonth::next,
        ))
        .width(40.0),
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

fn weekday_header(locale: &Locale) -> impl View {
    [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ]
    .into_iter()
    .map(|weekday| {
        Frame::new(Text::new(format_calendar_weekday(locale, weekday)).caption())
            .width(44.0)
            .height(24.0)
    })
    .collect::<HStack<_>>()
    .spacing(6.0)
}

fn single_day_cell_view(
    cell: DayCell,
    selected_date: Date,
    range: &RangeInclusive<Date>,
    selection: Binding<Date>,
    decorated_dates: &BTreeSet<Date>,
) -> AnyView {
    let is_selected = selected_date == cell.date;
    let is_in_range = range.contains(&cell.date);
    let is_selectable = cell.in_current_month && is_in_range;
    let decorated = decorated_dates.contains(&cell.date);

    if is_selectable {
        let button = if is_selected {
            button(day_cell_label(cell.date, decorated)).bordered_prominent()
        } else {
            button(day_cell_label(cell.date, decorated)).bordered()
        };

        AnyView::new(
            Frame::new(
                button
                    .with_state(&selection)
                    .action(move |selected| selected.set(cell.date)),
            )
            .width(44.0)
            .height(40.0),
        )
    } else {
        AnyView::new(
            Frame::new(day_cell_placeholder(cell, decorated))
                .width(44.0)
                .height(40.0),
        )
    }
}

pub(crate) fn multi_day_cell_view(
    cell: DayCell,
    selected_dates: &BTreeSet<Date>,
    range: &RangeInclusive<Date>,
    selection: Binding<BTreeSet<Date>>,
    decorated_dates: &BTreeSet<Date>,
) -> AnyView {
    let is_selected = selected_dates.contains(&cell.date);
    let is_in_range = range.contains(&cell.date);
    let is_selectable = cell.in_current_month && is_in_range;
    let decorated = decorated_dates.contains(&cell.date);

    if is_selectable {
        let button = if is_selected {
            button(day_cell_label(cell.date, decorated)).bordered_prominent()
        } else {
            button(day_cell_label(cell.date, decorated)).bordered()
        };

        AnyView::new(
            Frame::new(button.with_state(&selection).action(move |selected| {
                let mut dates = selected.get();
                if !dates.insert(cell.date) {
                    dates.remove(&cell.date);
                }
                selected.set(dates);
            }))
            .width(44.0)
            .height(40.0),
        )
    } else {
        AnyView::new(
            Frame::new(day_cell_placeholder(cell, decorated))
                .width(44.0)
                .height(40.0),
        )
    }
}

fn day_cell_label(date: Date, decorated: bool) -> AnyView {
    let day = Text::new(date.day().to_string());
    if decorated {
        AnyView::new(vstack((day, Text::new("•").caption())).spacing(0.0))
    } else {
        AnyView::new(day)
    }
}

fn day_cell_placeholder(cell: DayCell, decorated: bool) -> AnyView {
    if !cell.in_current_month {
        return AnyView::new(Text::new(String::new()).caption());
    }

    let day = Text::new(cell.date.day().to_string()).caption();
    if decorated {
        AnyView::new(vstack((day, Text::new("•").caption())).spacing(0.0))
    } else {
        AnyView::new(day)
    }
}

fn month_cells(month: VisibleMonth) -> Vec<DayCell> {
    let first_day = month.first_day();
    let offset = first_day.weekday().to_monday_zero_offset();
    let grid_start = first_day - i32::from(offset).days();

    (0_i32..42_i32)
        .map(|day_offset| {
            let date = grid_start + day_offset.days();
            DayCell {
                date,
                in_current_month: month.contains(date),
            }
        })
        .collect()
}

fn month_start(date: Date) -> Date {
    Date::new(date.year(), date.month(), 1)
        .expect("range month start must be a valid calendar date")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_grid_starts_on_monday() {
        let month = VisibleMonth {
            year: 2025,
            month: 6,
        };
        let cells = month_cells(month);

        assert_eq!(cells.len(), 42);
        assert_eq!(cells[0].date.weekday(), Weekday::Monday);
        assert!(
            cells
                .iter()
                .any(|cell| cell.date.day() == 1 && cell.in_current_month)
        );
    }

    #[test]
    fn visible_month_wraps_across_years() {
        let january = VisibleMonth {
            year: 2025,
            month: 1,
        };
        let december = january.previous();
        let next = december.next();

        assert_eq!(december.year, 2024);
        assert_eq!(december.month, 12);
        assert_eq!(next, january);
    }
}
