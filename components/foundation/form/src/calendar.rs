//! A WaterUI-composed month-grid calendar view.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};
use core::{num::NonZeroUsize, ops::RangeInclusive};

use jiff::{
    Timestamp, ToSpan,
    civil::{Date, Weekday},
};
use nami::{Binding, Computed, SignalExt, collection::SignalCollection, signal::IntoComputed};
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::{IntoLabel, button};
use waterui_core::{AnyView, Environment, View, id::Identifiable, views::ForEach};
use waterui_graphics::color::{AccentColor, AccentForegroundColor, Color, ForegroundColor};
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::stack::{Alignment, HStack, HorizontalAlignment, hstack, vstack};
use waterui_layout::{BackgroundView, LazyContainer, Size, grid::GridLayout};
use waterui_locale::format::date::{format_calendar_month_year, format_calendar_weekday};
use waterui_locale::{Locale, locale_binding};
use waterui_shape::{Circle, ShapeExt};
use waterui_text::{
    Text,
    styled::{Style, StyledStr},
};

const CALENDAR_DAY_SIZE: f32 = 40.0;
const CALENDAR_SELECTED_DAY_SIZE: f32 = 36.0;
const CALENDAR_CELL_SPACING: f32 = 8.0;
const CALENDAR_WEEKDAY_HEIGHT: f32 = 24.0;

#[derive(Debug)]
/// A calendar-style control for selecting a single date.
pub struct Calendar {
    label: Label,
    value: Binding<Date>,
    range: RangeInclusive<Date>,
    decorated: Computed<BTreeSet<Date>>,
    visible_month: Binding<VisibleMonth>,
}

impl Calendar {
    /// Creates a new `Calendar` with the given semantic label, selected date
    /// binding, and visible month binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(label: impl IntoLabel, date: &Binding<Date>, visible_month: &Binding<Date>) -> Self {
        let range = Date::MIN..=Date::MAX;
        Self {
            label: label.into_label(),
            value: date.clone(),
            range,
            decorated: Computed::constant(BTreeSet::new()),
            visible_month: map_visible_month_binding(visible_month),
        }
    }

    /// Sets the valid date range.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        if !visible_month_in_range(self.visible_month.get(), &range) {
            self.visible_month
                .set(initial_visible_month(Some(self.value.get()), &range));
        }
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
    pub const fn label_style(mut self, mode: LabelDisplayMode) -> Self {
        self.label.set_display_mode(mode);
        self
    }

    /// Visually hides the label above the calendar grid while preserving its
    /// semantic text for assistive technology.
    #[must_use]
    pub const fn hide_label(self) -> Self {
        self.label_style(LabelDisplayMode::Hidden)
    }
}

impl View for Calendar {
    fn body(self, env: &Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let range = self.range;
        let decorated = self.decorated;
        let visible_month = self.visible_month;
        let locale = locale_binding(env).computed();

        let cell_range = range.clone();
        let cell_selection = selection;
        let cell_decorated = decorated;
        let rows = calendar_rows(&visible_month, move |cell| {
            single_day_cell_content(cell, &cell_range, cell_selection.clone(), &cell_decorated)
        });
        let calendar = CalendarBody::new(locale, range, visible_month, rows);

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

#[derive(Debug, Clone, Copy, waterui_macros::Identifiable)]
pub(crate) struct DayCell {
    #[id]
    identity: (Date, bool),
    pub(crate) date: Date,
    pub(crate) in_current_month: bool,
}

pub(crate) struct CalendarBody<Content> {
    locale: Computed<Locale>,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    content: Content,
}

impl<Content> CalendarBody<Content> {
    pub(crate) const fn new(
        locale: Computed<Locale>,
        range: RangeInclusive<Date>,
        visible_month: Binding<VisibleMonth>,
        content: Content,
    ) -> Self {
        Self {
            locale,
            range,
            visible_month,
            content,
        }
    }
}

impl<Content: View> View for CalendarBody<Content> {
    fn body(self, _env: &Environment) -> impl View {
        Padding::new(
            EdgeInsets::all(8.0),
            vstack((
                build_month_header(&self.locale, self.visible_month, self.range),
                weekday_header(self.locale),
                self.content,
            ))
            .spacing(8.0),
        )
    }
}

pub(crate) fn calendar_rows<V, F>(
    visible_month: &Binding<VisibleMonth>,
    cell_view: F,
) -> impl View + use<V, F>
where
    V: View,
    F: Fn(DayCell) -> V + 'static,
{
    let cells = SignalCollection::new(visible_month.map(month_cells));
    LazyContainer::new(
        GridLayout::new(
            NonZeroUsize::new(7).expect("calendar column count must be non-zero"),
            Size::new(CALENDAR_CELL_SPACING, CALENDAR_CELL_SPACING),
            Alignment::Center,
        ),
        ForEach::new(cells, cell_view),
    )
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

pub(crate) fn visible_month_in_range(month: VisibleMonth, range: &RangeInclusive<Date>) -> bool {
    let first_day = month.first_day();
    first_day >= month_start(*range.start()) && first_day <= month_start(*range.end())
}

pub(crate) fn map_visible_month_binding(month: &Binding<Date>) -> Binding<VisibleMonth> {
    Binding::mapping(
        month,
        VisibleMonth::from_date,
        |binding, month: VisibleMonth| {
            binding.set(month.first_day());
        },
    )
}

fn build_month_header(
    locale: &Computed<Locale>,
    visible_month: Binding<VisibleMonth>,
    range: RangeInclusive<Date>,
) -> impl View {
    let title = Text::computed(
        locale
            .zip(&visible_month)
            .map(|(locale, month)| {
                StyledStr::plain(format_calendar_month_year(&locale, &month.first_day()))
            })
            .computed(),
    )
    .headline();
    let first_month = month_start(*range.start());
    let last_month = month_start(*range.end());
    let previous_disabled = visible_month
        .map(move |month| month.previous().first_day() < first_month)
        .computed();
    let next_disabled = visible_month
        .map(move |month| month.next().first_day() > last_month)
        .computed();

    hstack((
        Frame::new(month_navigation_button(
            "<",
            previous_disabled,
            visible_month.clone(),
            VisibleMonth::previous,
        ))
        .width(CALENDAR_DAY_SIZE),
        title,
        Frame::new(month_navigation_button(
            ">",
            next_disabled,
            visible_month,
            VisibleMonth::next,
        ))
        .width(CALENDAR_DAY_SIZE),
    ))
    .spacing(16.0)
}

fn month_navigation_button(
    label: &'static str,
    disabled: Computed<bool>,
    visible_month: Binding<VisibleMonth>,
    step: fn(VisibleMonth) -> VisibleMonth,
) -> impl View {
    button(label)
        .borderless()
        .disabled(disabled)
        .action(move || {
            let mut month = visible_month.get_mut();
            let next = step(*month);
            *month = next;
        })
}

/// Wraps day-grid content in its column cell: ideally [`CALENDAR_DAY_SIZE`]
/// wide but free to shrink with its row, so the seven columns compress evenly
/// on layouts narrower than the full grid instead of overflowing the calendar.
fn day_grid_cell(content: impl View, height: f32) -> Frame {
    Frame::new(content)
        .width(CALENDAR_DAY_SIZE)
        .min_width(0.0)
        .height(height)
}

fn weekday_header(locale: Computed<Locale>) -> impl View {
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
    .map(move |weekday| {
        day_grid_cell(
            Text::computed(
                locale
                    .clone()
                    .map(move |locale| StyledStr::plain(format_calendar_weekday(&locale, weekday))),
            )
            .caption(),
            CALENDAR_WEEKDAY_HEIGHT,
        )
    })
    .collect::<HStack<_>>()
    .spacing(CALENDAR_CELL_SPACING)
}

fn single_day_cell_content(
    cell: DayCell,
    range: &RangeInclusive<Date>,
    selection: Binding<Date>,
    decorated_dates: &Computed<BTreeSet<Date>>,
) -> AnyView {
    let is_in_range = range.contains(&cell.date);
    let is_selectable = cell.in_current_month && is_in_range;
    let selected = selection
        .map(move |selected_date| selected_date == cell.date)
        .computed();
    let decorated = decorated_dates
        .map(move |dates| dates.contains(&cell.date))
        .computed();

    if is_selectable {
        selectable_day_cell(cell.date, &decorated, &selected, move || {
            selection.set(cell.date);
        })
    } else {
        AnyView::new(day_grid_cell(
            day_cell_placeholder(cell, &decorated),
            CALENDAR_DAY_SIZE,
        ))
    }
}

pub(crate) fn multi_day_cell_content(
    cell: DayCell,
    range: &RangeInclusive<Date>,
    selection: Binding<BTreeSet<Date>>,
    decorated_dates: &Computed<BTreeSet<Date>>,
) -> AnyView {
    let is_in_range = range.contains(&cell.date);
    let is_selectable = cell.in_current_month && is_in_range;
    let selected = selection
        .map(move |dates| dates.contains(&cell.date))
        .computed();
    let decorated = decorated_dates
        .map(move |dates| dates.contains(&cell.date))
        .computed();

    if is_selectable {
        selectable_day_cell(cell.date, &decorated, &selected, move || {
            let mut dates = selection.get();
            if !dates.insert(cell.date) {
                dates.remove(&cell.date);
            }
            selection.set(dates);
        })
    } else {
        AnyView::new(day_grid_cell(
            day_cell_placeholder(cell, &decorated),
            CALENDAR_DAY_SIZE,
        ))
    }
}

fn selectable_day_cell(
    date: Date,
    decorated: &Computed<bool>,
    selected: &Computed<bool>,
    action: impl FnMut() + 'static,
) -> AnyView {
    let foreground = selected.map(|selected| {
        if selected {
            Color::new(AccentForegroundColor)
        } else {
            Color::new(ForegroundColor)
        }
    });
    let button = button(day_cell_label(date, decorated).color(foreground))
        .plain()
        .accessibility_label(day_cell_accessibility_label(date))
        .action(action);
    let content = day_grid_cell(button, CALENDAR_DAY_SIZE);

    AnyView::new(BackgroundView::new(
        content,
        Frame::new(
            Circle.fill(
                Color::new(AccentColor)
                    .with_opacity(selected.map(|selected| if selected { 1.0 } else { 0.0 })),
            ),
        )
        .width(CALENDAR_SELECTED_DAY_SIZE)
        .height(CALENDAR_SELECTED_DAY_SIZE),
    ))
}

fn day_cell_label(date: Date, decorated: &Computed<bool>) -> Text {
    Text::computed(decorated.map(move |decorated| {
        let mut label = StyledStr::plain(date.day().to_string());
        if decorated {
            label.push("•", Style::new());
        }
        label
    }))
    .text_align(HorizontalAlignment::Center)
}

fn day_cell_accessibility_label(date: Date) -> String {
    date.day().to_string()
}

fn day_cell_placeholder(cell: DayCell, decorated: &Computed<bool>) -> impl View {
    if cell.in_current_month {
        AnyView::new(day_cell_label(cell.date, decorated).caption())
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
                identity: (date, month.contains(date)),
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
