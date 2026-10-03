//! The `AppKit` date picker: an `NSDatePicker` in its text-field-and-stepper
//! style, with installable change action.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSDatePicker` subclass and calls `objc2`
//! bindings marked unsafe because `AppKit` control APIs are main-thread
//! only — which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSControl, NSDatePicker, NSDatePickerElementFlags, NSDatePickerStyle, NSFont,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSDate, NSObjectProtocol, NSString};

use crate::ActionTarget;
use crate::geometry::Size;

/// Which fields the picker edits, mirroring `NSDatePickerElementFlags`
/// combinations the framework supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePickerElements {
    /// Year, month and day.
    Date,
    /// Hour and minute.
    HourMinute,
    /// Hour, minute and second.
    HourMinuteSecond,
    /// Date plus hour and minute.
    DateHourMinute,
    /// Date plus hour, minute and second.
    DateHourMinuteSecond,
}

/// An `NSDatePicker` used as a value leaf.
pub struct DatePickerIvars {}

impl fmt::Debug for DatePickerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatePickerIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSDatePicker`'s designated initializer is `initWithFrame:`,
    // which `DatePicker::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSDatePicker))]
    #[name = "CocoaUiDatePicker"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DatePickerIvars]
    #[derive(Debug)]
    /// An `NSDatePicker` with an installable value-change action.
    pub struct DatePicker;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSDatePicker` subclass.
    unsafe impl NSObjectProtocol for DatePicker {}
);

impl DatePicker {
    /// A text-field-and-stepper picker editing `elements`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, elements: DatePickerElements) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DatePickerIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setDatePickerStyle(NSDatePickerStyle::TextFieldAndStepper);
        this.set_elements(elements);
        this
    }

    /// The fields the picker edits.
    pub fn set_elements(&self, elements: DatePickerElements) {
        use NSDatePickerElementFlags as F;
        let flags = match elements {
            DatePickerElements::Date => F::YearMonthDay,
            DatePickerElements::HourMinute => F::HourMinute,
            DatePickerElements::HourMinuteSecond => F::HourMinuteSecond,
            DatePickerElements::DateHourMinute => F::YearMonthDay | F::HourMinute,
            DatePickerElements::DateHourMinuteSecond => F::YearMonthDay | F::HourMinuteSecond,
        };
        self.setDatePickerElements(flags);
    }

    /// The date the picker currently shows.
    #[must_use]
    pub fn date(&self) -> Retained<NSDate> {
        self.dateValue()
    }

    /// Sets the date the picker shows.
    pub fn set_date(&self, date: &NSDate) {
        self.setDateValue(date);
    }

    /// The inclusive bounds the picker accepts; `None` removes the bound.
    pub fn set_range(&self, start: Option<&NSDate>, end: Option<&NSDate>) {
        self.setMinDate(start);
        self.setMaxDate(end);
    }

    /// The typeface the field's text draws in.
    pub fn set_font(&self, font: &NSFont) {
        self.setFont(Some(font));
    }

    /// Whether the picker responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// Calls `handler` each time the user edits the picker's value. The
    /// returned target must be kept for as long as the control should
    /// respond; dropping it detaches the target.
    pub fn install_action(&self, handler: impl Fn() + 'static) -> ActionTarget {
        let control: &NSControl = self;
        ActionTarget::new(control, move |_mtm| handler())
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
        self.setAccessibilityElement(label.is_some());
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
    }
}
