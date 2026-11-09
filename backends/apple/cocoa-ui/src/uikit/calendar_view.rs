//! The `UIKit` calendar: a `UICalendarView` with single- or multi-date
//! selection, per-day decorations and a reported visible month, driven
//! entirely by closures.
//!
//! The calendar's delegate protocols land on one `NSObject` the view owns
//! through this wrapper: the selection policy (`can_select`), the pick
//! callback (`on_pick`), the decoration lookup (`decoration`) and the
//! visible-month callback (`on_visible_month_change`) are ordinary Rust
//! closures reading shared state.
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate class and calls `objc2` bindings
//! marked unsafe because `UIKit` view and delegate APIs are main-thread
//! only — which the `MainThreadOnly` thread kinds and [`MainThreadMarker`]
//! constructor guarantee. Every delegate method the frameworks call runs
//! inside the panic guard.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSDateComponents, NSDateInterval, NSLocale, NSObject, NSObjectProtocol, NSString,
};
use objc2_ui_kit::{
    UICalendarSelectionMultiDate, UICalendarSelectionMultiDateDelegate,
    UICalendarSelectionSingleDate, UICalendarSelectionSingleDateDelegate, UICalendarView,
    UICalendarViewDecoration, UICalendarViewDecorationSize, UICalendarViewDelegate, UIColor,
};

use crate::callback::guarded;
use crate::date::{self, CalendarSelectionMode, DateParts, YearMonth};
use crate::geometry::Size;

/// A closure answering the decoration a date draws with; `None` means
/// no decoration for the date.
type DecorationFn = Box<dyn Fn(DateParts) -> Option<Retained<UIColor>>>;

/// The closures the delegate consults; `None` answers the permissive
/// defaults (`true` for selection, no decoration).
struct Shared {
    can_select: Option<Box<dyn Fn(DateParts) -> bool>>,
    on_pick: Option<Box<dyn Fn(DateParts)>>,
    on_visible_month_change: Option<Box<dyn Fn(YearMonth)>>,
    decoration: Option<DecorationFn>,
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
                    decoration: None,
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
                let color = self
                    .ivars()
                    .shared
                    .borrow()
                    .decoration
                    .as_ref()
                    .and_then(|decoration| decoration(parts))?;
                Some(UICalendarViewDecoration::decorationWithColor_size(
                    Some(&color),
                    UICalendarViewDecorationSize::Small,
                    self.mtm(),
                ))
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
                let (Ok(year), Ok(month)) = (
                    i32::try_from(visible.year()),
                    i32::try_from(visible.month()),
                ) else {
                    return;
                };
                if let Some(handler) = &self.ivars().shared.borrow().on_visible_month_change {
                    handler(YearMonth { year, month });
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
    /// A calendar selecting in `mode`, with no closures installed yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, mode: CalendarSelectionMode) -> Self {
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
        Self {
            view,
            selection,
            _delegate: delegate,
            shared,
        }
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

    /// Formats the calendar for the locale `identifier` names.
    pub fn set_locale(&self, identifier: &str) {
        self.view
            .setLocale(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
                identifier,
            )));
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

    /// Pushes `selected` and `decorated` onto the control: the selection is
    /// replaced wholesale — a single-date calendar takes the first day —
    /// then both day sets get their decorations re-asked of the delegate.
    pub fn sync(&self, selected: &[DateParts], decorated: &[DateParts]) {
        let selected_components = NSArray::from_retained_slice(
            &selected
                .iter()
                .map(|d| date::components(*d))
                .collect::<Vec<_>>(),
        );
        let decorated_components = NSArray::from_retained_slice(
            &decorated
                .iter()
                .map(|d| date::components(*d))
                .collect::<Vec<_>>(),
        );
        self.shared.borrow().syncing.set(true);
        match &self.selection {
            Selection::Single(selection) => {
                selection.setSelectedDate_animated(
                    selected.first().map(|d| date::components(*d)).as_deref(),
                    false,
                );
            }
            Selection::Multiple(selection) => selection.setSelectedDates(&selected_components),
        }
        self.view
            .reloadDecorationsForDateComponents_animated(&selected_components, false);
        self.view
            .reloadDecorationsForDateComponents_animated(&decorated_components, false);
        self.shared.borrow().syncing.set(false);
    }

    /// Re-asks decorations for `dates` only — the decoration-color update
    /// path.
    pub fn reload_decorations(&self, dates: &[DateParts]) {
        let components = NSArray::from_retained_slice(
            &dates
                .iter()
                .map(|d| date::components(*d))
                .collect::<Vec<_>>(),
        );
        self.view
            .reloadDecorationsForDateComponents_animated(&components, false);
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

    /// `handler` answers the color a decorated day draws its dot in;
    /// `None` means no decoration.
    pub fn set_decoration(
        &self,
        handler: impl Fn(DateParts) -> Option<Retained<UIColor>> + 'static,
    ) {
        self.shared.borrow_mut().decoration = Some(Box::new(handler));
    }

    /// The view's intrinsic size — what a measure pass reports.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.view.intrinsicContentSize();
        Size::new(size.width, size.height)
    }

    /// Names the view to a screen reader; `None` leaves it unnamed.
    /// Setting a label marks the view an accessibility element.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        use objc2_ui_kit::NSObjectUIAccessibility;
        let mtm = MainThreadMarker::from(&*self.view);
        self.view.setIsAccessibilityElement(label.is_some(), mtm);
        self.view
            .setAccessibilityLabel(label.map(NSString::from_str).as_deref(), mtm);
    }
}
