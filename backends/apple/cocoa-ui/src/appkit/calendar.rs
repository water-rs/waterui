//! The `AppKit` month calendar: a composition of `AppKit` controls laid out
//! like the graphical `NSDatePicker` (`NSDatePickerStyleClockAndCalendar`
//! with only the date elements), carrying what that control cannot — a
//! decoration dot under a day.
//!
//! The month title is an `NSTextField`; ◀ ● ▶ are borderless symbol
//! `NSButton`s in an `NSStackView`; the weekday symbols and the six weeks
//! of days share one `NSGridView`, whose cells are borderless `NSButton`s
//! with a layer-backed dot view. `AppKit` therefore draws the text in the
//! locale's digits, hit-tests, moves keyboard focus and draws its ring,
//! mirrors the layout right to left, and exposes each control to
//! assistive technology. Only the placement inside the control is ours: the
//! graphical picker's metrics, measured from `NSDatePicker` renders on
//! macOS 27.
//!
//! Days outside the visible month draw in the tertiary label color, days
//! outside the range are disabled buttons and draw as `AppKit` draws
//! disabled control text, and today's number draws in the accent color. A selected day sits in a
//! rounded highlight: the selection color while a day of the calendar has
//! keyboard focus in the key window, the unemphasized selection color
//! otherwise. Navigation stays inside the range: ◀ and ▶ disable at its
//! first and last month, and ● brings back the month of the selection.
//!
//! The calendar selects one day or many ([`CalendarSelectionMode`]).
//! Selection is owned by the caller: a click or Space on the focused day
//! reports the day through [`CalendarView::on_pick`] — and so does an
//! arrow key with one day selected, or Return with many, where the arrows
//! only move focus — and the caller pushes the resulting selection back
//! through
//! [`CalendarView::set_selection`]. Month navigation is local and reported
//! through [`CalendarView::on_visible_month_change`].
//!
//! # Safety
//!
//! The `unsafe` here defines `NSView` and `NSButton` subclasses, reads
//! framework-exported constants, and calls `objc2` bindings marked unsafe
//! because `AppKit` view APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee. Every override `AppKit` calls runs inside the panic guard.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::ops::RangeInclusive;
use std::rc::{Rc, Weak as RcWeak};

use keyboard_types::{Key, NamedKey};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityGroupRole, NSAccessibilityPostNotification,
    NSAccessibilitySelectedChildrenChangedNotification, NSAccessibilityValueChangedNotification,
    NSAttributedStringNSStringDrawing, NSBezierPath, NSBox, NSBoxType, NSButton,
    NSCellImagePosition, NSColor, NSEvent, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSGridCellPlacement, NSGridView, NSImage,
    NSImageSymbolConfiguration, NSMutableParagraphStyle, NSParagraphStyleAttributeName,
    NSResponder, NSStackView, NSStringDrawing, NSTextAlignment, NSTextField, NSTitlePosition,
    NSView, NSWindowDidBecomeKeyNotification, NSWindowDidResignKeyNotification,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSCalendar, NSDate, NSDateFormatter,
    NSDateFormatterStyle, NSDictionary, NSLocale, NSObjectProtocol, NSRect, NSString,
};

use crate::action::ActionTarget;
use crate::callback::guarded;
use crate::date::{self, CalendarSelectionMode, DateParts, YearMonth};
use crate::font;
use crate::geometry::Size;
use crate::notification::{self, NotificationName, NotificationObserver};
use crate::view::is_right_to_left;

/// The control's size — the graphical `NSDatePicker`'s
/// `intrinsicContentSize` with only the date elements.
const WIDTH: f64 = 138.5;
/// See [`WIDTH`].
const HEIGHT: f64 = 148.0;
/// The leading edge of the first grid column.
const GRID_LEADING: f64 = 4.5;
/// The top of the grid: the weekday row, whose symbols sit on
/// [`WEEKDAY_BASELINE`], then the week rows from [`FIRST_WEEK_TOP`].
const GRID_TOP: f64 = 21.0;
/// The top of the first week row.
const FIRST_WEEK_TOP: f64 = 39.0;
/// The pitch between grid columns.
const COLUMN_WIDTH: f64 = 18.5;
/// The pitch between week rows.
const ROW_HEIGHT: f64 = 18.0;
/// A day button's width: its selection highlight's.
const DAY_WIDTH: f64 = 18.0;
/// A day button's height: its selection highlight's.
const DAY_HEIGHT: f64 = 16.0;
/// The selection highlight's corner radius.
const HIGHLIGHT_RADIUS: f64 = 3.0;
/// The month title's leading inset.
const TITLE_LEADING: f64 = 3.0;
/// The month title field's first-baseline anchor, which puts the drawn
/// baseline where the graphical picker's is.
const TITLE_BASELINE: f64 = 14.25;
/// The weekday symbols' baseline.
const WEEKDAY_BASELINE: f64 = 33.25;
/// The separator under the weekday symbols: top, leading edge, width and
/// thickness.
const SEPARATOR: (f64, f64, f64, f64) = (37.0, 3.0, 132.5, 1.0);
/// A navigation button's width — the pitch of ◀ ● ▶ — and height.
const NAV_SIZE: (f64, f64) = (14.5, 14.0);
/// The navigation buttons' vertical centre and trailing inset.
const NAV_CENTER_Y: f64 = 10.5;
/// See [`NAV_CENTER_Y`].
const NAV_TRAILING: f64 = 1.75;
/// The navigation glyphs' symbol point size.
const NAV_SYMBOL_SIZE: f64 = 7.0;
/// The decoration dot's diameter.
const DOT_DIAMETER: f64 = 2.5;
/// The decoration dot's centre below its day button's top.
const DOT_CENTER: f64 = 15.0;
/// The title's point size and weight.
const TITLE_FONT: (f64, f64) = (13.0, font::weight::BOLD);
/// The weekday symbols' point size and weight.
const WEEKDAY_FONT: (f64, f64) = (10.5, font::weight::SEMI_BOLD);
/// The day numbers' point size and weight.
const DAY_FONT: (f64, f64) = (11.0, font::weight::REGULAR);
/// The days the grid shows: six weeks.
const GRID_DAYS: usize = 42;
/// A Sunday — the reference day the weekday symbols are formatted from.
const REFERENCE_SUNDAY: DateParts = DateParts {
    year: 2023,
    month: 1,
    day: 1,
};

type TextAttributes = NSDictionary<NSAttributedStringKey, AnyObject>;

/// The theme colours the calendar draws with. The faint text tiers and the
/// unemphasized highlight are `AppKit`'s own system colours, which follow
/// the view's appearance.
#[derive(Debug, Clone)]
pub struct CalendarColors {
    /// Day numbers and the month title.
    pub foreground: Retained<NSColor>,
    /// Weekday symbols, navigation glyphs and decoration dots.
    pub muted: Retained<NSColor>,
    /// The line under the weekday symbols.
    pub separator: Retained<NSColor>,
    /// Today's number.
    pub accent: Retained<NSColor>,
    /// The focused selection highlight.
    pub selection: Retained<NSColor>,
    /// Text and dots on the focused selection highlight.
    pub selection_foreground: Retained<NSColor>,
}

/// One of the colours in [`CalendarColors`], for [`CalendarView::set_color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarColorRole {
    /// [`CalendarColors::foreground`].
    Foreground,
    /// [`CalendarColors::muted`].
    Muted,
    /// [`CalendarColors::separator`].
    Separator,
    /// [`CalendarColors::accent`].
    Accent,
    /// [`CalendarColors::selection`].
    Selection,
    /// [`CalendarColors::selection_foreground`].
    SelectionForeground,
}

/// One of the navigation buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarNavigation {
    /// ◀: shows the month before.
    Previous,
    /// ●: shows the month of the selection.
    Selection,
    /// ▶: shows the month after.
    Next,
}

/// The spoken names of the three navigation buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarNavigationLabels {
    /// ◀: shows the month before.
    pub previous: String,
    /// ●: shows the month of the selection.
    pub selection: String,
    /// ▶: shows the month after.
    pub next: String,
}

/// One grid cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridDay {
    parts: DateParts,
    in_month: bool,
}

/// The six weeks showing `month`, starting on `first_weekday` (`1` is
/// Sunday).
fn month_grid(month: YearMonth, first_weekday: i32) -> [GridDay; GRID_DAYS] {
    let lead = (month.first_weekday() - first_weekday).rem_euclid(7);
    let previous = month.previous();
    let previous_count = previous.day_count();
    let count = month.day_count();
    let next = month.next();
    std::array::from_fn(|index| {
        let offset = i32::try_from(index).expect("42 grid days") - lead;
        if offset < 0 {
            GridDay {
                parts: DateParts {
                    day: previous_count + offset + 1,
                    ..previous.first_day()
                },
                in_month: false,
            }
        } else if offset < count {
            GridDay {
                parts: DateParts {
                    day: offset + 1,
                    ..month.first_day()
                },
                in_month: true,
            }
        } else {
            GridDay {
                parts: DateParts {
                    day: offset - count + 1,
                    ..next.first_day()
                },
                in_month: false,
            }
        }
    })
}

/// The locale-derived text the calendar shows and speaks.
struct LocaleText {
    /// The locale's Gregorian calendar, which also turns days into dates.
    calendar: Retained<NSCalendar>,
    /// The abbreviated month and year, e.g. "Nov 2026".
    title: Retained<NSDateFormatter>,
    /// The day of the month in the locale's digits.
    day: Retained<NSDateFormatter>,
    /// The full date a day button speaks.
    spoken: Retained<NSDateFormatter>,
    /// The weekday symbols, Sunday first.
    weekdays: [String; 7],
    /// The locale's first weekday, `1` (Sunday) through `7`.
    first_weekday: i32,
}

impl LocaleText {
    /// The text of the locale `identifier`, with weekday symbols that fit a
    /// column in `weekday_font`.
    fn new(identifier: &str, weekday_font: &NSFont) -> Self {
        let locale = NSLocale::localeWithLocaleIdentifier(&NSString::from_str(identifier));
        let calendar = date::gregorian();
        calendar.setLocale(Some(&locale));
        let formatter = |configure: &dyn Fn(&NSDateFormatter)| {
            let formatter = NSDateFormatter::new();
            formatter.setLocale(Some(&locale));
            formatter.setCalendar(Some(&calendar));
            configure(&formatter);
            formatter
        };
        let title =
            formatter(&|f| f.setLocalizedDateFormatFromTemplate(&NSString::from_str("MMMyyyy")));
        let day = formatter(&|f| f.setDateFormat(Some(&NSString::from_str("d"))));
        let spoken = formatter(&|f| {
            f.setDateStyle(NSDateFormatterStyle::FullStyle);
            f.setTimeStyle(NSDateFormatterStyle::NoStyle);
        });
        let symbol_formatters = [
            formatter(&|f| f.setDateFormat(Some(&NSString::from_str("cccccc")))),
            formatter(&|f| f.setDateFormat(Some(&NSString::from_str("ccccc")))),
        ];
        let first_weekday =
            i32::try_from(calendar.firstWeekday()).expect("firstWeekday is 1 through 7");
        let mut text = Self {
            calendar,
            title,
            day,
            spoken,
            weekdays: Default::default(),
            first_weekday,
        };
        // The graphical `NSDatePicker` heads its columns with the two-letter
        // symbols ("Su") when all of them fit a column, and with the narrow
        // ones otherwise ("周日" does not, so Chinese gets "日").
        let symbols = |symbol: &NSDateFormatter| {
            std::array::from_fn::<_, 7, _>(|offset| {
                let sunday_offset = i32::try_from(offset).expect("seven weekdays");
                symbol.stringFromDate(&text.date(date::add_days(REFERENCE_SUNDAY, sunday_offset)))
            })
        };
        let [two_letter, narrow] = &symbol_formatters;
        // SAFETY: the font attribute name is a static the platform exports,
        // and its value is the `NSFont` it documents.
        let attributes: Retained<TextAttributes> =
            unsafe { NSDictionary::from_slices(&[NSFontAttributeName], &[weekday_font.as_ref()]) };
        let short = symbols(two_letter);
        // SAFETY: `sizeWithAttributes:` is `NSStringDrawing`'s documented
        // measure; the dictionary carries the documented value types.
        let fits = short.iter().all(
            |symbol| unsafe { symbol.sizeWithAttributes(Some(&attributes)) }.width <= COLUMN_WIDTH,
        );
        let chosen = if fits { short } else { symbols(narrow) };
        text.weekdays = chosen.map(|symbol| symbol.to_string());
        text
    }

    /// The `NSDate` at midnight of `day` in the locale's calendar.
    ///
    /// # Panics
    ///
    /// When `day` is not a real Gregorian date.
    fn date(&self, day: DateParts) -> Retained<NSDate> {
        self.calendar
            .dateFromComponents(&date::components(day))
            .expect("calendar days are valid Gregorian dates")
    }
}

/// The calendar's model.
struct Model {
    month: YearMonth,
    range: RangeInclusive<DateParts>,
    grid: [GridDay; GRID_DAYS],
    selected: BTreeSet<DateParts>,
    decorated: BTreeSet<DateParts>,
    text: LocaleText,
    colors: CalendarColors,
    /// Each grid day's number in the locale's digits.
    numbers: Vec<Retained<NSString>>,
}

impl Model {
    fn in_range(&self, day: DateParts) -> bool {
        self.range.contains(&day)
    }

    /// The first and last month the range touches.
    const fn month_bounds(&self) -> (YearMonth, YearMonth) {
        (
            YearMonth::of(*self.range.start()),
            YearMonth::of(*self.range.end()),
        )
    }

    /// `month` moved into the range's months.
    fn clamp(&self, month: YearMonth) -> YearMonth {
        let (first, last) = self.month_bounds();
        month.clamp(first, last)
    }

    /// The month ● brings back: the earliest selected day's, or today's
    /// when nothing is selected, inside the range.
    fn selection_month(&self) -> YearMonth {
        self.clamp(YearMonth::of(
            self.selected.first().copied().unwrap_or_else(date::today),
        ))
    }

    fn index_of(&self, day: DateParts) -> Option<usize> {
        self.grid.iter().position(|cell| cell.parts == day)
    }
}

/// What a day button shows.
struct DayLook {
    title: Retained<NSAttributedString>,
    highlight: Option<Retained<NSColor>>,
    dot: Option<Retained<NSColor>>,
}

/// Called with the day the user picks.
type PickHandler = Rc<dyn Fn(DateParts)>;
/// Called with the month the user navigates to.
type MonthHandler = Rc<dyn Fn(YearMonth)>;

/// The views and state every control of one calendar shares.
struct Shared {
    /// Whether a pick replaces the selection or toggles a day, and so
    /// whether an arrow key picks or only moves focus.
    mode: CalendarSelectionMode,
    root: Retained<CalendarRoot>,
    title: Retained<NSTextField>,
    navigation: [Retained<NSButton>; 3],
    weekdays: Vec<Retained<NSTextField>>,
    separator: Retained<NSBox>,
    days: Vec<Retained<DayButton>>,
    model: RefCell<Model>,
    /// Whether a day button is the first responder.
    focused: Cell<bool>,
    /// Whether the selection highlight currently draws emphasized.
    emphasized: Cell<bool>,
    on_pick: RefCell<Option<PickHandler>>,
    on_visible_month_change: RefCell<Option<MonthHandler>>,
    actions: RefCell<Vec<ActionTarget>>,
    window_observers: RefCell<Vec<NotificationObserver>>,
}

impl Shared {
    fn mtm(&self) -> MainThreadMarker {
        self.root.mtm()
    }

    fn navigation_button(&self, button: CalendarNavigation) -> &NSButton {
        let index = match button {
            CalendarNavigation::Previous => 0,
            CalendarNavigation::Selection => 1,
            CalendarNavigation::Next => 2,
        };
        &self.navigation[index]
    }

    /// Whether the selection highlight is the focused one: a day has
    /// keyboard focus in the key window.
    fn is_emphasized(&self) -> bool {
        self.focused.get()
            && self
                .root
                .window()
                .is_some_and(|window| window.isKeyWindow())
    }

    /// Restyles the selected days when the emphasis changed.
    fn refresh_emphasis(&self) {
        let emphasized = self.is_emphasized();
        if self.emphasized.replace(emphasized) == emphasized {
            return;
        }
        let indices: Vec<usize> = {
            let model = self.model.borrow();
            model
                .selected
                .iter()
                .filter_map(|day| model.index_of(*day))
                .collect()
        };
        self.refresh_days(indices);
    }

    /// Lays out the model's month: the title, the navigation, and every
    /// day's number, state and spoken name.
    fn show_model_month(&self) {
        {
            let mut model = self.model.borrow_mut();
            model.grid = month_grid(model.month, model.text.first_weekday);
            let numbers = model
                .grid
                .iter()
                .map(|cell| model.text.day.stringFromDate(&model.text.date(cell.parts)))
                .collect();
            model.numbers = numbers;
            let title = model
                .text
                .title
                .stringFromDate(&model.text.date(model.month.first_day()));
            self.title.setStringValue(&title);
            let (first, last) = model.month_bounds();
            self.navigation_button(CalendarNavigation::Previous)
                .setEnabled(model.month > first);
            self.navigation_button(CalendarNavigation::Next)
                .setEnabled(model.month < last);
            for (button, cell) in self.days.iter().zip(&model.grid) {
                button.setEnabled(model.in_range(cell.parts));
                let spoken = model
                    .text
                    .spoken
                    .stringFromDate(&model.text.date(cell.parts));
                button.setAccessibilityLabel(Some(&spoken));
            }
        }
        self.refresh_days(0..GRID_DAYS);
        post_accessibility_notification(&self.title, AccessibilityNotification::ValueChanged);
    }

    /// Restyles the days at `indices`.
    fn refresh_days(&self, indices: impl IntoIterator<Item = usize>) {
        let emphasized = self.emphasized.get();
        let rtl = is_right_to_left(&self.root);
        let model = self.model.borrow();
        let today = date::today();
        let font = system_font(self.mtm(), DAY_FONT);
        for index in indices {
            let look = day_look(&model, index, emphasized, today, &font, rtl);
            let button = &self.days[index];
            button.show(&look, rtl);
            button.setAccessibilitySelected(model.selected.contains(&model.grid[index].parts));
        }
    }

    /// The weekday symbols in the locale's order and the muted colour.
    fn refresh_weekdays(&self) {
        let model = self.model.borrow();
        let font = system_font(self.mtm(), WEEKDAY_FONT);
        for (column, field) in self.weekdays.iter().enumerate() {
            let weekday = (usize::try_from(model.text.first_weekday - 1)
                .expect("firstWeekday is 1 through 7")
                + column)
                % 7;
            field.setStringValue(&NSString::from_str(&model.text.weekdays[weekday]));
            field.setFont(Some(&font));
            field.setTextColor(Some(&model.colors.muted));
        }
    }

    /// Shows `month` and reports it, unless it is already shown.
    fn navigate(&self, month: YearMonth) {
        {
            let mut model = self.model.borrow_mut();
            if model.month == month {
                return;
            }
            model.month = month;
        }
        self.show_model_month();
        let handler = self.on_visible_month_change.borrow().clone();
        if let Some(handler) = handler {
            handler(month);
        }
    }

    /// Activates a navigation button.
    fn activate(&self, button: CalendarNavigation) {
        let month = {
            let model = self.model.borrow();
            match button {
                CalendarNavigation::Previous => model.clamp(model.month.previous()),
                CalendarNavigation::Selection => model.selection_month(),
                CalendarNavigation::Next => model.clamp(model.month.next()),
            }
        };
        self.navigate(month);
    }

    /// Picks `day`: shows its month, focuses its button and reports it.
    fn pick(&self, day: DateParts) {
        self.move_to(day);
        let handler = self.on_pick.borrow().clone();
        if let Some(handler) = handler {
            handler(day);
        }
    }

    /// Moves keyboard focus to `day`, showing its month.
    fn move_to(&self, day: DateParts) {
        self.navigate(YearMonth::of(day));
        let index = self
            .model
            .borrow()
            .index_of(day)
            .expect("the shown month's grid holds the day");
        self.focus(index);
    }

    fn focus(&self, index: usize) {
        if let Some(window) = self.root.window() {
            let responder: &NSResponder = &self.days[index];
            window.makeFirstResponder(Some(responder));
        }
    }

    /// Arrow keys pick the neighbouring day; Page Up and Page Down change
    /// month.
    fn handle_key(&self, index: usize, event: &NSEvent) -> bool {
        let key = crate::keys::key_event(event).key;
        let rtl = is_right_to_left(&self.root);
        let step = match key {
            Key::Named(NamedKey::ArrowLeft) => Some(if rtl { 1 } else { -1 }),
            Key::Named(NamedKey::ArrowRight) => Some(if rtl { -1 } else { 1 }),
            Key::Named(NamedKey::ArrowUp) => Some(-7),
            Key::Named(NamedKey::ArrowDown) => Some(7),
            _ => None,
        };
        if let Some(step) = step {
            let target = {
                let model = self.model.borrow();
                let day = date::add_days(model.grid[index].parts, step);
                model.in_range(day).then_some(day)
            };
            if let Some(day) = target {
                match self.mode {
                    CalendarSelectionMode::Single => self.pick(day),
                    CalendarSelectionMode::Multiple => self.move_to(day),
                }
            }
            return true;
        }
        match key {
            // Space presses the focused day button; Return toggles it too,
            // as the multi-date calendars of the other platforms do.
            Key::Named(NamedKey::Enter) if self.mode == CalendarSelectionMode::Multiple => {
                let day = self.model.borrow().grid[index].parts;
                self.pick(day);
                true
            }
            Key::Named(NamedKey::PageUp) => {
                self.activate(CalendarNavigation::Previous);
                true
            }
            Key::Named(NamedKey::PageDown) => {
                self.activate(CalendarNavigation::Next);
                true
            }
            _ => false,
        }
    }

    /// Re-evaluates the emphasis whenever the window the calendar is now
    /// in becomes or stops being key.
    fn observe_window(self: &Rc<Self>) {
        let mut observers = self.window_observers.borrow_mut();
        observers.clear();
        let Some(window) = self.root.window() else {
            return;
        };
        // SAFETY: both names are immutable `NSString` statics `AppKit`
        // exports.
        let names = unsafe {
            [
                NSWindowDidBecomeKeyNotification,
                NSWindowDidResignKeyNotification,
            ]
        };
        for name in names {
            let weak = Rc::downgrade(self);
            observers.push(notification::observe_object(
                self.mtm(),
                &NotificationName::framework(name),
                AsRef::<AnyObject>::as_ref(&*window),
                move || {
                    if let Some(shared) = weak.upgrade() {
                        shared.refresh_emphasis();
                    }
                },
            ));
        }
    }
}

/// What a day shows given the model and the emphasis.
fn day_look(
    model: &Model,
    index: usize,
    emphasized: bool,
    today: DateParts,
    font: &NSFont,
    rtl: bool,
) -> DayLook {
    let cell = model.grid[index];
    let colors = &model.colors;
    let selected = model.selected.contains(&cell.parts);
    let in_range = model.in_range(cell.parts);
    let on_highlight = selected && emphasized;
    let highlight = selected.then(|| {
        if emphasized {
            colors.selection.clone()
        } else {
            NSColor::unemphasizedSelectedContentBackgroundColor()
        }
    });
    let text_color = if on_highlight {
        colors.selection_foreground.clone()
    } else if !in_range {
        // What `AppKit` draws a disabled button's title in.
        NSColor::disabledControlTextColor()
    } else if !cell.in_month {
        NSColor::tertiaryLabelColor()
    } else if cell.parts == today {
        colors.accent.clone()
    } else {
        colors.foreground.clone()
    };
    let dot = model.decorated.contains(&cell.parts).then(|| {
        if on_highlight {
            colors.selection_foreground.clone()
        } else if cell.in_month && in_range {
            colors.muted.clone()
        } else {
            NSColor::tertiaryLabelColor()
        }
    });
    DayLook {
        title: attributed(&model.numbers[index], &text_color, font, rtl),
        highlight,
        dot,
    }
}

/// `text` in `font` and `color`, aligned to the trailing edge.
fn attributed(
    text: &NSString,
    color: &NSColor,
    font: &NSFont,
    rtl: bool,
) -> Retained<NSAttributedString> {
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setAlignment(if rtl {
        NSTextAlignment::Left
    } else {
        NSTextAlignment::Right
    });
    // SAFETY: the three attribute names are statics the platform exports,
    // and each value is the type its attribute documents.
    let attributes: Retained<TextAttributes> = unsafe {
        NSDictionary::from_slices(
            &[
                NSFontAttributeName,
                NSForegroundColorAttributeName,
                NSParagraphStyleAttributeName,
            ],
            &[font.as_ref(), color.as_ref(), paragraph.as_ref()],
        )
    };
    // SAFETY: the attributes carry the value types their names document.
    unsafe { NSAttributedString::new_with_attributes(text, &attributes) }
}

fn system_font(mtm: MainThreadMarker, (size, weight): (f64, f64)) -> Retained<NSFont> {
    font::system(mtm, size, weight)
}

/// The two notifications the calendar posts for assistive technology.
#[derive(Clone, Copy)]
enum AccessibilityNotification {
    /// The month title changed.
    ValueChanged,
    /// The selected days changed.
    SelectedChildrenChanged,
}

fn post_accessibility_notification(element: &NSView, notification: AccessibilityNotification) {
    // SAFETY: both names are immutable `NSString` statics `AppKit` exports,
    // and the element is a live view on the main thread.
    unsafe {
        let name = match notification {
            AccessibilityNotification::ValueChanged => NSAccessibilityValueChangedNotification,
            AccessibilityNotification::SelectedChildrenChanged => {
                NSAccessibilitySelectedChildrenChangedNotification
            }
        };
        NSAccessibilityPostNotification(element, name);
    }
}

/// A [`CalendarRoot`]'s link to its calendar.
#[derive(Default)]
struct RootIvars {
    shared: RefCell<RcWeak<Shared>>,
}

impl fmt::Debug for RootIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RootIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `CalendarView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiCalendar"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RootIvars]
    #[derive(Debug)]
    /// The view holding a calendar's controls.
    struct CalendarRoot;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for CalendarRoot {}

    impl CalendarRoot {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            guarded("CalendarRoot viewDidMoveToWindow", || {
                if let Some(shared) = self.ivars().shared.borrow().upgrade() {
                    shared.observe_window();
                    shared.refresh_emphasis();
                }
            });
        }
    }
);

/// A [`DayButton`]'s position, link to its calendar, highlight and dot.
struct DayIvars {
    index: usize,
    shared: RefCell<RcWeak<Shared>>,
    highlight: RefCell<Option<Retained<NSColor>>>,
    dot: Retained<Dot>,
}

impl fmt::Debug for DayIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DayIvars")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSButton`'s designated initializer is `initWithFrame:`, which
    // `DayButton::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSButton))]
    #[name = "CocoaUiCalendarDay"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DayIvars]
    #[derive(Debug)]
    /// A day of the grid: a borderless button drawing its selection
    /// highlight behind its number, with a decoration dot under it.
    struct DayButton;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSButton` subclass.
    unsafe impl NSObjectProtocol for DayButton {}

    impl DayButton {
        // A day takes focus like the graphical date picker does, without
        // full keyboard access.
        // SAFETY: see the module safety note.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        // The calendar is one stop in the key-view loop: its selected day,
        // or its first selectable day of the month.
        // SAFETY: see the module safety note.
        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool {
            guarded("DayButton canBecomeKeyView", || {
                self.shared()
                    .is_some_and(|shared| shared_anchor(&shared) == Some(self.ivars().index))
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            // SAFETY: forwarding to the superclass is the responder contract.
            let accepted: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if accepted {
                guarded("DayButton becomeFirstResponder", || self.focus_changed(true));
            }
            accepted
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            // SAFETY: forwarding to the superclass is the responder contract.
            let accepted: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if accepted {
                guarded("DayButton resignFirstResponder", || self.focus_changed(false));
            }
            accepted
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let handled = guarded("DayButton keyDown:", || {
                self.shared()
                    .is_some_and(|shared| shared.handle_key(self.ivars().index, event))
            });
            if !handled {
                // SAFETY: forwarding an unhandled key to the superclass is
                // the responder-chain contract; Space presses the button.
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            guarded("DayButton drawRect:", || {
                if let Some(color) = self.ivars().highlight.borrow().as_ref() {
                    color.setFill();
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                        self.bounds(),
                        HIGHLIGHT_RADIUS,
                        HIGHLIGHT_RADIUS,
                    )
                    .fill();
                }
            });
            // SAFETY: the superclass draws the number over the highlight.
            unsafe { msg_send![super(self), drawRect: dirty_rect] }
        }
    }
);

/// The day the key-view loop stops at: the selected day shown, or the
/// first selectable day of the month.
fn shared_anchor(shared: &Shared) -> Option<usize> {
    let model = shared.model.borrow();
    model
        .selected
        .iter()
        .find_map(|day| model.index_of(*day))
        .or_else(|| {
            model
                .grid
                .iter()
                .position(|cell| cell.in_month && model.in_range(cell.parts))
        })
}

impl DayButton {
    fn new(mtm: MainThreadMarker, index: usize) -> Retained<Self> {
        let dot = Dot::new(mtm);
        let this = Self::alloc(mtm).set_ivars(DayIvars {
            index,
            shared: RefCell::new(RcWeak::new()),
            highlight: RefCell::new(None),
            dot,
        });
        // SAFETY: `initWithFrame:` is `NSButton`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setBordered(false);
        this.setTitle(&NSString::new());
        this.addSubview(&this.ivars().dot);
        this.setTranslatesAutoresizingMaskIntoConstraints(false);
        this.widthAnchor()
            .constraintEqualToConstant(DAY_WIDTH)
            .setActive(true);
        this.heightAnchor()
            .constraintEqualToConstant(DAY_HEIGHT)
            .setActive(true);
        this
    }

    fn shared(&self) -> Option<Rc<Shared>> {
        self.ivars().shared.borrow().upgrade()
    }

    fn focus_changed(&self, focused: bool) {
        if let Some(shared) = self.shared() {
            shared.focused.set(focused);
            shared.refresh_emphasis();
        }
    }

    /// Shows `look`: the number, the highlight and the dot under the
    /// number's centre.
    fn show(&self, look: &DayLook, rtl: bool) {
        self.setAttributedTitle(&look.title);
        self.ivars()
            .highlight
            .borrow_mut()
            .clone_from(&look.highlight);
        let view: &NSView = self;
        view.setNeedsDisplay(true);
        let dot = &self.ivars().dot;
        let Some(color) = &look.dot else {
            dot.setHidden(true);
            return;
        };
        dot.set_color(color);
        dot.setHidden(false);
        let bounds = NSRect::new(
            objc2_foundation::NSPoint::ZERO,
            objc2_foundation::NSSize::new(DAY_WIDTH, DAY_HEIGHT),
        );
        let title_rect = self
            .cell()
            .expect("an NSButton always has a cell")
            .titleRectForBounds(bounds);
        let text_width = look.title.size().width;
        let center_x = if rtl {
            title_rect.origin.x + text_width / 2.0
        } else {
            title_rect.origin.x + title_rect.size.width - text_width / 2.0
        };
        let center_y = if self.isFlipped() {
            DOT_CENTER
        } else {
            DAY_HEIGHT - DOT_CENTER
        };
        dot.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(
                center_x - DOT_DIAMETER / 2.0,
                center_y - DOT_DIAMETER / 2.0,
            ),
            objc2_foundation::NSSize::new(DOT_DIAMETER, DOT_DIAMETER),
        ));
    }
}

/// A [`Dot`]'s fill.
struct DotIvars {
    color: RefCell<Retained<NSColor>>,
}

impl fmt::Debug for DotIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DotIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `Dot::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiCalendarDot"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DotIvars]
    #[derive(Debug)]
    /// A decoration dot: a round layer filled with a colour resolved in
    /// the view's appearance.
    struct Dot;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for Dot {}

    impl Dot {
        // SAFETY: see the module safety note.
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        // `AppKit` makes the view's appearance current around this call,
        // so a dynamic colour resolves for it.
        // SAFETY: see the module safety note.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            guarded("Dot updateLayer", || {
                if let Some(layer) = self.layer() {
                    layer.setBackgroundColor(Some(&self.ivars().color.borrow().CGColor()));
                    layer.setCornerRadius(DOT_DIAMETER / 2.0);
                }
            });
        }
    }
);

impl Dot {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DotIvars {
            color: RefCell::new(NSColor::clearColor()),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        this.setHidden(true);
        this
    }

    fn set_color(&self, color: &NSColor) {
        *self.ivars().color.borrow_mut() = objc2::Message::retain(color);
        self.setNeedsDisplay(true);
    }
}

/// A month calendar composed of `AppKit` controls, laid out like the
/// graphical `NSDatePicker`, with decoration dots.
pub struct CalendarView {
    shared: Rc<Shared>,
}

impl fmt::Debug for CalendarView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarView").finish_non_exhaustive()
    }
}

impl CalendarView {
    /// A calendar selecting in `mode` over `range` showing `month`,
    /// formatted for the locale
    /// `locale` names, drawn in `colors`, with `labels` naming the
    /// navigation buttons.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        mode: CalendarSelectionMode,
        range: RangeInclusive<DateParts>,
        month: YearMonth,
        locale: &str,
        colors: CalendarColors,
        labels: &CalendarNavigationLabels,
    ) -> Self {
        let root = CalendarRoot::alloc(mtm).set_ivars(RootIvars::default());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let root: Retained<CalendarRoot> =
            unsafe { msg_send![super(root), initWithFrame: NSRect::ZERO] };
        root.setAccessibilityElement(true);
        // SAFETY: the role is an immutable `NSString` static `AppKit` exports.
        root.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));

        let rtl = is_right_to_left(&root);
        let title = NSTextField::labelWithString(&NSString::new(), mtm);
        title.setFont(Some(&system_font(mtm, TITLE_FONT)));
        title.setTextColor(Some(&colors.foreground));
        let navigation = [
            navigation_button(
                mtm,
                if rtl {
                    "arrowtriangle.right.fill"
                } else {
                    "arrowtriangle.left.fill"
                },
                &labels.previous,
                &colors.muted,
            ),
            navigation_button(mtm, "circle.fill", &labels.selection, &colors.muted),
            navigation_button(
                mtm,
                if rtl {
                    "arrowtriangle.left.fill"
                } else {
                    "arrowtriangle.right.fill"
                },
                &labels.next,
                &colors.muted,
            ),
        ];
        let weekdays: Vec<_> = (0..7)
            .map(|_| {
                let field = NSTextField::labelWithString(&NSString::new(), mtm);
                field.setAlignment(NSTextAlignment::Center);
                field
            })
            .collect();
        let days: Vec<_> = (0..GRID_DAYS)
            .map(|index| DayButton::new(mtm, index))
            .collect();
        let separator = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
        separator.setBoxType(NSBoxType::Custom);
        separator.setTitlePosition(NSTitlePosition::NoTitle);
        separator.setBorderWidth(0.0);
        separator.setFillColor(&colors.separator);

        let text = LocaleText::new(locale, &system_font(mtm, WEEKDAY_FONT));
        let model = Model {
            month,
            range,
            grid: month_grid(month, text.first_weekday),
            selected: BTreeSet::new(),
            decorated: BTreeSet::new(),
            text,
            colors,
            numbers: Vec::new(),
        };
        let shared = Rc::new(Shared {
            mode,
            root,
            title,
            navigation,
            weekdays,
            separator,
            days,
            model: RefCell::new(model),
            focused: Cell::new(false),
            emphasized: Cell::new(false),
            on_pick: RefCell::new(None),
            on_visible_month_change: RefCell::new(None),
            actions: RefCell::new(Vec::new()),
            window_observers: RefCell::new(Vec::new()),
        });
        *shared.root.ivars().shared.borrow_mut() = Rc::downgrade(&shared);
        for day in &shared.days {
            *day.ivars().shared.borrow_mut() = Rc::downgrade(&shared);
        }
        install(&shared);
        connect(&shared);
        shared.refresh_weekdays();
        shared.show_model_month();
        Self { shared }
    }

    /// The view to add to a container and frame at
    /// [`intrinsic_size`][Self::intrinsic_size].
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.shared.root
    }

    /// The calendar's fixed size — the graphical date picker's.
    #[must_use]
    pub const fn intrinsic_size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }

    /// Formats the calendar for the locale `identifier` names: its
    /// month title, digits, weekday symbols, first weekday and spoken
    /// dates.
    pub fn set_locale(&self, identifier: &str) {
        let font = system_font(self.shared.root.mtm(), WEEKDAY_FONT);
        self.shared.model.borrow_mut().text = LocaleText::new(identifier, &font);
        self.shared.refresh_weekdays();
        self.shared.show_model_month();
    }

    /// Replaces the selected days.
    pub fn set_selection(&self, days: &[DateParts]) {
        let selected: BTreeSet<DateParts> = days.iter().copied().collect();
        let changed: Vec<usize> = {
            let mut model = self.shared.model.borrow_mut();
            let previous = std::mem::replace(&mut model.selected, selected);
            previous
                .symmetric_difference(&model.selected)
                .filter_map(|day| model.index_of(*day))
                .collect()
        };
        self.shared.refresh_days(changed);
        post_accessibility_notification(
            &self.shared.root,
            AccessibilityNotification::SelectedChildrenChanged,
        );
    }

    /// Replaces the days carrying a decoration dot.
    pub fn set_decorations(&self, days: impl IntoIterator<Item = DateParts>) {
        let decorated: BTreeSet<DateParts> = days.into_iter().collect();
        let changed: Vec<usize> = {
            let mut model = self.shared.model.borrow_mut();
            let previous = std::mem::replace(&mut model.decorated, decorated);
            previous
                .symmetric_difference(&model.decorated)
                .filter_map(|day| model.index_of(*day))
                .collect()
        };
        self.shared.refresh_days(changed);
    }

    /// Shows `month` without reporting it back.
    pub fn set_visible_month(&self, month: YearMonth) {
        {
            let mut model = self.shared.model.borrow_mut();
            if model.month == month {
                return;
            }
            model.month = month;
        }
        self.shared.show_model_month();
    }

    /// Replaces one of the theme colours.
    pub fn set_color(&self, role: CalendarColorRole, color: &NSColor) {
        let color = objc2::Message::retain(color);
        {
            let mut model = self.shared.model.borrow_mut();
            let colors = &mut model.colors;
            match role {
                CalendarColorRole::Foreground => colors.foreground = color.clone(),
                CalendarColorRole::Muted => colors.muted = color.clone(),
                CalendarColorRole::Separator => colors.separator = color.clone(),
                CalendarColorRole::Accent => colors.accent = color.clone(),
                CalendarColorRole::Selection => colors.selection = color.clone(),
                CalendarColorRole::SelectionForeground => {
                    colors.selection_foreground = color.clone();
                }
            }
        }
        match role {
            CalendarColorRole::Foreground => {
                self.shared.title.setTextColor(Some(&color));
                self.shared.refresh_days(0..GRID_DAYS);
            }
            CalendarColorRole::Muted => {
                self.shared.refresh_weekdays();
                for button in &self.shared.navigation {
                    button.setContentTintColor(Some(&color));
                }
                self.shared.refresh_days(0..GRID_DAYS);
            }
            CalendarColorRole::Separator => self.shared.separator.setFillColor(&color),
            CalendarColorRole::Accent
            | CalendarColorRole::Selection
            | CalendarColorRole::SelectionForeground => {
                self.shared.refresh_days(0..GRID_DAYS);
            }
        }
    }

    /// Names one navigation button to assistive technology.
    pub fn set_navigation_label(&self, button: CalendarNavigation, label: &str) {
        self.shared
            .navigation_button(button)
            .setAccessibilityLabel(Some(&NSString::from_str(label)));
    }

    /// `handler` is called with the day the user picks.
    pub fn on_pick(&self, handler: impl Fn(DateParts) + 'static) {
        *self.shared.on_pick.borrow_mut() = Some(Rc::new(handler));
    }

    /// `handler` is called with the month the user navigates to.
    pub fn on_visible_month_change(&self, handler: impl Fn(YearMonth) + 'static) {
        *self.shared.on_visible_month_change.borrow_mut() = Some(Rc::new(handler));
    }

    /// Names the calendar to assistive technology; `None` leaves it
    /// unnamed.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        self.shared
            .root
            .setAccessibilityLabel(label.map(NSString::from_str).as_deref());
    }
}

/// A borderless button showing the named symbol, tinted `tint`, spoken as
/// `label`.
fn navigation_button(
    mtm: MainThreadMarker,
    symbol: &str,
    label: &str,
    tint: &NSColor,
) -> Retained<NSButton> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        None,
    )
    .and_then(|image| {
        image.imageWithSymbolConfiguration(
            &NSImageSymbolConfiguration::configurationWithPointSize_weight(
                NAV_SYMBOL_SIZE,
                font::weight::REGULAR,
            ),
        )
    })
    .unwrap_or_else(|| panic!("the SF Symbols catalog has {symbol}"));
    let button = NSButton::initWithFrame(NSButton::alloc(mtm), NSRect::ZERO);
    button.setBordered(false);
    button.setTitle(&NSString::new());
    button.setImage(Some(&image));
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    button.setContentTintColor(Some(tint));
    button.setAccessibilityLabel(Some(&NSString::from_str(label)));
    let (width, height) = NAV_SIZE;
    button.setTranslatesAutoresizingMaskIntoConstraints(false);
    button
        .widthAnchor()
        .constraintEqualToConstant(width)
        .setActive(true);
    button
        .heightAnchor()
        .constraintEqualToConstant(height)
        .setActive(true);
    button
}

/// Adds the controls to the root and places them.
fn install(shared: &Shared) {
    let mtm = shared.mtm();
    let root: &NSView = &shared.root;

    let title = &shared.title;
    title.setTranslatesAutoresizingMaskIntoConstraints(false);
    root.addSubview(title);
    title
        .leadingAnchor()
        .constraintEqualToAnchor_constant(&root.leadingAnchor(), TITLE_LEADING)
        .setActive(true);
    title
        .firstBaselineAnchor()
        .constraintEqualToAnchor_constant(&root.topAnchor(), TITLE_BASELINE)
        .setActive(true);

    let navigation_views: Vec<&NSView> = shared
        .navigation
        .iter()
        .map(|button| -> &NSView { button })
        .collect();
    let navigation = NSStackView::stackViewWithViews(&NSArray::from_slice(&navigation_views), mtm);
    navigation.setSpacing(0.0);
    navigation.setTranslatesAutoresizingMaskIntoConstraints(false);
    root.addSubview(&navigation);
    navigation
        .trailingAnchor()
        .constraintEqualToAnchor_constant(&root.trailingAnchor(), -NAV_TRAILING)
        .setActive(true);
    navigation
        .centerYAnchor()
        .constraintEqualToAnchor_constant(&root.topAnchor(), NAV_CENTER_Y)
        .setActive(true);

    let (separator_top, separator_leading, separator_width, separator_height) = SEPARATOR;
    let separator = &shared.separator;
    separator.setTranslatesAutoresizingMaskIntoConstraints(false);
    root.addSubview(separator);
    separator
        .leadingAnchor()
        .constraintEqualToAnchor_constant(&root.leadingAnchor(), separator_leading)
        .setActive(true);
    separator
        .topAnchor()
        .constraintEqualToAnchor_constant(&root.topAnchor(), separator_top)
        .setActive(true);
    separator
        .widthAnchor()
        .constraintEqualToConstant(separator_width)
        .setActive(true);
    separator
        .heightAnchor()
        .constraintEqualToConstant(separator_height)
        .setActive(true);

    let weekday_row: Vec<&NSView> = shared
        .weekdays
        .iter()
        .map(|field| -> &NSView { field })
        .collect();
    let mut rows = vec![NSArray::from_slice(&weekday_row)];
    for week in shared.days.chunks(7) {
        let views: Vec<&NSView> = week.iter().map(|day| -> &NSView { day }).collect();
        rows.push(NSArray::from_slice(&views));
    }
    let grid = NSGridView::gridViewWithViews(&NSArray::from_retained_slice(&rows), mtm);
    grid.setRowSpacing(0.0);
    grid.setColumnSpacing(0.0);
    for column in 0..7 {
        let column = grid.columnAtIndex(column);
        column.setWidth(COLUMN_WIDTH);
        column.setXPlacement(NSGridCellPlacement::Center);
    }
    // The weekday row is placed by its symbols' baseline, the week rows
    // hold their day buttons at the top.
    let weekday_row = grid.rowAtIndex(0);
    weekday_row.setHeight(FIRST_WEEK_TOP - GRID_TOP);
    weekday_row.setYPlacement(NSGridCellPlacement::None);
    for row in 1..=GRID_DAYS / 7 {
        let row = grid.rowAtIndex(isize::try_from(row).expect("seven grid rows"));
        row.setHeight(ROW_HEIGHT);
        row.setYPlacement(NSGridCellPlacement::Top);
    }
    grid.setTranslatesAutoresizingMaskIntoConstraints(false);
    root.addSubview(&grid);
    grid.leadingAnchor()
        .constraintEqualToAnchor_constant(&root.leadingAnchor(), GRID_LEADING)
        .setActive(true);
    grid.topAnchor()
        .constraintEqualToAnchor_constant(&root.topAnchor(), GRID_TOP)
        .setActive(true);
    for field in &shared.weekdays {
        field
            .firstBaselineAnchor()
            .constraintEqualToAnchor_constant(&root.topAnchor(), WEEKDAY_BASELINE)
            .setActive(true);
    }
}

/// Wires the buttons' actions.
fn connect(shared: &Rc<Shared>) {
    let mut actions = shared.actions.borrow_mut();
    for button in [
        CalendarNavigation::Previous,
        CalendarNavigation::Selection,
        CalendarNavigation::Next,
    ] {
        let weak = Rc::downgrade(shared);
        actions.push(ActionTarget::new(
            shared.navigation_button(button),
            move |_| {
                if let Some(shared) = weak.upgrade() {
                    shared.activate(button);
                }
            },
        ));
    }
    for (index, day) in shared.days.iter().enumerate() {
        let weak = Rc::downgrade(shared);
        actions.push(ActionTarget::new(day, move |_| {
            if let Some(shared) = weak.upgrade() {
                let day = shared.model.borrow().grid[index].parts;
                shared.pick(day);
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_starts_on_the_locale_first_weekday() {
        let november = YearMonth {
            year: 2026,
            month: 11,
        };
        // 1 November 2026 is a Sunday: a Sunday-first grid opens on it.
        let sunday_first = month_grid(november, 1);
        assert_eq!(sunday_first[0].parts, november.first_day());
        assert!(sunday_first[0].in_month);
        assert_eq!(
            sunday_first[41].parts,
            DateParts {
                year: 2026,
                month: 12,
                day: 12
            }
        );
        // A Monday-first grid leads with six October days.
        let monday_first = month_grid(november, 2);
        assert_eq!(
            monday_first[0].parts,
            DateParts {
                year: 2026,
                month: 10,
                day: 26
            }
        );
        assert!(!monday_first[5].in_month);
        assert_eq!(monday_first[6].parts, november.first_day());
        assert_eq!(monday_first.iter().filter(|day| day.in_month).count(), 30);
    }

    #[test]
    fn locale_text_follows_the_locale() {
        let font = NSFont::systemFontOfSize_weight(WEEKDAY_FONT.0, WEEKDAY_FONT.1);
        let text = LocaleText::new("en-US", &font);
        assert_eq!(text.weekdays[0], "Su");
        assert_eq!(text.weekdays[6], "Sa");
        assert_eq!(text.first_weekday, 1);
        // German weeks start on Monday, and so does an English locale
        // carrying a Monday first-weekday preference.
        assert_eq!(LocaleText::new("de-DE", &font).first_weekday, 2);
        assert_eq!(LocaleText::new("en-US-u-fw-mon", &font).first_weekday, 2);
        // Two-letter symbols that overflow a column give way to the narrow
        // ones, as in the graphical date picker.
        assert_eq!(LocaleText::new("zh-Hans-CN", &font).weekdays[0], "日");
        assert_eq!(LocaleText::new("ja-JP", &font).weekdays[0], "日");
        assert_eq!(LocaleText::new("ko-KR", &font).weekdays[0], "일");
        assert_eq!(LocaleText::new("ar", &font).weekdays[0], "ح");
        // Egyptian Arabic writes day numbers in Arabic-Indic digits.
        let arabic = LocaleText::new("ar-EG", &font);
        let twentieth = DateParts {
            year: 2026,
            month: 11,
            day: 20,
        };
        assert_eq!(
            arabic
                .day
                .stringFromDate(&arabic.date(twentieth))
                .to_string(),
            "٢٠"
        );
    }
}
