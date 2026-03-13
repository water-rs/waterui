use crate::components::text::WuiText;
use crate::id::WuiId;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{WuiAnyView, WuiStr};
use alloc::vec::Vec;
use jiff::civil::{Date, DateTime};
use waterui::text::styled::StyledStr;
use waterui::{
    Color, Str,
    component::{
        menu::ResolvedMenuItem,
        slider::SliderConfig,
        stepper::StepperConfig,
        text_field::{KeyboardType, ResolvedTextFieldConfig},
        toggle::{ToggleConfig, ToggleStyle},
    },
};
use waterui_core::id::Id;
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::date::{DatePickerConfig, DatePickerType};
use waterui_form::picker::{PickerConfig, PickerItem, PickerStyle};
use waterui_form::secure::{Secure, SecureFieldConfig};

into_ffi! {KeyboardType, Text, pub enum WuiKeyboardType {
    Text,
    Email,
    URL,
    Number,
    PhoneNumber
}}

into_ffi! {ResolvedTextFieldConfig,
    pub struct WuiTextField {
        label: *mut WuiAnyView,
        value: *mut WuiBinding<StyledStr>,
        prompt: WuiText,
        keyboard: WuiKeyboardType,
        selection_menu: *mut WuiComputed<Vec<ResolvedMenuItem>>,
    }
}

into_ffi! {ToggleStyle, Automatic, pub enum WuiToggleStyle {
    Automatic,
    Switch,
    Checkbox,
}}

into_ffi! {ToggleConfig,
    pub struct WuiToggle {
        label: *mut WuiAnyView,
        toggle: *mut WuiBinding<bool>,
        style: WuiToggleStyle,
    }
}

/// C representation of a range
#[repr(C)]
pub struct WuiRange<T> {
    /// Start of the range
    pub start: T,
    /// End of the range
    pub end: T,
}

into_ffi! {SliderConfig,
    pub struct WuiSlider {
        label: *mut WuiAnyView,
        min_value_label: *mut WuiAnyView,
        max_value_label: *mut WuiAnyView,
        range: WuiRange<f64>,
        value: *mut WuiBinding<f64>,
    }
}

into_ffi! {StepperConfig,
    pub struct WuiStepper {
        value: *mut WuiBinding<i32>,
        step: *mut WuiComputed<i32>,
        label: *mut WuiAnyView,
        range: WuiRange<i32>,
    }
}

// Implement RangeInclusive conversions
use crate::IntoFFI;
use core::ops::RangeInclusive;

impl IntoFFI for RangeInclusive<f64> {
    type FFI = WuiRange<f64>;
    fn into_ffi(self) -> Self::FFI {
        WuiRange {
            start: *self.start(),
            end: *self.end(),
        }
    }
}

impl IntoFFI for RangeInclusive<i32> {
    type FFI = WuiRange<i32>;
    fn into_ffi(self) -> Self::FFI {
        WuiRange {
            start: *self.start(),
            end: *self.end(),
        }
    }
}

// FFI view bindings for form components
ffi_view!(ResolvedTextFieldConfig, WuiTextField, text_field);

ffi_view!(ToggleConfig, WuiToggle, toggle);

ffi_view!(SliderConfig, WuiSlider, slider);

ffi_view!(StepperConfig, WuiStepper, stepper);

ffi_view!(ColorPickerConfig, WuiColorPicker, color_picker);

ffi_view!(PickerConfig, WuiPicker, picker);

ffi_view!(SecureFieldConfig, WuiSecureField, secure_field);

into_ffi! {PickerStyle, Automatic, pub enum WuiPickerStyle {
    Automatic,
    Menu,
    Radio,
}}

into_ffi! {PickerConfig,
    pub struct WuiPicker {
        items: *mut WuiComputed<Vec<PickerItem<Id>>>,
        selection: *mut WuiBinding<Id>,
        style: WuiPickerStyle,
    }
}

into_ffi! {PickerItem<Id>,
    pub struct WuiPickerItem {
        tag: WuiId,
        content: WuiText,
    }
}

into_ffi! {ColorPickerConfig,
    pub struct WuiColorPicker {
        label: *mut WuiAnyView,
        value: *mut WuiBinding<Color>,
        support_alpha: bool,
        support_hdr: bool,
    }
}

// Secure type FFI - uses WuiStr representation
// The Secure type is treated as a string at the FFI boundary
impl IntoFFI for Secure {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        use alloc::string::String;
        // Create an owned string from the exposed value before Secure is dropped
        let owned = String::from(self.expose());
        Str::from(owned).into_ffi()
    }
}

into_ffi! {SecureFieldConfig,
    pub struct WuiSecureField {
        label: *mut WuiAnyView,
        value: *mut WuiBinding<Secure>,
    }
}

// ========== DatePicker ==========

/// C-compatible date representation using year, month (1-12), and day (1-31).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WuiDate {
    /// Year (e.g., 2024)
    pub year: i32,
    /// Month (1-12)
    pub month: u8,
    /// Day of month (1-31)
    pub day: u8,
}

/// C-compatible date-time representation with second precision.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WuiDateTime {
    /// Year (e.g., 2024)
    pub year: i32,
    /// Month (1-12)
    pub month: u8,
    /// Day of month (1-31)
    pub day: u8,
    /// Hour of day (0-23)
    pub hour: u8,
    /// Minute of hour (0-59)
    pub minute: u8,
    /// Second of minute (0-59)
    pub second: u8,
}

impl IntoFFI for Date {
    type FFI = WuiDate;
    fn into_ffi(self) -> Self::FFI {
        WuiDate {
            year: i32::from(self.year()),
            month: self.month().try_into().expect("jiff month must fit in u8"),
            day: self.day().try_into().expect("jiff day must fit in u8"),
        }
    }
}

impl IntoFFI for DateTime {
    type FFI = WuiDateTime;
    fn into_ffi(self) -> Self::FFI {
        WuiDateTime {
            year: i32::from(self.year()),
            month: self.month().try_into().expect("jiff month must fit in u8"),
            day: self.day().try_into().expect("jiff day must fit in u8"),
            hour: self.hour().try_into().expect("jiff hour must fit in u8"),
            minute: self
                .minute()
                .try_into()
                .expect("jiff minute must fit in u8"),
            second: self
                .second()
                .try_into()
                .expect("jiff second must fit in u8"),
        }
    }
}

impl crate::IntoRust for WuiDate {
    type Rust = Date;
    unsafe fn into_rust(self) -> Self::Rust {
        let year = i16::try_from(self.year)
            .expect("invalid year received from native DatePicker FFI bridge");
        Date::new(year, self.month as i8, self.day as i8).unwrap_or_else(|_| {
            panic!(
                "invalid date received from native DatePicker FFI bridge: year={}, month={}, day={}",
                self.year, self.month, self.day
            )
        })
    }
}

impl crate::IntoRust for WuiDateTime {
    type Rust = DateTime;
    unsafe fn into_rust(self) -> Self::Rust {
        let year = i16::try_from(self.year)
            .expect("invalid year received from native DatePicker FFI bridge");
        DateTime::new(
            year,
            self.month as i8,
            self.day as i8,
            self.hour as i8,
            self.minute as i8,
            self.second as i8,
            0,
        )
        .unwrap_or_else(|_| {
            panic!(
                "invalid date-time received from native DatePicker FFI bridge: year={}, month={}, day={}, hour={}, minute={}, second={}",
                self.year, self.month, self.day, self.hour, self.minute, self.second
            )
        })
    }
}

impl IntoFFI for core::ops::RangeInclusive<Date> {
    type FFI = WuiRange<WuiDate>;
    fn into_ffi(self) -> Self::FFI {
        WuiRange {
            start: (*self.start()).into_ffi(),
            end: (*self.end()).into_ffi(),
        }
    }
}

impl IntoFFI for core::ops::RangeInclusive<DateTime> {
    type FFI = WuiRange<WuiDateTime>;
    fn into_ffi(self) -> Self::FFI {
        WuiRange {
            start: (*self.start()).into_ffi(),
            end: (*self.end()).into_ffi(),
        }
    }
}

into_ffi! {DatePickerType, pub enum WuiDatePickerType {
    Date,
    HourAndMinute,
    HourMinuteAndSecond,
    DateHourAndMinute,
    DateHourMinuteAndSecond,
}}

into_ffi! {DatePickerConfig,
    pub struct WuiDatePicker {
        label: *mut WuiAnyView,
        value: *mut WuiBinding<DateTime>,
        range: WuiRange<WuiDateTime>,
        ty: WuiDatePickerType,
    }
}

ffi_view!(DatePickerConfig, WuiDatePicker, date_picker);
