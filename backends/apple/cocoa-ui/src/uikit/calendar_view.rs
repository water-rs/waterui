//! The `UIKit` calendar: a `UICalendarView` with single- or multi-date
//! selection, per-day decorations and a reported visible month, driven
//! entirely by closures.
//!
//! The calendar's delegate protocols land on one `NSObject` the view owns
//! through this wrapper: the selection policy (`can_select`), the pick
//! callback (`on_pick`) and the visible-month callback
//! (`on_visible_month_change`) are ordinary Rust closures, and the
//! decorated days are a set the decoration lookup reads. A decorated day
//! carries the platform's default decoration.
//!
//! The view computes in the Gregorian calendar of the locale it is given,
//! whatever calendar the device uses, so the date components it receives
//! and reports are always Gregorian.
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate class and calls `objc2` bindings
//! marked unsafe because `UIKit` view and delegate APIs are main-thread
//! only — which the `MainThreadOnly` thread kinds and [`MainThreadMarker`]
//! constructor guarantee. Every delegate method the frameworks call runs
//! inside the panic guard.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSDateComponents, NSDateInterval, NSLocale, NSObject, NSObjectProtocol, NSString,
};
use objc2_ui_kit::{
    UIAccessibilityContainerType, UICalendarSelectionMultiDate,
    UICalendarSelectionMultiDateDelegate, UICalendarSelectionSingleDate,
    UICalendarSelectionSingleDateDelegate, UICalendarView, UICalendarViewDecoration,
    UICalendarViewDecorationSize, UICalendarViewDelegate, UILayoutPriorityFittingSizeLevel,
    UILayoutPriorityRequired,
};

use crate::callback::guarded;
use crate::date::{self, CalendarSelectionMode, DateParts, YearMonth};
use crate::geometry::Size;

/// The closures the delegate consults — `None` answers the permissive
/// default (`true` for selection) — and the decorated days.
struct Shared {
    can_select: Option<Box<dyn Fn(DateParts) -> bool>>,
    on_pick: Option<Box<dyn Fn(DateParts)>>,
    on_visible_month_change: Option<Box<dyn Fn(YearMonth)>>,
    decorated: BTreeSet<DateParts>,
    /// `true` while the wrapper pushes a model-driven selection onto the
    /// control — delegate callbacks then ignore the echo.
    syncing: Cell<bool>,
}

/// The delegate object: the selection delegate of either mode and
/// `UICalendarViewDelegate`, backed by [`Shared`].
struct DelegateIvars {
    shared: Rc<RefCell<Shared>>,
}

impl fmt::Debug for DelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelegateIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSObject`'s designated initializer is `init`, which the
    // override below calls, and the class does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiCalendarDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    /// Selection and decoration delegate of a [`CalendarView`].
    struct CalendarDelegate;

    impl CalendarDelegate {
        // SAFETY: `init` is `NSObject`'s designated initializer.
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(DelegateIvars {
                shared: Rc::new(RefCell::new(Shared {
                    can_select: None,
                    on_pick: None,
                    on_visible_month_change: None,
                    decorated: BTreeSet::new(),
                    syncing: Cell::new(false),
                })),
            });
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for CalendarDelegate {}

    // SAFETY: every method reads shared state and calls the owner's
    // closures — main-thread work only.
    unsafe impl UICalendarSelectionMultiDateDelegate for CalendarDelegate {
        #[unsafe(method(multiDateSelection:didSelectDate:))]
        fn did_select_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) {
            guarded("CalendarDelegate didSelectDate:", || {
                self.pick(date_components);
            });
        }

        #[unsafe(method(multiDateSelection:didDeselectDate:))]
        fn did_deselect_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) {
            guarded("CalendarDelegate didDeselectDate:", || {
                self.pick(date_components);
            });
        }

        #[unsafe(method(multiDateSelection:canSelectDate:))]
        fn can_select_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) -> bool {
            guarded("CalendarDelegate canSelectDate:", || {
                self.can_pick(date_components)
            })
        }

        #[unsafe(method(multiDateSelection:canDeselectDate:))]
        fn can_deselect_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) -> bool {
            guarded("CalendarDelegate canDeselectDate:", || {
                self.can_pick(date_components)
            })
        }
    }

    // SAFETY: every method reads shared state and calls the owner's
    // closures — main-thread work only.
    unsafe impl UICalendarSelectionSingleDateDelegate for CalendarDelegate {
        #[unsafe(method(dateSelection:didSelectDate:))]
        fn single_did_select_date(
            &self,
            _selection: &UICalendarSelectionSingleDate,
            date_components: Option<&NSDateComponents>,
        ) {
            guarded("CalendarDelegate dateSelection:didSelectDate:", || {
                if let Some(date_components) = date_components {
                    self.pick(date_components);
                }
            });
        }

        #[unsafe(method(dateSelection:canSelectDate:))]
        fn single_can_select_date(
            &self,
            _selection: &UICalendarSelectionSingleDate,
            date_components: Option<&NSDateComponents>,
        ) -> bool {
            guarded("CalendarDelegate dateSelection:canSelectDate:", || {
                // A `nil` day is the deselection of the selected one; a
                // single-date calendar always holds a day.
                date_components.is_some_and(|date_components| self.can_pick(date_components))
            })
        }
    }

    // SAFETY: both optional methods read shared state on the main thread.
    unsafe impl UICalendarViewDelegate for CalendarDelegate {
        #[unsafe(method_id(calendarView:decorationForDateComponents:))]
        fn decoration_for_date_components(
            &self,
            _calendar_view: &UICalendarView,
            date_components: &NSDateComponents,
        ) -> Option<Retained<UICalendarViewDecoration>> {
            guarded("CalendarDelegate decorationForDateComponents:", || {
                let parts = date::parts(date_components)?;
                self.ivars()
                    .shared
                    .borrow()
                    .decorated
                    .contains(&parts)
                    .then(|| {
                        // No color: the platform's default decoration.
                        UICalendarViewDecoration::decorationWithColor_size(
                            None,
                            UICalendarViewDecorationSize::Small,
                            self.mtm(),
                        )
                    })
            })
        }

        #[unsafe(method(calendarView:didChangeVisibleDateComponentsFrom:))]
        fn did_change_visible_date_components(
            &self,
            calendar_view: &UICalendarView,
            _previous_date_components: &NSDateComponents,
        ) {
            guarded("CalendarDelegate didChangeVisibleDateComponentsFrom:", || {
                let visible = calendar_view.visibleDateComponents();
                // The view only shows months inside its available range,
                // which comes from `DateParts`, so both fit `i32`.
                let month = YearMonth {
                    year: i32::try_from(visible.year())
                        .expect("UICalendarView reported a year outside its available range"),
                    month: i32::try_from(visible.month())
                        .expect("UICalendarView reported a month outside 1 through 12"),
                };
                if let Some(handler) = &self.ivars().shared.borrow().on_visible_month_change {
                    handler(month);
                }
            });
        }
    }
);

impl CalendarDelegate {
    /// The pick callback, suppressed while the model syncs the control.
    fn pick(&self, date_components: &NSDateComponents) {
        let Some(parts) = date::parts(date_components) else {
            return;
        };
        let shared = self.ivars().shared.borrow();
        if shared.syncing.get() {
            return;
        }
        if let Some(on_pick) = &shared.on_pick {
            on_pick(parts);
        }
    }

    /// The selection policy; permissive when unset, closed when the day
    /// fails to resolve.
    fn can_pick(&self, date_components: &NSDateComponents) -> bool {
        let Some(parts) = date::parts(date_components) else {
            return false;
        };
        self.ivars()
            .shared
            .borrow()
            .can_select
            .as_ref()
            .is_none_or(|can_select| can_select(parts))
    }
}

/// The selection behaviour a [`CalendarView`] installed.
enum Selection {
    Single(Retained<UICalendarSelectionSingleDate>),
    Multiple(Retained<UICalendarSelectionMultiDate>),
}

/// A `UICalendarView` with single- or multi-date selection and decoration
/// dots.
///
/// Owns the view, the selection behaviour and the delegate object; the
/// closures install after construction.
pub struct CalendarView {
    view: Retained<UICalendarView>,
    selection: Selection,
    _delegate: Retained<CalendarDelegate>,
    shared: Rc<RefCell<Shared>>,
}

impl fmt::Debug for CalendarView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarView").finish_non_exhaustive()
    }
}

impl CalendarView {
    /// A calendar selecting in `mode`, formatted for the locale `locale`
    /// names, with no closures installed yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, mode: CalendarSelectionMode, locale: &str) -> Self {
        let view = UICalendarView::new(mtm);
        // SAFETY: `init` is `NSObject`'s designated initializer, which the
        // delegate forwards to its superclass.
        let delegate: Retained<CalendarDelegate> =
            unsafe { msg_send![CalendarDelegate::alloc(mtm), init] };
        let selection = match mode {
            CalendarSelectionMode::Single => {
                let selection = UICalendarSelectionSingleDate::initWithDelegate(
                    UICalendarSelectionSingleDate::alloc(mtm),
                    Some(ProtocolObject::from_ref(&*delegate)),
                );
                view.setSelectionBehavior(Some(&selection));
                Selection::Single(selection)
            }
            CalendarSelectionMode::Multiple => {
                let selection = UICalendarSelectionMultiDate::initWithDelegate(
                    UICalendarSelectionMultiDate::alloc(mtm),
                    Some(ProtocolObject::from_ref(&*delegate)),
                );
                view.setSelectionBehavior(Some(&selection));
                Selection::Multiple(selection)
            }
        };
        view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let shared = Rc::clone(&delegate.ivars().shared);
        let this = Self {
            view,
            selection,
            _delegate: delegate,
            shared,
        };
        this.set_locale(locale);
        this
    }

    /// The view the calendar renders; add it to a container and frame it
    /// like any other view.
    #[must_use]
    pub fn view(&self) -> &objc2_ui_kit::UIView {
        &self.view
    }

    /// The range of days the calendar shows and allows picking within.
    pub fn set_available_range(&self, range: &NSDateInterval) {
        self.view.setAvailableDateRange(range);
    }

    /// Formats the calendar for the locale `identifier` names, in that
    /// locale's Gregorian calendar — its first weekday included.
    pub fn set_locale(&self, identifier: &str) {
        let locale = NSLocale::localeWithLocaleIdentifier(&NSString::from_str(identifier));
        let calendar = date::gregorian();
        calendar.setLocale(Some(&locale));
        self.view.setCalendar(&calendar);
        self.view.setLocale(&locale);
    }

    /// Shows `month` without animation; reports nothing back.
    pub fn set_visible_month(&self, month: YearMonth) {
        let current = self.view.visibleDateComponents();
        if i32::try_from(current.year()) == Ok(month.year)
            && i32::try_from(current.month()) == Ok(month.month)
        {
            return;
        }
        self.view
            .setVisibleDateComponents_animated(&date::components(month.first_day()), false);
    }

    /// Replaces the selection with `selected` — a single-date calendar
    /// takes the first day — without reporting it back as a pick.
    pub fn set_selection(&self, selected: &[DateParts]) {
        self.shared.borrow().syncing.set(true);
        match &self.selection {
            Selection::Single(selection) => {
                selection.setSelectedDate_animated(
                    selected.first().map(|d| date::components(*d)).as_deref(),
                    false,
                );
            }
            Selection::Multiple(selection) => {
                selection.setSelectedDates(&components_array(selected.iter().copied()));
            }
        }
        self.shared.borrow().syncing.set(false);
    }

    /// Replaces the decorated days with `decorated` and re-asks the
    /// decoration of every day that gained or lost one.
    pub fn set_decorations(&self, decorated: impl IntoIterator<Item = DateParts>) {
        let decorated: BTreeSet<DateParts> = decorated.into_iter().collect();
        let previous =
            std::mem::replace(&mut self.shared.borrow_mut().decorated, decorated.clone());
        let changed = previous.symmetric_difference(&decorated).copied();
        self.view
            .reloadDecorationsForDateComponents_animated(&components_array(changed), false);
    }

    /// `handler` answers whether the user may pick `day` — the range check
    /// the delegate applies to selection and deselection alike.
    pub fn set_can_select(&self, handler: impl Fn(DateParts) -> bool + 'static) {
        self.shared.borrow_mut().can_select = Some(Box::new(handler));
    }

    /// `handler` is called with the day the user tapped — in single mode
    /// the day to select, in multiple mode the day to toggle — after the
    /// control has already updated its local selection; the owner writes
    /// the model and [`sync`][Self::sync] echoes it back.
    pub fn on_pick(&self, handler: impl Fn(DateParts) + 'static) {
        self.shared.borrow_mut().on_pick = Some(Box::new(handler));
    }

    /// `handler` is called with the month the user scrolled or navigated
    /// to.
    pub fn on_visible_month_change(&self, handler: impl Fn(YearMonth) + 'static) {
        self.shared.borrow_mut().on_visible_month_change = Some(Box::new(handler));
    }

    /// The view's intrinsic size — its natural width.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.view.intrinsicContentSize();
        Size::new(size.width, size.height)
    }

    /// The size the calendar lays out at `width`: its own fitting measure,
    /// required horizontally and free vertically, so every week row of the
    /// shown month fits.
    #[must_use]
    pub fn fitting_size(&self, width: f64) -> Size {
        let size = self
            .view
            .systemLayoutSizeFittingSize_withHorizontalFittingPriority_verticalFittingPriority(
                objc2_core_foundation::CGSize::new(width, 0.0),
                UILayoutPriorityRequired,
                UILayoutPriorityFittingSizeLevel,
            );
        Size::new(size.width, size.height)
    }

    /// Names the calendar to a screen reader; `None` leaves it unnamed.
    ///
    /// The calendar stays a container: `VoiceOver` announces the name as it
    /// enters the calendar's semantic group and still reaches every day and
    /// month control inside.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        use objc2_ui_kit::{NSObjectUIAccessibility, NSObjectUIAccessibilityContainer};
        let mtm = MainThreadMarker::from(&*self.view);
        self.view
            .setAccessibilityContainerType(UIAccessibilityContainerType::SemanticGroup, mtm);
        self.view
            .setAccessibilityLabel(label.map(NSString::from_str).as_deref(), mtm);
    }
}

/// `days` as the `NSDateComponents` array the calendar's batch APIs take.
fn components_array(
    days: impl IntoIterator<Item = DateParts>,
) -> Retained<NSArray<NSDateComponents>> {
    NSArray::from_retained_slice(&days.into_iter().map(date::components).collect::<Vec<_>>())
}
