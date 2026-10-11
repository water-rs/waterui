//! The `UIKit` date picker: a `UIDatePicker` in the compact style with an
//! installable value-change action.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIDatePicker` subclass and calls `objc2`
//! bindings marked unsafe because `UIKit` control APIs are main-thread
//! only — which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSDate, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    NSObjectUIAccessibility, UIControl, UIDatePicker, UIDatePickerMode, UIDatePickerStyle,
};

use crate::ActionTarget;
use crate::action::ControlEvents;
use crate::geometry::Size;

/// Which fields the picker edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePickerMode {
    /// Year, month and day.
    Date,
    /// Hour and minute (the compact picker shows no seconds column; the
    /// leaf pairs this mode with a seconds stepper).
    Time,
    /// Date plus hour and minute.
    DateAndTime,
}

impl DatePickerMode {
    const fn native(self) -> UIDatePickerMode {
        match self {
            Self::Date => UIDatePickerMode::Date,
            Self::Time => UIDatePickerMode::Time,
            Self::DateAndTime => UIDatePickerMode::DateAndTime,
        }
    }
}

/// A `UIDatePicker` used as a value leaf.
pub struct DatePickerIvars {}

impl fmt::Debug for DatePickerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatePickerIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UIDatePicker`'s designated initializer is `initWithFrame:`,
    // which `DatePicker::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIDatePicker))]
    #[name = "CocoaUiDatePicker"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DatePickerIvars]
    #[derive(Debug)]
    /// A compact `UIDatePicker` with an installable value-change action.
    pub struct DatePicker;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIDatePicker` subclass.
    unsafe impl NSObjectProtocol for DatePicker {}
);

impl DatePicker {
    /// A compact picker editing `mode`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, mode: DatePickerMode) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DatePickerIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setPreferredDatePickerStyle(UIDatePickerStyle::Compact);
        this.setDatePickerMode(mode.native());
        this
    }

    /// The date the picker currently shows.
    #[must_use]
    pub fn date(&self) -> Retained<NSDate> {
        UIDatePicker::date(self)
    }

    /// Sets the date the picker shows.
    pub fn set_date(&self, date: &NSDate) {
        UIDatePicker::setDate(self, date);
    }

    /// The inclusive bounds the picker accepts; `None` removes the bound.
    pub fn set_range(&self, start: Option<&NSDate>, end: Option<&NSDate>) {
        self.setMinimumDate(start);
        self.setMaximumDate(end);
    }

    /// Whether the picker responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// Calls `handler` each time the user edits the picker's value. The
    /// returned target must be kept for as long as the control should
    /// respond; dropping it detaches the target.
    pub fn install_action(&self, handler: impl Fn() + 'static) -> ActionTarget {
        let control: &UIControl = self;
        ActionTarget::new(control, ControlEvents::VALUE_CHANGED, move |_mtm| handler())
    }

    /// The control's intrinsic size — what a measure pass reports.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.intrinsicContentSize();
        Size::new(size.width, size.height)
    }

    /// Names the control to a screen reader; `None` leaves it unnamed.
    /// Setting a label marks the view an accessibility element.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        let mtm = MainThreadMarker::from(self);
        self.setIsAccessibilityElement(label.is_some(), mtm);
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref(), mtm);
    }
}
