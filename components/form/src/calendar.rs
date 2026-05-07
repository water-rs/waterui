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
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::{IntoLabel, button};
use waterui_core::{AnyView, Environment, LocalStateScope, LocalStateStore, View};
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::spacer;
use waterui_layout::stack::{HStack, HorizontalAlignment, VStack, hstack, vstack};
use waterui_locale::format::date::{format_calendar_month_year, format_calendar_weekday};
use waterui_locale::{Locale, locale_binding};
use waterui_text::{
    Text,
    font::Caption,
    styled::{Style, StyledStr},
    text,
};

#[derive(Debug)]
/// A calendar-style control for selecting a single date.
pub struct Calendar {
    label: Label,
    value: Binding<Date>,
    range: RangeInclusive<Date>,
    decorated: Computed<BTreeSet<Date>>,
}

impl Calendar {
    /// Creates a new `Calendar` with the given semantic label and selected
    /// date binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(label: impl IntoLabel, date: &Binding<Date>) -> Self {
        let range = Date::MIN..=Date::MAX;
        Self {
            label: label.into_label(),
            value: date.clone(),
            range,
            decorated: Computed::constant(BTreeSet::new()),
        }
    }

    /// Sets the valid date range.
    #[must_use]
    pub const fn range(mut self, range: RangeInclusive<Date>) -> Self {
        self.range = range;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    #[must_use]
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.decorated = decorated.into_computed();
        self
    }

    /// Sets the visual presentation mode of the label displayed above the
    /// calendar grid. The semantic identity is always retained for assistive
    /// technology.
    #[must_use]
    pub fn label_style(mut self, mode: LabelDisplayMode) -> Self {
        self.label.set_display_mode(mode);
        self
    }

    /// Visually hides the label above the calendar grid while preserving its
    /// semantic text for assistive technology.
    #[must_use]
    pub fn hide_label(self) -> Self {
        self.label_style(LabelDisplayMode::Hidden)
    }
}

impl View for Calendar {
    fn body(self, env: &Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let range = self.range;
        let decorated = self.decorated;
        let locale = resolve_locale(env);
        let visible_month = local_binding(env, {
            let selection = selection.clone();
            let range = range.clone();
            move || initial_visible_month(Some(selection.get()), &range)
        });

        let selection_and_decorated = selection.zip(&decorated);
        let calendar = visible_month
            .clone()
            .zip(&selection_and_decorated)
            .map(move |(month, (selected_date, decorated_dates))| {
                let cell_range = range.clone();
                let cell_selection = selection.clone();
                CalendarBody::new(
                    locale.clone(),
                    month,
                    range.clone(),
                    visible_month.clone(),
                    calendar_rows(month, move |cell| {
                        single_day_cell_content(
                            cell,
                            selected_date,
                            &cell_range,
                            cell_selection.clone(),
                            &decorated_dates,
                        )
                    }),
                )
            })
            .computed();

        vstack((label, calendar)).spacing(10.0)
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

    pub(crate) const fn previous(self) -> Self {
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

    pub(crate) const fn next(self) -> Self {
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

pub(crate) struct CalendarBody<Content> {
    locale: Locale,
    month: VisibleMonth,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    content: Content,
}

impl<Content> CalendarBody<Content> {
    pub(crate) const fn new(
        locale: Locale,
        month: VisibleMonth,
        range: RangeInclusive<Date>,
        visible_month: Binding<VisibleMonth>,
        content: Content,
    ) -> Self {
        Self {
            locale,
            month,
            range,
            visible_month,
            content,
        }
    }
}

impl<Content: View> View for CalendarBody<Content> {
    fn body(self, _env: &Environment) -> impl View {
        let can_go_previous = self.month.previous().first_day() >= month_start(*self.range.start());
        let can_go_next = self.month.next().first_day() <= month_start(*self.range.end());

        Padding::new(
            EdgeInsets::all(8.0),
            vstack((
                build_month_header(
                    &self.locale,
                    self.month,
                    self.visible_month,
                    can_go_previous,
                    can_go_next,
                ),
                weekday_header(&self.locale),
                self.content,
            ))
            .spacing(8.0),
        )
    }
}

pub(crate) fn calendar_rows<V: View>(
    month: VisibleMonth,
    mut cell_view: impl FnMut(DayCell) -> V,
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
        rows.push(row);
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
    locale_binding(env).get()
}

pub(crate) fn local_binding<T: Clone + 'static>(
    env: &Environment,
    init: impl FnOnce() -> T + 'static,
) -> Binding<T> {
    let scope = env
        .get::<LocalStateScope>()
        .unwrap_or_else(|| panic!("waterui-form requires renderer LocalStateScope support"))
        .clone();
    env.get::<LocalStateStore>()
        .unwrap_or_else(|| panic!("waterui-form requires renderer LocalStateStore support"))
        .binding(&scope, init)
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
        AnyView::new(button(label).bordered().action(move || {
            visible_month.set(step(visible_month.get()));
        }))
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

fn single_day_cell_content(
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
        let accessibility_label = day_cell_accessibility_label(cell.date);
        let button = if is_selected {
            button(day_cell_label(cell.date, decorated))
                .accessibility_label(accessibility_label)
                .bordered_prominent()
        } else {
            button(day_cell_label(cell.date, decorated))
                .accessibility_label(accessibility_label)
                .bordered()
        };

        AnyView::new(
            Frame::new(button.action(move || {
                selection.set(cell.date);
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

pub(crate) fn multi_day_cell_content(
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
        let accessibility_label = day_cell_accessibility_label(cell.date);
        let button = if is_selected {
            button(day_cell_label(cell.date, decorated))
                .accessibility_label(accessibility_label)
                .bordered_prominent()
        } else {
            button(day_cell_label(cell.date, decorated))
                .accessibility_label(accessibility_label)
                .bordered()
        };

        AnyView::new(
            Frame::new(button.action(move || {
                let mut dates = selection.get();
                if !dates.insert(cell.date) {
                    dates.remove(&cell.date);
                }
                selection.set(dates);
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

fn day_cell_label(date: Date, decorated: bool) -> Text {
    let mut label = StyledStr::plain(date.day().to_string());
    if decorated {
        label.push("\n•", Style::new().font(Caption));
    }
    Text::new(label).text_align(HorizontalAlignment::Center)
}

fn day_cell_accessibility_label(date: Date) -> String {
    date.day().to_string()
}

fn day_cell_placeholder(cell: DayCell, decorated: bool) -> impl View {
    if cell.in_current_month {
        let day = Text::new(cell.date.day().to_string()).caption();
        if decorated {
            AnyView::new(vstack((day, Text::new("•").caption())).spacing(0.0))
        } else {
            AnyView::new(day)
        }
    } else {
        AnyView::new(Text::new(String::new()).caption())
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
