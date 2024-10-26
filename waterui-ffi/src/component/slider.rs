use core::ops::RangeInclusive;

use waterui::component::form::slider::SliderConfig;

use crate::{binding::waterui_binding_double, waterui_anyview, IntoFFI};

#[repr(C)]
pub struct waterui_slider {
    label: *mut waterui_anyview,
    min_value_label: *mut waterui_anyview,
    max_value_label: *mut waterui_anyview,
    range: waterui_range_inclusive<f64>,
    value: *mut waterui_binding_double,
}

#[repr(C)]
pub struct waterui_range_inclusive<T> {
    start: T,
    end: T,
}

impl<T: Clone + IntoFFI> IntoFFI for RangeInclusive<T> {
    type FFI = waterui_range_inclusive<T::FFI>;
    fn into_ffi(self) -> Self::FFI {
        Self::FFI {
            start: self.start().clone().into_ffi(),
            end: self.end().clone().into_ffi(),
        }
    }
}

into_ffi!(
    SliderConfig,
    waterui_slider,
    label,
    min_value_label,
    max_value_label,
    range,
    value
);

native_view!(
    SliderConfig,
    waterui_slider,
    waterui_view_force_as_slider,
    waterui_view_slider_id
);
