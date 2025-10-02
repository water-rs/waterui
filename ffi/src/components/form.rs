use crate::color::WuiColor;
use crate::components::text::WuiText;
use crate::{ffi_struct, ffi_view, impl_binding, WuiAnyView, WuiId};
use alloc::vec::Vec;
use waterui::component::Native;
use waterui::{Binding, Color, Computed, Str};
use waterui_core::id::{Id};
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::{PickerConfig, PickerItem};
use waterui_form::{
    slider::SliderConfig,
    stepper::StepperConfig,
    text_field::{KeyboardType, TextFieldConfig},
    toggle::ToggleConfig,
};

ffi_enum_with_default!(
    KeyboardType,
    WuiKeyboardType,
    Text,
    Secure,
    Email,
    URL,
    Number,
    PhoneNumber
);

/// C representation of a TextField configuration
#[repr(C)]
pub struct WuiTextField {
    /// Pointer to the text field's label view
    pub label: *mut WuiAnyView,
    /// Pointer to the text value binding
    pub value: *mut Binding<Str>,
    /// Pointer to the prompt text
    pub prompt: WuiText,
    /// The keyboard type to use
    pub keyboard: WuiKeyboardType,
}

/// C representation of a Toggle configuration
#[repr(C)]
pub struct WuiToggle {
    /// Pointer to the toggle's label view
    pub label: *mut WuiAnyView,
    /// Pointer to the toggle state binding
    pub toggle: *mut Binding<bool>,
}

/// C representation of a range
#[repr(C)]
pub struct WuiRange<T> {
    /// Start of the range
    pub start: T,
    /// End of the range
    pub end: T,
}

/// C representation of a Slider configuration
#[repr(C)]
pub struct WuiSlider {
    /// Pointer to the slider's label view
    pub label: *mut WuiAnyView,
    /// Pointer to the minimum value label view
    pub min_value_label: *mut WuiAnyView,
    /// Pointer to the maximum value label view
    pub max_value_label: *mut WuiAnyView,
    /// The range of values
    pub range: WuiRange<f64>,
    /// Pointer to the value binding
    pub value: *mut Binding<f64>,
}

/// C representation of a Stepper configuration
#[repr(C)]
pub struct WuiStepper {
    /// Pointer to the value binding
    pub value: *mut Binding<i32>,
    /// Pointer to the step size computed value
    pub step: *mut Computed<i32>,
    /// Pointer to the stepper's label view
    pub label: *mut WuiAnyView,
    /// The valid range of values
    pub range: WuiRange<i32>,
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

// Implement struct conversions
ffi_struct!(
    TextFieldConfig,
    WuiTextField,
    label,
    value,
    prompt,
    keyboard
);

ffi_struct!(ToggleConfig, WuiToggle, label, toggle);
ffi_struct!(
    SliderConfig,
    WuiSlider,
    label,
    min_value_label,
    max_value_label,
    range,
    value
);
ffi_struct!(StepperConfig, WuiStepper, value, step, label, range);

// FFI view bindings for form components
ffi_view!(
    Native<TextFieldConfig>,
    WuiTextField,
    waterui_text_field_id,
    waterui_force_as_text_field
);

ffi_view!(
    Native<ToggleConfig>,
    WuiToggle,
    waterui_toggle_id,
    waterui_force_as_toggle
);

ffi_view!(
    Native<SliderConfig>,
    WuiSlider,
    waterui_slider_id,
    waterui_force_as_slider
);

ffi_view!(
    Native<StepperConfig>,
    WuiStepper,
    waterui_stepper_id,
    waterui_force_as_stepper
);

ffi_view!(
    Native<ColorPickerConfig>,
    WuiColorPicker,
    waterui_color_picker_id,
    waterui_force_as_color_picker
);

ffi_view!(Native<PickerConfig>,WuiPicker,waterui_picker_id,waterui_force_as_picker);

/*

  /// The items to display in the picker.
    pub items: Computed<Vec<PickerItem<Id>>>,
    /// The binding to the currently selected item.
    pub selection: Binding<Id>,

*/

#[repr(C)]
pub struct WuiPicker{
    items:*mut Computed<Vec<PickerItem<Id>>>,
    selection:*mut Binding<Id>,
}


#[repr(C)]
pub struct WuiPickerItem{
    tag:WuiId,
    content:WuiText,
}

ffi_struct!(PickerItem<Id>,WuiPickerItem,tag,content);

ffi_struct!(PickerConfig,WuiPicker,items,selection);


#[repr(C)]
pub struct WuiColorPicker {
    pub label: *mut WuiAnyView,
    pub value: *mut Binding<Color>,
}

ffi_struct!(ColorPickerConfig, WuiColorPicker, label, value);

impl_binding!(
    Color,
    WuiColor,
    waterui_binding_read_color,
    waterui_binding_set_color,
    waterui_watch_color,
    waterui_drop_color_watcher_guard
);
