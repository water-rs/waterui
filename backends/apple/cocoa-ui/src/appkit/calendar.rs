//! The `AppKit` month calendar: an `NSView` that draws and behaves like the
//! graphical `NSDatePicker` (`NSDatePickerStyleClockAndCalendar` with only
//! the date elements) and adds what that control cannot carry — decoration
//! dots under days and multi-day selection.
//!
//! The view reproduces the graphical picker's layout: the abbreviated month
//! and year in bold, the ◀ ● ▶ month navigation, the locale's two-letter
//! weekday symbols over a separator, and a six-week grid of trailing-aligned
//! day numbers. Days outside the visible month or the selectable range draw
//! faint, today draws in the accent colour, and a selected day sits in a
//! rounded highlight — the accent while the calendar is the first responder
//! of the key window, unemphasized otherwise. Every metric is the graphical
//! picker's, measured from `NSDatePicker` renders on macOS 27, and so is the
//! navigation: ◀ and ▶ move freely, past the selectable range, without
//! dimming, and ● brings back the month of the selection.
//!
//! Selection is owned by the caller: a click, an arrow key (single
//! selection) or Space and Return (multiple selection) report the day
//! through [`CalendarView::on_pick`], and the caller pushes the resulting
//! selection back through [`CalendarView::set_selection`]. Month navigation
//! is local and reported through [`CalendarView::on_visible_month_change`].
//!
//! Assistive technology sees a group with the title, the three navigation
//! buttons and one button per day, each labelled with the full date and
//! carrying its selected and enabled state.
//!
//! # Safety
//!
//! The `unsafe` here defines `NSView` and `NSAccessibilityElement`
//! subclasses, reads framework-exported constants, and calls `objc2`
//! bindings marked unsafe because `AppKit` view and drawing APIs are
//! main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee. Every override `AppKit`
//! calls runs inside the panic guard.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::ops::RangeInclusive;
use std::rc::Rc;

use keyboard_types::{Key, NamedKey};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityButtonRole, NSAccessibilityElement, NSAccessibilityFrameInView,
    NSAccessibilityGroupRole, NSAccessibilityStaticTextRole, NSColor, NSEvent, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSGraphicsContext, NSResponder,
    NSStringDrawing, NSUserInterfaceLayoutDirection, NSView, NSWindowDidBecomeKeyNotification,
    NSWindowDidResignKeyNotification,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGContext, CGPath};
use objc2_foundation::{
    NSArray, NSAttributedStringKey, NSDateFormatter, NSDateFormatterStyle, NSDictionary, NSLocale,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::callback::guarded;
use crate::date::{self, CalendarSelectionMode, DateParts, YearMonth};
use crate::font;
use crate::geometry::Size;
use crate::notification::{self, NotificationName, NotificationObserver};

/// The control's fixed size — the graphical `NSDatePicker`'s
/// `intrinsicContentSize` with only the date elements.
const WIDTH: f64 = 138.5;
/// See [`WIDTH`].
const HEIGHT: f64 = 148.0;
/// The leading edge of the first grid column.
const GRID_X: f64 = 4.5;
/// The pitch between grid columns.
const COLUMN_WIDTH: f64 = 18.5;
/// The top of the first week row.
const GRID_Y: f64 = 39.0;
/// The pitch between week rows.
const ROW_HEIGHT: f64 = 18.0;
/// The selection highlight's width.
const HIGHLIGHT_WIDTH: f64 = 18.0;
/// The selection highlight's height.
const HIGHLIGHT_HEIGHT: f64 = 16.0;
/// The selection highlight's corner radius.
const HIGHLIGHT_RADIUS: f64 = 3.0;
/// The gap between a day number's trailing edge and its column's.
const TEXT_TRAILING_INSET: f64 = 2.5;
/// A day number's baseline below its row's top.
const DAY_BASELINE: f64 = 12.25;
/// The month title's leading inset.
const TITLE_X: f64 = 3.0;
/// The month title's baseline.
const TITLE_BASELINE: f64 = 15.25;
/// The weekday symbols' baseline.
const WEEKDAY_BASELINE: f64 = 33.25;
/// The separator under the weekday symbols: top, leading edge, width.
const SEPARATOR: (f64, f64, f64) = (37.0, 3.0, 132.5);
/// The navigation glyphs: leading edge and width of ◀, ●, ▶ in that order,
/// all `NAV_TOP`-aligned and `NAV_HEIGHT` tall.
const NAV_GLYPHS: [(f64, f64); 3] = [(97.5, 6.0), (111.5, 7.0), (126.5, 6.0)];
/// See [`NAV_GLYPHS`].
const NAV_TOP: f64 = 7.0;
/// See [`NAV_GLYPHS`].
const NAV_HEIGHT: f64 = 7.0;
/// How far a navigation glyph's click target extends past its ink.
const NAV_HIT_OUTSET: f64 = 4.0;
/// The decoration dot's diameter.
const DOT_DIAMETER: f64 = 3.0;
/// The decoration dot's centre below its row's top.
const DOT_CENTER: f64 = 14.5;
/// The keyboard cursor ring's stroke width.
const CURSOR_RING_WIDTH: f64 = 1.5;
/// The title's point size and weight.
const TITLE_FONT: (f64, f64) = (13.0, font::weight::BOLD);
/// The weekday symbols' point size and weight.
const WEEKDAY_FONT: (f64, f64) = (11.0, font::weight::MEDIUM);
/// The day numbers' point size and weight.
const DAY_FONT: (f64, f64) = (11.0, font::weight::REGULAR);
/// Opacity of `muted` for text outside the visible month or the range —
/// the platform's tertiary label against its secondary one.
const FAINT_OPACITY: f64 = 0.5;
/// Opacity of `muted` for text both outside the visible month and the
/// range — the platform's quaternary label against its secondary one.
const FAINTER_OPACITY: f64 = 0.2;
/// Opacity of `muted` for the selection highlight while the calendar is
/// not focused — the unemphasized selected-content background.
const UNEMPHASIZED_OPACITY: f64 = 0.3;
/// The days the grid shows: six weeks.
const GRID_DAYS: usize = 42;
/// A Sunday — the reference day the weekday symbols are formatted from.
const REFERENCE_SUNDAY: DateParts = DateParts {
    year: 2023,
    month: 1,
    day: 1,
};

type TextAttributes = NSDictionary<NSAttributedStringKey, AnyObject>;

/// The colours the calendar draws with. The faint text tiers and the
/// unemphasized highlight derive from `muted`.
#[derive(Debug, Clone)]
pub struct CalendarColors {
    /// Day numbers and the month title.
    pub foreground: Retained<NSColor>,
    /// Weekday symbols, navigation glyphs and decoration dots.
    pub muted: Retained<NSColor>,
    /// The line under the weekday symbols.
    pub separator: Retained<NSColor>,
    /// Today's number and the keyboard cursor ring.
    pub accent: Retained<NSColor>,
    /// The focused selection highlight.
    pub selection: Retained<NSColor>,
    /// Text and dots on the focused selection highlight.
    pub selection_foreground: Retained<NSColor>,
}

impl CalendarColors {
    /// The platform's own colours — the values a caller without a theme
    /// passes.
    #[must_use]
    pub fn system() -> Self {
        Self {
            foreground: NSColor::labelColor(),
            muted: NSColor::secondaryLabelColor(),
            separator: NSColor::separatorColor(),
            accent: NSColor::controlAccentColor(),
            selection: NSColor::selectedContentBackgroundColor(),
            selection_foreground: NSColor::alternateSelectedControlTextColor(),
        }
    }
}

/// What a click, a key or an assistive press acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The month title — static text.
    Title,
    /// ◀: the month before.
    Previous,
    /// ●: the month of the selection.
    Selection,
    /// ▶: the month after.
    Next,
    /// A grid cell, `0..GRID_DAYS`.
    Day(usize),
}

impl Target {
    /// Every target in accessibility order.
    fn all() -> impl Iterator<Item = Self> {
        [Self::Title, Self::Previous, Self::Selection, Self::Next]
            .into_iter()
            .chain((0..GRID_DAYS).map(Self::Day))
    }
}

/// One grid cell.
#[derive(Debug, Clone, Copy)]
struct GridDay {
    parts: DateParts,
    in_month: bool,
}

/// The locale-derived text the calendar draws and speaks.
struct LocaleText {
    /// The abbreviated month and year, e.g. "Nov 2026".
    title: Retained<NSDateFormatter>,
    /// The full date a day button speaks.
    spoken: Retained<NSDateFormatter>,
    /// The two-letter weekday symbols, Sunday first.
    weekdays: [String; 7],
    /// The locale's first weekday, `1` (Sunday) through `7`.
    first_weekday: i32,
}

impl LocaleText {
    fn new(locale: &NSLocale) -> Self {
        let calendar = date::gregorian();
        calendar.setLocale(Some(locale));
        let formatter = |configure: &dyn Fn(&NSDateFormatter)| {
            let formatter = NSDateFormatter::new();
            formatter.setLocale(Some(locale));
            formatter.setCalendar(Some(&calendar));
            configure(&formatter);
            formatter
        };
        let title =
            formatter(&|f| f.setLocalizedDateFormatFromTemplate(&NSString::from_str("MMMyyyy")));
        let spoken = formatter(&|f| {
            f.setDateStyle(NSDateFormatterStyle::FullStyle);
            f.setTimeStyle(NSDateFormatterStyle::NoStyle);
        });
        let symbol = formatter(&|f| f.setDateFormat(Some(&NSString::from_str("cccccc"))));
        let weekdays = std::array::from_fn(|offset| {
            let day = date::add_days(
                REFERENCE_SUNDAY,
                i32::try_from(offset).expect("seven weekdays"),
            );
            symbol.stringFromDate(&ns_date(day)).to_string()
        });
        let first_weekday =
            i32::try_from(calendar.firstWeekday()).expect("firstWeekday is 1 through 7");
        Self {
            title,
            spoken,
            weekdays,
            first_weekday,
        }
    }
}

/// The `NSDate` at midnight of `day`.
///
/// # Panics
///
/// When `day` is not a real Gregorian date.
fn ns_date(day: DateParts) -> Retained<objc2_foundation::NSDate> {
    date::ns_date_from_date_parts(day).expect("calendar days are valid Gregorian dates")
}

/// The calendar's model and appearance.
struct State {
    mode: CalendarSelectionMode,
    visible: YearMonth,
    grid: [GridDay; GRID_DAYS],
    selected: BTreeSet<DateParts>,
    decorated: BTreeSet<DateParts>,
    range: RangeInclusive<DateParts>,
    /// The keyboard cursor: the day arrow keys move from.
    cursor: DateParts,
    colors: CalendarColors,
    text: LocaleText,
    navigation_labels: CalendarNavigationLabels,
}

impl State {
    /// Lays out `visible` as six weeks starting on the locale's first
    /// weekday.
    fn rebuild_grid(&mut self) {
        let month = self.visible;
        let lead = (month.first_weekday() - self.text.first_weekday).rem_euclid(7);
        let previous = month.previous();
        let previous_count = previous.day_count();
        let count = month.day_count();
        let next = month.next();
        self.grid = std::array::from_fn(|index| {
            let offset = i32::try_from(index).expect("42 grid days") - lead;
            if offset < 0 {
                GridDay {
                    parts: DateParts {
                        year: previous.year,
                        month: previous.month,
                        day: previous_count + offset + 1,
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
        });
    }

    fn in_range(&self, day: DateParts) -> bool {
        self.range.contains(&day)
    }

    /// The month ● brings back: the earliest selected day's, or today's
    /// when nothing is selected.
    fn selection_month(&self) -> YearMonth {
        YearMonth::of(self.selected.first().copied().unwrap_or_else(date::today))
    }
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

/// The user's callbacks. Cloned out before a call so a callback that
/// pushes state back into the calendar finds nothing borrowed.
#[derive(Default)]
struct Handlers {
    pick: Option<Rc<dyn Fn(DateParts)>>,
    visible_month: Option<Rc<dyn Fn(YearMonth)>>,
}

/// The calendar's instance state.
pub struct CalendarViewIvars {
    state: RefCell<State>,
    handlers: RefCell<Handlers>,
    /// One element per [`Target`], in [`Target::all`] order.
    elements: RefCell<Vec<Retained<CalendarElement>>>,
    /// The key-window observers of the window the view is in.
    window_observers: RefCell<Vec<NotificationObserver>>,
}

impl fmt::Debug for CalendarViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("CalendarViewIvars")
            .field("mode", &state.mode)
            .field("visible", &state.visible)
            .field("selected", &state.selected)
            .field("decorated", &state.decorated)
            .field("range", &state.range)
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `CalendarView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiCalendarView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = CalendarViewIvars]
    #[derive(Debug)]
    /// A month calendar drawn and operated like the graphical
    /// `NSDatePicker`, with decorations and multi-day selection.
    ///
    /// Its coordinates are flipped: the origin is the top-left corner and
    /// `y` grows downward, matching the rest of the kit.
    pub struct CalendarView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for CalendarView {}

    // SAFETY: the view answers its accessibility from state it owns.
    unsafe impl NSAccessibility for CalendarView {}

    impl CalendarView {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size_override(&self) -> NSSize {
            NSSize::new(WIDTH, HEIGHT)
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder_override(&self) -> bool {
            self.setNeedsDisplay(true);
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder_override(&self) -> bool {
            self.setNeedsDisplay(true);
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window_override(&self) {
            guarded("CalendarView viewDidMoveToWindow", || self.observe_window());
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance_override(&self) {
            self.setNeedsDisplay(true);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDown:))]
        fn mouse_down_override(&self, event: &NSEvent) {
            guarded("CalendarView mouseDown:", || {
                if let Some(window) = self.window() {
                    let responder: &NSResponder = self;
                    window.makeFirstResponder(Some(responder));
                }
                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                if let Some(target) = self.target_at(point) {
                    self.activate(target);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(keyDown:))]
        fn key_down_override(&self, event: &NSEvent) {
            let handled = guarded("CalendarView keyDown:", || self.handle_key(event));
            if !handled {
                // SAFETY: forwarding an unhandled key to the superclass is
                // the documented responder-chain contract.
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(drawRect:))]
        fn draw_rect_override(&self, _dirty_rect: NSRect) {
            guarded("CalendarView drawRect:", || self.draw());
        }
    }
);

impl CalendarView {
    /// A calendar in `mode` showing today's month in the user's locale,
    /// drawn in the platform's colours, with nothing selected and its
    /// navigation buttons named `navigation_labels`.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        mode: CalendarSelectionMode,
        navigation_labels: CalendarNavigationLabels,
    ) -> Retained<Self> {
        let today = date::today();
        let text = LocaleText::new(&NSLocale::autoupdatingCurrentLocale());
        let mut state = State {
            mode,
            visible: YearMonth::of(today),
            grid: [GridDay {
                parts: today,
                in_month: true,
            }; GRID_DAYS],
            selected: BTreeSet::new(),
            decorated: BTreeSet::new(),
            range: DateParts {
                year: i32::from(i16::MIN),
                month: 1,
                day: 1,
            }..=DateParts {
                year: i32::from(i16::MAX),
                month: 12,
                day: 31,
            },
            cursor: today,
            colors: CalendarColors::system(),
            text,
            navigation_labels,
        };
        state.rebuild_grid();
        let this = Self::alloc(mtm).set_ivars(CalendarViewIvars {
            state: RefCell::new(state),
            handlers: RefCell::new(Handlers::default()),
            elements: RefCell::new(Vec::new()),
            window_observers: RefCell::new(Vec::new()),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let view: Retained<Self> = unsafe {
            msg_send![super(this), initWithFrame: CGRect::new(CGPoint::ZERO, CGSize::new(WIDTH, HEIGHT))]
        };
        view.install_accessibility();
        view
    }

    /// The control's size — fixed, as the graphical picker's.
    #[must_use]
    pub const fn intrinsic_size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }

    /// Formats the title, weekday symbols, spoken dates and first weekday
    /// for the locale `identifier` names.
    pub fn set_locale(&self, identifier: &str) {
        let locale = NSLocale::localeWithLocaleIdentifier(&NSString::from_str(identifier));
        {
            let mut state = self.ivars().state.borrow_mut();
            state.text = LocaleText::new(&locale);
            state.rebuild_grid();
        }
        self.refresh();
    }

    /// The colours the calendar draws with.
    pub fn set_colors(&self, colors: CalendarColors) {
        self.ivars().state.borrow_mut().colors = colors;
        self.setNeedsDisplay(true);
    }

    /// The spoken names of ◀, ● and ▶.
    pub fn set_navigation_labels(&self, labels: CalendarNavigationLabels) {
        self.ivars().state.borrow_mut().navigation_labels = labels;
        self.update_accessibility();
    }

    /// The inclusive range of selectable days. Days outside it draw faint
    /// and ignore clicks and keys; navigation still shows any month, as the
    /// graphical picker's does.
    pub fn set_range(&self, range: RangeInclusive<DateParts>) {
        self.ivars().state.borrow_mut().range = range;
        self.refresh();
    }

    /// The selected days. In single mode the keyboard cursor follows the
    /// first of them.
    pub fn set_selection(&self, selected: impl IntoIterator<Item = DateParts>) {
        {
            let mut state = self.ivars().state.borrow_mut();
            state.selected = selected.into_iter().collect();
            if state.mode == CalendarSelectionMode::Single
                && let Some(first) = state.selected.first().copied()
            {
                state.cursor = first;
            }
        }
        self.refresh();
    }

    /// The days that carry a decoration dot.
    pub fn set_decorations(&self, decorated: impl IntoIterator<Item = DateParts>) {
        self.ivars().state.borrow_mut().decorated = decorated.into_iter().collect();
        self.setNeedsDisplay(true);
    }

    /// The month the calendar shows.
    #[must_use]
    pub fn visible_month(&self) -> YearMonth {
        self.ivars().state.borrow().visible
    }

    /// Shows `month`; reports nothing back.
    pub fn set_visible_month(&self, month: YearMonth) {
        {
            let mut state = self.ivars().state.borrow_mut();
            if state.visible == month {
                return;
            }
            state.visible = month;
            if YearMonth::of(state.cursor) != month {
                state.cursor = month.first_day();
            }
            state.rebuild_grid();
        }
        self.refresh();
    }

    /// `handler` receives the day the user picked: in single mode the day
    /// to select, in multiple mode the day to toggle. The calendar changes
    /// nothing itself; the caller answers with
    /// [`set_selection`](Self::set_selection).
    pub fn on_pick(&self, handler: impl Fn(DateParts) + 'static) {
        self.ivars().handlers.borrow_mut().pick = Some(Rc::new(handler));
    }

    /// `handler` receives the month the user navigated to.
    pub fn on_visible_month_change(&self, handler: impl Fn(YearMonth) + 'static) {
        self.ivars().handlers.borrow_mut().visible_month = Some(Rc::new(handler));
    }

    /// Names the calendar group to a screen reader.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
    }

    /// Whether the selection highlight draws emphasized: the calendar is the
    /// first responder of the key window.
    fn is_emphasized(&self) -> bool {
        self.window().is_some_and(|window| window.isKeyWindow()) && self.is_first_responder()
    }

    /// Whether the calendar is its window's first responder.
    fn is_first_responder(&self) -> bool {
        self.window().is_some_and(|window| {
            window.firstResponder().is_some_and(|responder| {
                let own: &NSResponder = self;
                std::ptr::eq(&raw const *responder, own)
            })
        })
    }

    fn is_rtl(&self) -> bool {
        self.userInterfaceLayoutDirection() == NSUserInterfaceLayoutDirection::RightToLeft
    }

    /// Mirrors a rect drawn at `x` with `width` for right-to-left layout.
    fn mirrored(&self, x: f64, width: f64) -> f64 {
        if self.is_rtl() { WIDTH - x - width } else { x }
    }

    /// The column's leading edge, mirrored for right-to-left layout.
    fn column_x(&self, column: usize) -> f64 {
        let column = f64::from(u8::try_from(column).expect("seven columns"));
        self.mirrored(column.mul_add(COLUMN_WIDTH, GRID_X), COLUMN_WIDTH)
    }

    /// The top of week row `row`.
    fn row_y(row: usize) -> f64 {
        f64::from(u8::try_from(row).expect("six rows")).mul_add(ROW_HEIGHT, GRID_Y)
    }

    /// The highlight rect of grid cell `index`.
    fn cell_rect(&self, index: usize) -> CGRect {
        CGRect::new(
            CGPoint::new(self.column_x(index % 7), Self::row_y(index / 7)),
            CGSize::new(HIGHLIGHT_WIDTH, HIGHLIGHT_HEIGHT),
        )
    }

    /// The ink rect of navigation glyph `slot` (0 ◀, 1 ●, 2 ▶).
    fn nav_rect(&self, slot: usize) -> CGRect {
        let (x, width) = NAV_GLYPHS[slot];
        CGRect::new(
            CGPoint::new(self.mirrored(x, width), NAV_TOP),
            CGSize::new(width, NAV_HEIGHT),
        )
    }

    /// The rect `target` occupies, for hit testing and accessibility.
    fn target_rect(&self, target: Target) -> CGRect {
        let outset = |rect: CGRect| {
            CGRect::new(
                CGPoint::new(
                    rect.origin.x - NAV_HIT_OUTSET,
                    rect.origin.y - NAV_HIT_OUTSET,
                ),
                CGSize::new(
                    2.0f64.mul_add(NAV_HIT_OUTSET, rect.size.width),
                    2.0f64.mul_add(NAV_HIT_OUTSET, rect.size.height),
                ),
            )
        };
        match target {
            Target::Title => {
                let state = self.ivars().state.borrow();
                let title = title_string(&state);
                let attributes = attributes(&state.colors.foreground, &title_font(self.mtm()));
                // SAFETY: `sizeWithAttributes:` is `NSStringDrawing`'s
                // documented measure.
                let size = unsafe { title.sizeWithAttributes(Some(&attributes)) };
                let font = title_font(self.mtm());
                let top = TITLE_BASELINE - font.ascender();
                CGRect::new(CGPoint::new(self.mirrored(TITLE_X, size.width), top), size)
            }
            Target::Previous => outset(self.nav_rect(0)),
            Target::Selection => outset(self.nav_rect(1)),
            Target::Next => outset(self.nav_rect(2)),
            Target::Day(index) => self.cell_rect(index),
        }
    }

    /// The interactive target under `point`.
    fn target_at(&self, point: NSPoint) -> Option<Target> {
        let contains = |rect: CGRect| {
            point.x >= rect.origin.x
                && point.x < rect.origin.x + rect.size.width
                && point.y >= rect.origin.y
                && point.y < rect.origin.y + rect.size.height
        };
        [Target::Previous, Target::Selection, Target::Next]
            .into_iter()
            .find(|target| contains(self.target_rect(*target)))
            .or_else(|| {
                // Grid cells tile their full pitch for hits, not just the
                // highlight.
                let column = (0..7).find(|column| {
                    let x = self.column_x(*column);
                    point.x >= x && point.x < x + COLUMN_WIDTH
                })?;
                let row = (0..6).find(|row| {
                    let y = Self::row_y(*row);
                    point.y >= y && point.y < y + ROW_HEIGHT
                })?;
                Some(Target::Day(row * 7 + column))
            })
    }

    /// Shows `month` and reports it.
    fn navigate(&self, month: YearMonth) {
        {
            let mut state = self.ivars().state.borrow_mut();
            if state.visible == month {
                return;
            }
            state.visible = month;
            state.rebuild_grid();
        }
        self.refresh();
        let handler = self.ivars().handlers.borrow().visible_month.clone();
        if let Some(handler) = handler {
            handler(month);
        }
    }

    /// Reports `day` as picked, showing its month first.
    fn pick(&self, day: DateParts) {
        if !self.ivars().state.borrow().in_range(day) {
            return;
        }
        self.navigate(YearMonth::of(day));
        self.ivars().state.borrow_mut().cursor = day;
        self.setNeedsDisplay(true);
        let handler = self.ivars().handlers.borrow().pick.clone();
        if let Some(handler) = handler {
            handler(day);
        }
    }

    /// Runs what a click or an assistive press on `target` does.
    fn activate(&self, target: Target) {
        match target {
            Target::Title => {}
            Target::Previous => {
                let month = self.ivars().state.borrow().visible.previous();
                self.navigate(month);
            }
            Target::Next => {
                let month = self.ivars().state.borrow().visible.next();
                self.navigate(month);
            }
            Target::Selection => {
                let month = self.ivars().state.borrow().selection_month();
                self.navigate(month);
            }
            Target::Day(index) => {
                let day = self.ivars().state.borrow().grid[index].parts;
                self.pick(day);
            }
        }
    }

    /// Arrow keys move the cursor (and, in single mode, the selection);
    /// Page Up and Page Down change month; Space and Return toggle the
    /// cursor day in multiple mode.
    fn handle_key(&self, event: &NSEvent) -> bool {
        let key = crate::keys::key_event(event).key;
        let rtl = self.is_rtl();
        let days = match key {
            Key::Named(NamedKey::ArrowLeft) => Some(if rtl { 1 } else { -1 }),
            Key::Named(NamedKey::ArrowRight) => Some(if rtl { -1 } else { 1 }),
            Key::Named(NamedKey::ArrowUp) => Some(-7),
            Key::Named(NamedKey::ArrowDown) => Some(7),
            _ => None,
        };
        let (mode, cursor) = {
            let state = self.ivars().state.borrow();
            (state.mode, state.cursor)
        };
        if let Some(days) = days {
            let day = date::add_days(cursor, days);
            if !self.ivars().state.borrow().in_range(day) {
                return true;
            }
            match mode {
                CalendarSelectionMode::Single => self.pick(day),
                CalendarSelectionMode::Multiple => {
                    self.navigate(YearMonth::of(day));
                    self.ivars().state.borrow_mut().cursor = day;
                    self.setNeedsDisplay(true);
                }
            }
            return true;
        }
        match key {
            Key::Named(NamedKey::PageUp) => {
                self.activate(Target::Previous);
                true
            }
            Key::Named(NamedKey::PageDown) => {
                self.activate(Target::Next);
                true
            }
            Key::Named(NamedKey::Enter) if mode == CalendarSelectionMode::Multiple => {
                self.pick(cursor);
                true
            }
            Key::Character(ref text) if text == " " && mode == CalendarSelectionMode::Multiple => {
                self.pick(cursor);
                true
            }
            _ => false,
        }
    }

    /// Redraws on key-window changes of the window the view is now in.
    fn observe_window(&self) {
        let mut observers = self.ivars().window_observers.borrow_mut();
        observers.clear();
        let Some(window) = self.window() else {
            return;
        };
        let mtm = self.mtm();
        // SAFETY: both names are immutable `NSString` statics `AppKit`
        // exports.
        let names = unsafe {
            [
                NSWindowDidBecomeKeyNotification,
                NSWindowDidResignKeyNotification,
            ]
        };
        for name in names {
            let weak = Weak::new(self);
            observers.push(notification::observe_object(
                mtm,
                &NotificationName::framework(name),
                AsRef::<AnyObject>::as_ref(&*window),
                move || {
                    if let Some(view) = weak.load() {
                        view.setNeedsDisplay(true);
                    }
                },
            ));
        }
    }

    /// Redraws and brings every accessibility element up to date.
    fn refresh(&self) {
        self.setNeedsDisplay(true);
        self.update_accessibility();
    }

    /// Creates one element per target and hangs them under the view.
    fn install_accessibility(&self) {
        // SAFETY: the role constant is an immutable `NSString` `AppKit`
        // exports.
        self.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        self.setAccessibilityElement(true);
        let elements: Vec<Retained<CalendarElement>> = Target::all()
            .map(|target| CalendarElement::new(self, target))
            .collect();
        let children: Vec<Retained<AnyObject>> = elements
            .iter()
            .map(|element| Retained::into_super(Retained::into_super(element.clone())).into())
            .collect();
        // SAFETY: every entry is an `NSAccessibilityElement`.
        unsafe { self.setAccessibilityChildren(Some(&NSArray::from_retained_slice(&children))) };
        *self.ivars().elements.borrow_mut() = elements;
        self.update_accessibility();
    }

    /// Re-labels every element from the current state.
    fn update_accessibility(&self) {
        let elements = self.ivars().elements.borrow();
        for element in elements.iter() {
            let target = element.ivars().target;
            let base: &NSAccessibilityElement = element;
            let accessibility: &ProtocolObject<dyn NSAccessibility> =
                ProtocolObject::from_ref(base);
            let state = self.ivars().state.borrow();
            let (label, enabled, selected) = match target {
                Target::Title => (title_string(&state).to_string(), true, false),
                Target::Previous => (state.navigation_labels.previous.clone(), true, false),
                Target::Selection => (state.navigation_labels.selection.clone(), true, false),
                Target::Next => (state.navigation_labels.next.clone(), true, false),
                Target::Day(index) => {
                    let day = state.grid[index].parts;
                    (
                        state.text.spoken.stringFromDate(&ns_date(day)).to_string(),
                        state.in_range(day),
                        state.selected.contains(&day),
                    )
                }
            };
            accessibility.setAccessibilityLabel(Some(&NSString::from_str(&label)));
            accessibility.setAccessibilityEnabled(enabled);
            accessibility.setAccessibilitySelected(selected);
        }
    }

    /// Draws the whole calendar.
    fn draw(&self) {
        let Some(context) = NSGraphicsContext::currentContext() else {
            return;
        };
        let context = context.CGContext();
        let state = self.ivars().state.borrow();
        let muted = &state.colors.muted;
        let tiers = FaintTiers {
            faint: muted.colorWithAlphaComponent(muted.alphaComponent() * FAINT_OPACITY),
            fainter: muted.colorWithAlphaComponent(muted.alphaComponent() * FAINTER_OPACITY),
            unemphasized: muted
                .colorWithAlphaComponent(muted.alphaComponent() * UNEMPHASIZED_OPACITY),
        };
        self.draw_header(&context, &state);
        self.draw_days(&context, &state, &tiers);
    }

    /// Draws the title, the navigation glyphs, the weekday symbols and the
    /// separator.
    fn draw_header(&self, context: &CGContext, state: &State) {
        let mtm = self.mtm();
        let rtl = self.is_rtl();
        let colors = &state.colors;

        // Title.
        let title_font = title_font(mtm);
        let title = title_string(state);
        let title_attributes = attributes(&colors.foreground, &title_font);
        // SAFETY: `sizeWithAttributes:` measures under the attributes the
        // draw below uses.
        let title_width = unsafe { title.sizeWithAttributes(Some(&title_attributes)) }.width;
        draw_text(
            &title,
            &title_attributes,
            &title_font,
            self.mirrored(TITLE_X, title_width),
            TITLE_BASELINE,
        );

        // Navigation: ◀ ● ▶, never dimmed — the graphical picker navigates
        // past its range.
        fill_triangle(context, self.nav_rect(0), true, &colors.muted);
        CGContext::set_fill_color_with_color(Some(context), Some(&colors.muted.CGColor()));
        CGContext::fill_ellipse_in_rect(Some(context), self.nav_rect(1));
        fill_triangle(context, self.nav_rect(2), false, &colors.muted);

        // Weekday symbols and the separator.
        let weekday_font = system_font(mtm, WEEKDAY_FONT);
        let weekday_attributes = attributes(&colors.muted, &weekday_font);
        for column in 0..7 {
            let weekday = usize::try_from(
                (state.text.first_weekday - 1 + i32::try_from(column).expect("seven columns"))
                    .rem_euclid(7),
            )
            .expect("weekday index is 0 through 6");
            let symbol = NSString::from_str(&state.text.weekdays[weekday]);
            draw_trailing(
                &symbol,
                &weekday_attributes,
                &weekday_font,
                self.column_x(column),
                WEEKDAY_BASELINE,
                rtl,
            );
        }
        let (separator_y, separator_x, separator_width) = SEPARATOR;
        CGContext::set_fill_color_with_color(Some(context), Some(&colors.separator.CGColor()));
        CGContext::fill_rect(
            Some(context),
            CGRect::new(
                CGPoint::new(self.mirrored(separator_x, separator_width), separator_y),
                CGSize::new(separator_width, 1.0),
            ),
        );
    }

    /// Draws the six weeks of days with their highlights and decorations.
    fn draw_days(&self, context: &CGContext, state: &State, tiers: &FaintTiers) {
        let mtm = self.mtm();
        let emphasized = self.is_emphasized();
        let rtl = self.is_rtl();
        let colors = &state.colors;
        let FaintTiers {
            faint,
            fainter,
            unemphasized,
        } = tiers;
        let today = date::today();
        let day_font = system_font(mtm, DAY_FONT);
        let focused = self.is_first_responder();
        for (index, cell) in state.grid.iter().enumerate() {
            let rect = self.cell_rect(index);
            let selected = state.selected.contains(&cell.parts);
            let in_range = state.in_range(cell.parts);
            if selected {
                let fill = if emphasized {
                    &colors.selection
                } else {
                    unemphasized
                };
                fill_rounded_rect(context, rect, fill);
            }
            if focused
                && state.mode == CalendarSelectionMode::Multiple
                && cell.parts == state.cursor
            {
                stroke_rounded_rect(context, rect, &colors.accent);
            }
            let on_highlight = selected && emphasized;
            let color = if on_highlight {
                &colors.selection_foreground
            } else if !cell.in_month && !in_range {
                fainter
            } else if !cell.in_month || !in_range {
                faint
            } else if cell.parts == today {
                &colors.accent
            } else {
                &colors.foreground
            };
            let number = NSString::from_str(&cell.parts.day.to_string());
            let number_attributes = attributes(color, &day_font);
            let text_rect = draw_trailing(
                &number,
                &number_attributes,
                &day_font,
                self.column_x(index % 7),
                rect.origin.y + DAY_BASELINE,
                rtl,
            );
            if state.decorated.contains(&cell.parts) {
                let dot = if on_highlight {
                    &colors.selection_foreground
                } else if cell.in_month && in_range {
                    &colors.muted
                } else {
                    faint
                };
                let center_x = text_rect.origin.x + text_rect.size.width / 2.0;
                CGContext::set_fill_color_with_color(Some(context), Some(&dot.CGColor()));
                CGContext::fill_ellipse_in_rect(
                    Some(context),
                    CGRect::new(
                        CGPoint::new(
                            center_x - DOT_DIAMETER / 2.0,
                            rect.origin.y + DOT_CENTER - DOT_DIAMETER / 2.0,
                        ),
                        CGSize::new(DOT_DIAMETER, DOT_DIAMETER),
                    ),
                );
            }
        }
    }
}

/// The colours derived from `muted`, resolved once per draw.
struct FaintTiers {
    /// Text outside the visible month or the range.
    faint: Retained<NSColor>,
    /// Text outside both.
    fainter: Retained<NSColor>,
    /// The selection highlight while the calendar is not focused.
    unemphasized: Retained<NSColor>,
}

/// The visible month's title in the locale's abbreviated format.
fn title_string(state: &State) -> Retained<NSString> {
    state
        .text
        .title
        .stringFromDate(&ns_date(state.visible.first_day()))
}

fn system_font(mtm: MainThreadMarker, (size, weight): (f64, f64)) -> Retained<NSFont> {
    font::system(mtm, size, weight)
}

fn title_font(mtm: MainThreadMarker) -> Retained<NSFont> {
    system_font(mtm, TITLE_FONT)
}

fn attributes(color: &NSColor, font: &NSFont) -> Retained<TextAttributes> {
    // SAFETY: both attribute names are statics the platform exports; the
    // values are the `NSFont`/`NSColor` each documents.
    unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[font.as_ref(), color.as_ref()],
        )
    }
}

/// Draws `text` with its baseline at `baseline` and its leading edge at
/// `x` in the flipped view.
fn draw_text(text: &NSString, attributes: &TextAttributes, font: &NSFont, x: f64, baseline: f64) {
    let point = NSPoint::new(x, baseline - font.ascender());
    // SAFETY: `drawAtPoint:withAttributes:` renders the string in the
    // current context; the dictionary carries the documented value types.
    unsafe { text.drawAtPoint_withAttributes(point, Some(attributes)) };
}

/// Draws `text` aligned to the trailing edge of the column at `column_x`
/// and returns the rect its advance occupies.
fn draw_trailing(
    text: &NSString,
    attributes: &TextAttributes,
    font: &NSFont,
    column_x: f64,
    baseline: f64,
    rtl: bool,
) -> CGRect {
    // SAFETY: `sizeWithAttributes:` measures under the attributes the draw
    // uses.
    let size = unsafe { text.sizeWithAttributes(Some(attributes)) };
    let x = if rtl {
        column_x + COLUMN_WIDTH - HIGHLIGHT_WIDTH + TEXT_TRAILING_INSET
    } else {
        column_x + HIGHLIGHT_WIDTH - TEXT_TRAILING_INSET - size.width
    };
    draw_text(text, attributes, font, x, baseline);
    CGRect::new(CGPoint::new(x, baseline - font.ascender()), size)
}

/// Fills a triangle inscribed in `rect`, pointing to the left edge when
/// `leftward`.
fn fill_triangle(context: &CGContext, rect: CGRect, leftward: bool, color: &NSColor) {
    let (left, right) = (rect.origin.x, rect.origin.x + rect.size.width);
    let (top, bottom) = (rect.origin.y, rect.origin.y + rect.size.height);
    let middle = f64::midpoint(top, bottom);
    let (tip, base) = if leftward {
        (left, right)
    } else {
        (right, left)
    };
    CGContext::set_fill_color_with_color(Some(context), Some(&color.CGColor()));
    CGContext::begin_path(Some(context));
    CGContext::move_to_point(Some(context), tip, middle);
    CGContext::add_line_to_point(Some(context), base, top);
    CGContext::add_line_to_point(Some(context), base, bottom);
    CGContext::close_path(Some(context));
    CGContext::fill_path(Some(context));
}

fn rounded_rect_path(rect: CGRect) -> objc2_core_foundation::CFRetained<CGPath> {
    // SAFETY: `rect` is plain data and a null transform is valid.
    unsafe { CGPath::with_rounded_rect(rect, HIGHLIGHT_RADIUS, HIGHLIGHT_RADIUS, std::ptr::null()) }
}

fn fill_rounded_rect(context: &CGContext, rect: CGRect, color: &NSColor) {
    CGContext::set_fill_color_with_color(Some(context), Some(&color.CGColor()));
    CGContext::add_path(Some(context), Some(&rounded_rect_path(rect)));
    CGContext::fill_path(Some(context));
}

fn stroke_rounded_rect(context: &CGContext, rect: CGRect, color: &NSColor) {
    let inset = CURSOR_RING_WIDTH / 2.0;
    let rect = CGRect::new(
        CGPoint::new(rect.origin.x + inset, rect.origin.y + inset),
        CGSize::new(
            2.0f64.mul_add(-inset, rect.size.width),
            2.0f64.mul_add(-inset, rect.size.height),
        ),
    );
    CGContext::set_stroke_color_with_color(Some(context), Some(&color.CGColor()));
    CGContext::set_line_width(Some(context), CURSOR_RING_WIDTH);
    CGContext::add_path(Some(context), Some(&rounded_rect_path(rect)));
    CGContext::stroke_path(Some(context));
}

/// What a [`CalendarElement`] stands for and the view it reads from.
struct CalendarElementIvars {
    view: Weak<CalendarView>,
    target: Target,
}

define_class!(
    // SAFETY: `NSAccessibilityElement`'s designated initializer is `init`,
    // which `CalendarElement::new` calls, and the class does not implement
    // `Drop`.
    #[unsafe(super(NSAccessibilityElement))]
    #[name = "CocoaUiCalendarElement"]
    #[ivars = CalendarElementIvars]
    struct CalendarElement;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSAccessibilityElement`
    // subclass.
    unsafe impl NSObjectProtocol for CalendarElement {}

    impl CalendarElement {
        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> CGRect {
            guarded("CalendarElement accessibilityFrame", || {
                self.ivars().view.load().map_or(CGRect::ZERO, |view| {
                    NSAccessibilityFrameInView(&view, view.target_rect(self.ivars().target))
                })
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityFrameInParentSpace))]
        fn accessibility_frame_in_parent_space(&self) -> CGRect {
            guarded("CalendarElement accessibilityFrameInParentSpace", || {
                self.ivars()
                    .view
                    .load()
                    .map_or(CGRect::ZERO, |view| view.target_rect(self.ivars().target))
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            guarded("CalendarElement accessibilityPerformPress", || {
                let target = self.ivars().target;
                if target == Target::Title {
                    return false;
                }
                self.ivars().view.load().is_some_and(|view| {
                    view.activate(target);
                    true
                })
            })
        }
    }
);

impl CalendarElement {
    /// The element for `target`, parented to `view`.
    fn new(view: &CalendarView, target: Target) -> Retained<Self> {
        let mtm = view.mtm();
        let this = mtm.alloc::<Self>().set_ivars(CalendarElementIvars {
            view: Weak::new(view),
            target,
        });
        // SAFETY: `init` is the class's designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let element: &NSAccessibilityElement = &this;
        let accessibility = ProtocolObject::<dyn NSAccessibility>::from_ref(element);
        accessibility.setAccessibilityElement(true);
        // SAFETY: the role constants are immutable `NSString`s `AppKit`
        // exports.
        let role = unsafe {
            if target == Target::Title {
                NSAccessibilityStaticTextRole
            } else {
                NSAccessibilityButtonRole
            }
        };
        accessibility.setAccessibilityRole(Some(role));
        // SAFETY: the view is the element's accessibility parent — an
        // `NSView` is a valid parent object.
        unsafe {
            accessibility.setAccessibilityParent(Some(AsRef::<AnyObject>::as_ref(view)));
        }
        this
    }
}

impl fmt::Debug for CalendarElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarElement")
            .field("target", &self.ivars().target)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(first_weekday: i32, visible: YearMonth) -> State {
        let mut text = LocaleText::new(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
            "en_US",
        )));
        text.first_weekday = first_weekday;
        let day = visible.first_day();
        let mut state = State {
            mode: CalendarSelectionMode::Single,
            visible,
            grid: [GridDay {
                parts: day,
                in_month: true,
            }; GRID_DAYS],
            selected: BTreeSet::new(),
            decorated: BTreeSet::new(),
            range: day..=day,
            cursor: day,
            colors: CalendarColors::system(),
            text,
            navigation_labels: CalendarNavigationLabels {
                previous: "Previous Month".to_owned(),
                selection: "Selected Month".to_owned(),
                next: "Next Month".to_owned(),
            },
        };
        state.rebuild_grid();
        state
    }

    #[test]
    fn grid_starts_on_the_locale_first_weekday() {
        let november = YearMonth {
            year: 2026,
            month: 11,
        };
        // 1 November 2026 is a Sunday: a Sunday-first grid opens on it.
        let sunday_first = state(1, november);
        assert_eq!(sunday_first.grid[0].parts, november.first_day());
        assert!(sunday_first.grid[0].in_month);
        assert_eq!(
            sunday_first.grid[41].parts,
            DateParts {
                year: 2026,
                month: 12,
                day: 12
            }
        );
        // A Monday-first grid leads with six October days.
        let monday_first = state(2, november);
        assert_eq!(
            monday_first.grid[0].parts,
            DateParts {
                year: 2026,
                month: 10,
                day: 26
            }
        );
        assert!(!monday_first.grid[5].in_month);
        assert_eq!(monday_first.grid[6].parts, november.first_day());
        assert_eq!(
            monday_first.grid.iter().filter(|day| day.in_month).count(),
            30
        );
    }

    #[test]
    fn locale_text_uses_the_two_letter_weekday_symbols() {
        let text = LocaleText::new(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
            "en_US",
        )));
        assert_eq!(text.weekdays[0], "Su");
        assert_eq!(text.weekdays[6], "Sa");
        assert_eq!(text.first_weekday, 1);
        let german = LocaleText::new(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
            "de_DE",
        )));
        assert_eq!(german.first_weekday, 2);
    }
}
