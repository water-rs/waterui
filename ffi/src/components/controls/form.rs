use crate::components::text::WuiText;
use crate::id::WuiId;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{AnyView, WuiAnyView, WuiLabel, WuiStr};
use alloc::vec::Vec;
use jiff::civil::{Date, DateTime};
use waterui::text::styled::StyledStr;
use waterui::{
    Color, Str,
    component::{
        slider::SliderConfig,
        stepper::StepperConfig,
        text_field::{KeyboardType, ResolvedTextFieldConfig},
        toggle::{ToggleConfig, ToggleStyle},
    },
};
use waterui_core::{Native, id::Id};
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::date::{DatePickerConfig, DatePickerType};
use waterui_form::picker::multi_date::MultiDatePickerConfig;
use waterui_form::picker::{PickerConfig, PickerItem, PickerStyle};
use waterui_form::secure::{Secure, SecureFieldConfig};

into_ffi! {KeyboardType, non_exhaustive, pub enum WuiKeyboardType {
    Text,
    Email,
    URL,
    Number,
    PhoneNumber
}}

/// FFI representation of the `TextField` component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiTextField {
    /// Semantic label slot for the field — see [`WuiLabel`].
    pub label: WuiLabel,
    /// Reactive binding to the field's styled text content.
    pub value: *mut WuiBinding<StyledStr>,
    /// Placeholder text shown when the field is empty.
    pub prompt: WuiText,
    /// The on-screen keyboard variant to present while editing.
    pub keyboard: WuiKeyboardType,
    /// Context menu items offered when the user selects text in the field.
    pub selection_menu: *mut crate::views::WuiAnyViews,
    /// Maximum number of lines the field accepts.
    ///
    /// `1` is a single-line field; a larger value caps a multi-line field; `0`
    /// means the field has no line limit. Backends must reject input that would
    /// exceed the limit rather than truncating the existing value.
    pub line_limit: usize,
}

impl IntoFFI for ResolvedTextFieldConfig {
    type FFI = WuiTextField;

    fn into_ffi(self) -> Self::FFI {
        WuiTextField {
            label: self.label.into_ffi(),
            value: self.value.into_ffi(),
            prompt: self.prompt.into_ffi(),
            keyboard: self.keyboard.into_ffi(),
            selection_menu: crate::menu_items_views(self.selection_menu),
            line_limit: self.line_limit.map_or(0, core::num::NonZeroUsize::get),
        }
    }
}

into_ffi! {ToggleStyle, non_exhaustive, pub enum WuiToggleStyle {
    Automatic,
    Switch,
    Checkbox,
}}

into_ffi! {ToggleConfig,
    pub struct WuiToggle {
        label: WuiLabel,
        toggle: *mut WuiBinding<bool>,
        style: WuiToggleStyle,
    }
}

/// C representation of a range
#[repr(C)]
#[derive(Debug)]
pub struct WuiRange<T> {
    /// Start of the range
    pub start: T,
    /// End of the range
    pub end: T,
}

into_ffi! {SliderConfig,
    pub struct WuiSlider {
        label: WuiLabel,
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
        label: WuiLabel,
        value_formatter: *mut WuiComputed<StyledStr>,
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

into_ffi! {PickerStyle, non_exhaustive, pub enum WuiPickerStyle {
    Automatic,
    Menu,
    Radio,
    Segmented,
}}

/// FFI representation of the `Picker` component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiPicker {
    /// Semantic label slot for the picker — see [`WuiLabel`].
    pub label: WuiLabel,
    /// The picker's items, rendered as an identity-tracked view collection.
    pub items: *mut crate::views::WuiAnyViews,
    /// Reactive binding to the tag of the currently selected item.
    pub selection: *mut WuiBinding<Id>,
    /// The visual presentation style for the picker.
    pub style: WuiPickerStyle,
}

impl IntoFFI for PickerConfig {
    type FFI = WuiPicker;

    fn into_ffi(self) -> Self::FFI {
        WuiPicker {
            label: self.label.into_ffi(),
            items: crate::views::signal_vec_views(
                self.items,
                |items, index| items[index].tag,
                |item| AnyView::new(Native::new(ResolvedPickerItem(item))),
            ),
            selection: self.selection.into_ffi(),
            style: self.style.into_ffi(),
        }
    }
}

/// Raw semantic picker item stored in the framework's identity-aware collection.
#[doc(hidden)]
#[derive(Debug)]
pub struct ResolvedPickerItem(PickerItem<Id>);

waterui_core::raw_view!(ResolvedPickerItem);

/// FFI representation of a single resolved picker item.
#[repr(C)]
#[derive(Debug)]
pub struct WuiPickerItem {
    /// The stable identity tag used to track selection for this item.
    pub tag: WuiId,
    /// Reactive computed label text displayed for this item.
    pub label: *mut WuiComputed<StyledStr>,
}

impl IntoFFI for ResolvedPickerItem {
    type FFI = WuiPickerItem;

    fn into_ffi(self) -> Self::FFI {
        WuiPickerItem {
            tag: self.0.tag.into_ffi(),
            label: self.0.content.content().into_ffi(),
        }
    }
}

ffi_view!(ResolvedPickerItem, WuiPickerItem, picker_item);

into_ffi! {ColorPickerConfig,
    pub struct WuiColorPicker {
        label: WuiLabel,
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
        label: WuiLabel,
        value: *mut WuiBinding<Secure>,
    }
}

// ========== DatePicker ==========

/// C-compatible date representation using year, month (1-12), and day (1-31).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
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
        let month = i8::try_from(self.month)
            .expect("invalid month received from native DatePicker FFI bridge");
        let day =
            i8::try_from(self.day).expect("invalid day received from native DatePicker FFI bridge");
        Date::new(year, month, day).unwrap_or_else(|_| {
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
        let month = i8::try_from(self.month)
            .expect("invalid month received from native DatePicker FFI bridge");
        let day =
            i8::try_from(self.day).expect("invalid day received from native DatePicker FFI bridge");
        let hour = i8::try_from(self.hour)
            .expect("invalid hour received from native DatePicker FFI bridge");
        let minute = i8::try_from(self.minute)
            .expect("invalid minute received from native DatePicker FFI bridge");
        let second = i8::try_from(self.second)
            .expect("invalid second received from native DatePicker FFI bridge");
        DateTime::new(year, month, day, hour, minute, second, 0).unwrap_or_else(|_| {
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
        label: WuiLabel,
        value: *mut WuiBinding<DateTime>,
        range: WuiRange<WuiDateTime>,
        ty: WuiDatePickerType,
    }
}

ffi_view!(DatePickerConfig, WuiDatePicker, date_picker);

into_ffi! {MultiDatePickerConfig,
    pub struct WuiMultiDatePicker {
        label: WuiLabel,
        value: *mut WuiBinding<Vec<Date>>,
        range: WuiRange<WuiDate>,
        decorated: *mut WuiComputed<Vec<Date>>,
    }
}

ffi_view!(MultiDatePickerConfig, WuiMultiDatePicker, multi_date_picker);
