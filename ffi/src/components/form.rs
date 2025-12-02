use crate::WuiAnyView;
use crate::components::text::WuiText;
use crate::id::WuiId;
use crate::reactive::{WuiBinding, WuiComputed};
use alloc::vec::Vec;
use waterui::prelude::{Slider, Stepper, TextField, Toggle};
use waterui::{
    Color, Str,
    component::{
        slider::SliderConfig,
        stepper::StepperConfig,
        text_field::{KeyboardType, TextFieldConfig},
        toggle::ToggleConfig,
    },
};
use waterui_core::id::Id;
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::{ColorPicker, Picker, PickerConfig, PickerItem};

into_ffi! {KeyboardType, Text, pub enum WuiKeyboardType {
    Text,
    Secure,
    Email,
    URL,
    Number,
    PhoneNumber
}}

into_ffi! {TextFieldConfig,
    pub struct WuiTextField {
        label: *mut WuiAnyView,
        value: *mut WuiBinding<Str>,
        prompt: WuiText,
        keyboard: WuiKeyboardType,
    }
}

into_ffi! {ToggleConfig,
    pub struct WuiToggle {
        label: *mut WuiAnyView,
        toggle: *mut WuiBinding<bool>,
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
ffi_view!(TextFieldConfig, WuiTextField, text_field);

ffi_view!(ToggleConfig, WuiToggle, toggle);

ffi_view!(SliderConfig, WuiSlider, slider);

ffi_view!(StepperConfig, WuiStepper, stepper);

ffi_view!(ColorPickerConfig, WuiColorPicker, color_picker);

ffi_view!(PickerConfig, WuiPicker, picker);

into_ffi! {PickerConfig,
    pub struct WuiPicker {
        items: *mut WuiComputed<Vec<PickerItem<Id>>>,
        selection: *mut WuiBinding<Id>,
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
    }
}
