//! The `UIKit` calendar: a `UICalendarView` with multi-date selection and
//! per-day decorations, driven entirely by closures.
//!
//! The calendar's two delegate protocols land on one `NSObject` the view
//! owns through this wrapper: the selection policy (`can_select`), the
//! toggle callback (`on_toggle`), and the decoration lookup (`decoration`)
//! are ordinary Rust closures reading shared state.
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
use objc2_foundation::{NSArray, NSDateComponents, NSObject, NSObjectProtocol};
use objc2_ui_kit::{
    UICalendarSelectionMultiDate, UICalendarSelectionMultiDateDelegate, UICalendarView,
    UICalendarViewDecoration, UICalendarViewDecorationSize, UICalendarViewDelegate, UIColor,
};

use crate::callback::guarded;
use crate::date::{self, DateParts};
use crate::geometry::Size;
use objc2_foundation::NSDateInterval;

/// A closure answering the decoration a date draws with; `None` means
/// no decoration for the date.
type DecorationFn = Box<dyn Fn(DateParts) -> Option<Retained<UIColor>>>;

/// The closures the delegate consults; `None` answers the permissive
/// defaults (`true` for selection, no decoration).
struct Shared {
    can_select: Option<Box<dyn Fn(DateParts) -> bool>>,
    on_toggle: Option<Box<dyn Fn(DateParts)>>,
    decoration: Option<DecorationFn>,
    /// `true` while the wrapper pushes a model-driven selection onto the
    /// control — delegate callbacks then ignore the echo.
    syncing: Cell<bool>,
}

/// The delegate object: both `UICalendarSelectionMultiDateDelegate` and
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
                    on_toggle: None,
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
                self.toggle(date_components);
            });
        }

        #[unsafe(method(multiDateSelection:didDeselectDate:))]
        fn did_deselect_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) {
            guarded("CalendarDelegate didDeselectDate:", || {
                self.toggle(date_components);
            });
        }

        #[unsafe(method(multiDateSelection:canSelectDate:))]
        fn can_select_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) -> bool {
            guarded("CalendarDelegate canSelectDate:", || {
                self.can_toggle(date_components)
            })
        }

        #[unsafe(method(multiDateSelection:canDeselectDate:))]
        fn can_deselect_date(
            &self,
            _selection: &UICalendarSelectionMultiDate,
            date_components: &NSDateComponents,
        ) -> bool {
            guarded("CalendarDelegate canDeselectDate:", || {
                self.can_toggle(date_components)
            })
        }
    }

    // SAFETY: the one optional method reads the decoration lookup and
    // builds the decoration object on the main thread.
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
    }
);

impl CalendarDelegate {
    /// The toggle callback, suppressed while the model syncs the control.
    fn toggle(&self, date_components: &NSDateComponents) {
        let Some(parts) = date::parts(date_components) else {
            return;
        };
        let shared = self.ivars().shared.borrow();
        if shared.syncing.get() {
            return;
        }
        if let Some(on_toggle) = &shared.on_toggle {
            on_toggle(parts);
        }
    }

    /// The selection policy; permissive when unset or when the day fails
    /// to resolve.
    fn can_toggle(&self, date_components: &NSDateComponents) -> bool {
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

/// A `UICalendarView` with multi-date selection and decoration dots.
///
/// Owns the view, the selection behaviour and the delegate object; the
/// closures install after construction.
pub struct CalendarView {
    view: Retained<UICalendarView>,
    selection: Retained<UICalendarSelectionMultiDate>,
    _delegate: Retained<CalendarDelegate>,
    shared: Rc<RefCell<Shared>>,
}

impl fmt::Debug for CalendarView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarView").finish_non_exhaustive()
    }
}

impl CalendarView {
    /// A calendar with multi-date selection but no closures installed yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let view = UICalendarView::new(mtm);
        // SAFETY: `init` is `NSObject`'s designated initializer, which the
        // delegate forwards to its superclass.
        let delegate: Retained<CalendarDelegate> =
            unsafe { msg_send![CalendarDelegate::alloc(mtm), init] };
        let selection = UICalendarSelectionMultiDate::initWithDelegate(
            UICalendarSelectionMultiDate::alloc(mtm),
            Some(ProtocolObject::from_ref(&*delegate)),
        );
        view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        view.setSelectionBehavior(Some(&selection));
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

    /// The range of days the calendar shows and allows toggling within.
    pub fn set_available_range(&self, range: &NSDateInterval) {
        self.view.setAvailableDateRange(range);
    }

    /// Pushes `selected` and `decorated` onto the control: the selection
    /// array is replaced wholesale, then both day sets get their
    /// decorations re-asked of the delegate.
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
        self.selection.setSelectedDates(&selected_components);
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

    /// `handler` answers whether the user may toggle `day` — the range
    /// check the delegate applies to selection and deselection alike.
    pub fn set_can_select(&self, handler: impl Fn(DateParts) -> bool + 'static) {
        self.shared.borrow_mut().can_select = Some(Box::new(handler));
    }

    /// `handler` is called with the day the user tapped — after the
    /// control has already flipped its local selection state; the owner
    /// writes the model and [`sync`][Self::sync] echoes it back.
    pub fn on_toggle(&self, handler: impl Fn(DateParts) + 'static) {
        self.shared.borrow_mut().on_toggle = Some(Box::new(handler));
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
        use objc2_foundation::NSString;
        use objc2_ui_kit::NSObjectUIAccessibility;
        let mtm = MainThreadMarker::from(&*self.view);
        self.view.setIsAccessibilityElement(label.is_some(), mtm);
        self.view
            .setAccessibilityLabel(label.map(NSString::from_str).as_deref(), mtm);
    }
}
