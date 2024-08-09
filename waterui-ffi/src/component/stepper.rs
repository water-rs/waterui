use crate::{binding::waterui_binding_int, computed::waterui_computed_int, IntoFFI};
use waterui::component::stepper::StepperConfig;
#[repr(C)]
pub struct waterui_stepper {
    value: *const waterui_binding_int,
    step: *mut waterui_computed_int,
}

impl IntoFFI for StepperConfig {
    type FFI = waterui_stepper;
    fn into_ffi(self) -> Self::FFI {
        waterui_stepper {
            value: self.value.into_ffi(),
            step: self.step.into_ffi(),
        }
    }
}

native_view!(
    StepperConfig,
    waterui_stepper,
    waterui_view_force_as_stepper,
    waterui_view_stepper_id
);
