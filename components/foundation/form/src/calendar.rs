//! The single-date `Calendar` control and the month grid WaterUI composes
//! for backends that do not realize it natively.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};
use core::{num::NonZeroUsize, ops::RangeInclusive};

// `Timestamp::now` reads the system clock, which only exists with `std`.
#[cfg(feature = "std")]
use jiff::Timestamp;
use jiff::{
    ToSpan,
    civil::{Date, Weekday},
};
use nami::watcher::BoxWatcherGuard;
use nami::{
    Binding, Computed, Signal, SignalExt, collection::SignalCollection, signal::IntoComputed,
};
use waterui_controls::label::{ControlLabel, Label, LabelKind, TitleLabel};
use waterui_controls::{IntoLabel, button, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{
    AnyView, Environment, Metadata, Native, NativeView, OneOf2, Retain, View, contract_violation,
    env::use_env, views::ForEach,
};
use waterui_graphics::color::Color;
use waterui_interaction::interaction::Disabled;
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

/// Validates a calendar date range: `start <= end`.
///
/// # Contract violation
///
/// A reversed range panics in debug builds and logs and clamps to
/// `start..=start` in release builds.
#[must_use]
pub fn sanitize_date_range(range: RangeInclusive<Date>) -> RangeInclusive<Date> {
    let (start, end) = (*range.start(), *range.end());
    if start <= end {
        return start..=end;
    }
    contract_violation!(
        "a calendar range must have start <= end, got {start}..={end}; clamping the range"
    );
    start..=start
}

/// Clamps `value` in place when it falls outside `range`.
pub fn clamp_date_to_range(value: &Binding<Date>, range: &RangeInclusive<Date>) {
    let range = sanitize_date_range(range.clone());
    let current = value.snapshot();
    let clamped = current.clamp(*range.start(), *range.end());
    if clamped != current {
        value.set(clamped);
    }
}

/// Configuration for the [`Calendar`] component — what a backend realizes
/// through its `calendar` method, or a hook targeting `CalendarConfig`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CalendarConfig<S> {
    label: ControlLabel<S>,
    value: Binding<Date>,
    range: Computed<RangeInclusive<Date>>,
    decorated: Computed<BTreeSet<Date>>,
    visible_month: Binding<Date>,
}

impl<S> CalendarConfig<S> {
    /// The label announced for the calendar and displayed above it.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: ControlLabel<S>) {
        self.label = label;
    }

    /// The label, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut ControlLabel<S> {
        &mut self.label
    }

    /// The selected date.
    #[must_use]
    pub const fn value(&self) -> &Binding<Date> {
        &self.value
    }

    /// The inclusive range of selectable dates.
    #[must_use]
    pub const fn range(&self) -> &Computed<RangeInclusive<Date>> {
        &self.range
    }

    /// Watches `range` and re-clamps `value` and `visible_month` in place
    /// whenever the range updates, returning the subscription guard.
    pub fn watch_range_clamp(&self) -> BoxWatcherGuard {
        let value = self.value.clone();
        let visible_month = self.visible_month.clone();
        let apply = move |range: &RangeInclusive<Date>| {
            let range = sanitize_date_range(range.clone());
            clamp_date_to_range(&value, &range);
            ensure_visible_month_in_range(&visible_month, Some(value.snapshot()), &range);
        };
        apply(&self.range.snapshot());
        self.range.watch(move |ctx| apply(ctx.value()))
    }

    /// The days marked with a passive decoration dot.
    #[must_use]
    pub const fn decorated(&self) -> &Computed<BTreeSet<Date>> {
        &self.decorated
    }

    /// The month the calendar shows: any day of it reads as that month, and
    /// the calendar writes the month's first day when the user navigates.
    #[must_use]
    pub const fn visible_month(&self) -> &Binding<Date> {
        &self.visible_month
    }
}

/// A calendar-style control for selecting a single date.
#[derive(Debug, Clone)]
#[must_use]
pub struct Calendar<L> {
    label: Label<L>,
    value: Binding<Date>,
    range: Computed<RangeInclusive<Date>>,
    decorated: Computed<BTreeSet<Date>>,
    visible_month: Binding<Date>,
}

impl<L: LabelKind> Calendar<L> {
    /// Returns the configuration for this calendar.
    #[must_use]
    pub fn config(self) -> CalendarConfig<Label<L>> {
        CalendarConfig {
            label: self.label.into_unscoped_control_label(),
            value: self.value,
            range: self.range,
            decorated: self.decorated,
            visible_month: self.visible_month,
        }
    }
}

impl<S: View> From<CalendarConfig<S>> for Calendar<ControlLabel<S>> {
    fn from(value: CalendarConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            value: value.value,
            range: value.range,
            decorated: value.decorated,
            visible_month: value.visible_month,
        }
    }
}

impl<S: 'static> NativeView for CalendarConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}

impl<S: 'static> MapSlots<S> for CalendarConfig<S> {
    type With<T: 'static> = CalendarConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> CalendarConfig<T> {
        CalendarConfig {
            label: self.label.map_slots(f),
            value: self.value,
            range: self.range,
            decorated: self.decorated,
            visible_month: self.visible_month,
        }
    }
}

impl<L> Calendar<L> {
    /// Creates a new `Calendar` with the given semantic label and selected date
    /// binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, date: &Binding<Date>) -> Self {
        let initial_range = Date::MIN..=Date::MAX;
        let visible_month = Binding::container(
            initial_visible_month(Some(date.snapshot()), &initial_range).first_day(),
        );
        Self {
            label: label.into_label(),
            value: date.clone(),
            range: Computed::constant(initial_range),
            decorated: Computed::constant(BTreeSet::new()),
            visible_month,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the valid date range.
    ///
    /// # Contract violation
    ///
    /// A reversed range panics in debug builds and logs and clamps in release
    /// builds.
    pub fn range(mut self, range: impl IntoComputed<RangeInclusive<Date>>) -> Self {
        let mapped = range.into_computed().map(sanitize_date_range).computed();
        let initial = mapped.snapshot();
        clamp_date_to_range(&self.value, &initial);
        ensure_visible_month_in_range(&self.visible_month, Some(self.value.snapshot()), &initial);
        self.range = mapped;
        self
    }

    /// Binds external control over the currently displayed month.
    pub fn visible_month(mut self, month: &Binding<Date>) -> Self {
        let visible_month = month.clone();
        ensure_visible_month_in_range(
            &visible_month,
            Some(self.value.snapshot()),
            &self.range.snapshot(),
        );
        self.visible_month = visible_month;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.decorated = decorated.into_computed();
        self
    }
}

impl_label_style_methods!(Calendar<L>);

/// Translation key for the control that shows the month before.
pub const CALENDAR_PREVIOUS_MONTH_LABEL_KEY: &str = "waterui.calendar.previous_month";
/// Translation key for the control that shows the month of the selection.
pub const CALENDAR_SELECTED_MONTH_LABEL_KEY: &str = "waterui.calendar.selected_month";
/// Translation key for the control that shows the month after.
pub const CALENDAR_NEXT_MONTH_LABEL_KEY: &str = "waterui.calendar.next_month";

/// Semantic label for a calendar's previous-month control.
///
/// Backends that draw month navigation speak these labels rather than
/// strings of their own, so one catalog entry translates them everywhere.
#[must_use]
pub fn calendar_previous_month_label() -> Label<TitleLabel> {
    Text::localized_or(CALENDAR_PREVIOUS_MONTH_LABEL_KEY, "Previous Month").into_label()
}

/// Semantic label for a calendar's control that returns to the month of the
/// selection. See [`calendar_previous_month_label`].
#[must_use]
pub fn calendar_selected_month_label() -> Label<TitleLabel> {
    Text::localized_or(CALENDAR_SELECTED_MONTH_LABEL_KEY, "Selected Month").into_label()
}

/// Semantic label for a calendar's next-month control. See
/// [`calendar_previous_month_label`].
#[must_use]
pub fn calendar_next_month_label() -> Label<TitleLabel> {
    Text::localized_or(CALENDAR_NEXT_MONTH_LABEL_KEY, "Next Month").into_label()
}

impl<L: LabelKind> Calendar<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> CalendarConfig<S> {
        CalendarConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
            range: self.range,
            decorated: self.decorated,
            visible_month: self.visible_month,
        }
    }
}

impl<L: LabelKind> View for Calendar<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

/// The calendar composed from `WaterUI` views: how a self-drawn backend
/// realizes [`CalendarConfig`].
#[derive(Debug)]
pub struct CalendarGrid<S>(CalendarConfig<S>);

impl<S> CalendarGrid<S> {
    /// The grid for `config`.
    #[must_use]
    pub const fn new(config: CalendarConfig<S>) -> Self {
        Self(config)
    }

    /// Splits this grid into its resolved label and its composed calendar body.
    pub fn into_parts(self, env: &Environment) -> (ControlLabel<S>, impl View + use<S>) {
        let range_guard = self.0.watch_range_clamp();
        let CalendarConfig {
            label,
            value: selection,
            range,
            decorated,
            visible_month,
        } = self.0;
        let visible_month = map_visible_month_binding(&visible_month);
        let locale = locale_binding(env).computed();

        let cell_range = range.clone();
        let cell_selection = selection;
        let cell_decorated = decorated;
        let rows = calendar_rows(&visible_month, move |cell| {
            single_day_cell_content(cell, &cell_range, cell_selection.clone(), &cell_decorated)
        });
        let calendar = Metadata::new(
            CalendarBody::new(locale, range, visible_month, rows),
            Retain::new(range_guard),
        );
        (label, calendar)
    }
}

impl<S: View> View for CalendarGrid<S> {
    fn body(self, env: &Environment) -> impl View {
        let (label, calendar) = self.into_parts(env);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, waterui_macros::Identifiable)]
pub(crate) struct DayCell {
    #[id]
    identity: (Date, bool),
    pub(crate) date: Date,
    pub(crate) in_current_month: bool,
}

pub(crate) struct CalendarBody<Content> {
    locale: Computed<Locale>,
    range: Computed<RangeInclusive<Date>>,
    visible_month: Binding<VisibleMonth>,
    content: Content,
}

impl<Content> CalendarBody<Content> {
    pub(crate) const fn new(
        locale: Computed<Locale>,
        range: Computed<RangeInclusive<Date>>,
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
                build_month_header(&self.locale, self.visible_month, &self.range),
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

pub(crate) fn ensure_visible_month_in_range(
    visible_month: &Binding<Date>,
    selected: Option<Date>,
    range: &RangeInclusive<Date>,
) {
    let visible = VisibleMonth::from_date(visible_month.snapshot());
    if !visible_month_in_range(visible, range) {
        visible_month.set(initial_visible_month(selected, range).first_day());
    }
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
    range: &Computed<RangeInclusive<Date>>,
) -> impl View + use<> {
    let title = Text::computed(
        locale
            .zip(&visible_month)
            .map(|(locale, month)| {
                StyledStr::plain(format_calendar_month_year(&locale, &month.first_day()))
            })
            .computed(),
    )
    .headline();
    let previous_disabled = visible_month
        .zip(range)
        .map(|(month, range)| month.previous().first_day() < month_start(*range.start()))
        .computed();
    let next_disabled = visible_month
        .zip(range)
        .map(|(month, range)| month.next().first_day() > month_start(*range.end()))
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
    let content = button(label).borderless().action(move || {
        let mut month = visible_month.get_mut();
        let next = step(*month);
        *month = next;
    });
    // Disabled is a scoped environment attribute: install it for the subtree and
    // let the button read the state in force at its own position. `Metadata`
    // carries the extended environment across every backend, native included.
    use_env(move |mut env: Environment| {
        Disabled::install(&mut env, disabled);
        Metadata::new(content, env)
    })
}

/// Wraps day-grid content in its column cell: ideally [`CALENDAR_DAY_SIZE`]
/// wide but free to shrink with its row, so the seven columns compress evenly
/// on layouts narrower than the full grid instead of overflowing the calendar.
fn day_grid_cell<C: View>(content: C, height: f32) -> Frame<C> {
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
    range: &Computed<RangeInclusive<Date>>,
    selection: Binding<Date>,
    decorated_dates: &Computed<BTreeSet<Date>>,
) -> impl View + use<> {
    let selected = selection
        .map(move |selected_date| selected_date == cell.date)
        .computed();
    let decorated = decorated_dates
        .map(move |dates| dates.contains(&cell.date))
        .computed();

    if cell.in_current_month {
        let disabled = range
            .map(move |range| !range.contains(&cell.date))
            .computed();
        let range = range.clone();
        OneOf2::A(selectable_day_cell(
            cell.date,
            &decorated,
            &selected,
            disabled,
            move || {
                if range.snapshot().contains(&cell.date) {
                    selection.set(cell.date);
                }
            },
        ))
    } else {
        OneOf2::B(day_grid_cell(
            day_cell_placeholder(cell, &decorated),
            CALENDAR_DAY_SIZE,
        ))
    }
}

#[cfg(feature = "multi-date-picker")]
pub(crate) fn multi_day_cell_content(
    cell: DayCell,
    range: &Computed<RangeInclusive<Date>>,
    selection: Binding<BTreeSet<Date>>,
    decorated_dates: &Computed<BTreeSet<Date>>,
) -> impl View + use<> {
    let selected = selection
        .map(move |dates| dates.contains(&cell.date))
        .computed();
    let decorated = decorated_dates
        .map(move |dates| dates.contains(&cell.date))
        .computed();

    if cell.in_current_month {
        let disabled = range
            .map(move |range| !range.contains(&cell.date))
            .computed();
        let range = range.clone();
        OneOf2::A(selectable_day_cell(
            cell.date,
            &decorated,
            &selected,
            disabled,
            move || {
                if range.snapshot().contains(&cell.date) {
                    selection.with_mut(|dates| {
                        if !dates.insert(cell.date) {
                            dates.remove(&cell.date);
                        }
                    });
                }
            },
        ))
    } else {
        OneOf2::B(day_grid_cell(
            day_cell_placeholder(cell, &decorated),
            CALENDAR_DAY_SIZE,
        ))
    }
}

fn selection_fill_color(selected: &Computed<bool>) -> Color {
    Color::tint().with_opacity(selected.map(|selected| if selected { 1.0 } else { 0.0 }))
}

fn selectable_day_cell<F: FnMut() + 'static>(
    date: Date,
    decorated: &Computed<bool>,
    selected: &Computed<bool>,
    disabled: Computed<bool>,
    action: F,
) -> impl View + use<F> {
    let foreground = selected.map(|selected| {
        if selected {
            Color::accent_foreground()
        } else {
            Color::foreground()
        }
    });
    let button = button(day_cell_label(date, decorated).color(foreground))
        .plain()
        .a11y_label(day_cell_accessibility_label(date))
        .action(action);
    let content = day_grid_cell(button, CALENDAR_DAY_SIZE);

    let cell = BackgroundView::new(
        content,
        Frame::new(Circle.fill(selection_fill_color(selected)))
            .width(CALENDAR_SELECTED_DAY_SIZE)
            .height(CALENDAR_SELECTED_DAY_SIZE),
    );
    use_env(move |mut env: Environment| {
        Disabled::install(&mut env, disabled);
        Metadata::new(cell, env)
    })
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

fn day_cell_placeholder(cell: DayCell, decorated: &Computed<bool>) -> impl View + use<> {
    if cell.in_current_month {
        OneOf2::A(day_cell_label(cell.date, decorated).caption())
    } else {
        OneOf2::B(Text::new(String::new()).caption())
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
    use waterui_core::Environment;
    use waterui_graphics::color::{
        Tint, Working, WorkingColor, install_color_signal, token::Accent,
    };

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

    #[test]
    fn owned_visible_month_defaults_to_selection_and_opt_in_replaces_it() {
        let selected = Binding::container(Date::new(2025, 6, 15).unwrap());
        let default_cfg = Calendar::new("Trip", &selected).config();
        assert_eq!(
            default_cfg.visible_month().snapshot(),
            Date::new(2025, 6, 1).unwrap()
        );

        let shown = Binding::container(Date::new(2025, 10, 12).unwrap());
        let opt_in_cfg = Calendar::new("Trip", &selected)
            .range(Date::new(2025, 1, 1).unwrap()..=Date::new(2025, 8, 31).unwrap())
            .visible_month(&shown)
            .config();
        assert_eq!(
            opt_in_cfg.visible_month().snapshot(),
            Date::new(2025, 6, 1).unwrap()
        );
        assert_eq!(shown.snapshot(), Date::new(2025, 6, 1).unwrap());
    }

    #[test]
    fn selection_fill_follows_tint_and_falls_back_to_accent() {
        let accent = WorkingColor::new([0.0, 0.2, 1.0, 1.0]);
        let tint = WorkingColor::new([1.0, 0.1, 0.1, 1.0]);
        let mut env = Environment::new();
        install_color_signal::<Accent>(&mut env, Computed::constant(accent));

        let selected = Binding::bool(true).computed();
        let fill = selection_fill_color(&selected);

        assert_eq!(fill.resolve(&env).snapshot().components, accent.components);

        let tinted_env = env.extending(Tint::new(Color::new(Working(tint))));
        assert_eq!(
            fill.resolve(&tinted_env).snapshot().components,
            tint.components
        );
    }
}
