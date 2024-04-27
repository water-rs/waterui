use crate::{binding::waterui_binding_int, computed::waterui_computed_int, ffi_view, IntoFFI};
use waterui::component::stepper::Stepper;
#[repr(C)]
pub struct waterui_stepper {
    value: *const waterui_binding_int,
    step: *mut waterui_computed_int,
}

impl IntoFFI for Stepper {
    type FFI = waterui_stepper;
    fn into_ffi(self) -> Self::FFI {
        waterui_stepper {
            value: self._value.into_ffi(),
            step: self._step.into_ffi(),
        }
    }
}

ffi_view!(
    Stepper,
    waterui_stepper,
    waterui_view_force_as_stepper,
    waterui_view_stepper_id
);
