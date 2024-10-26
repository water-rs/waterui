use crate::{binding::waterui_binding_int, computed::waterui_computed_int};
use waterui::component::form::stepper::StepperConfig;
#[repr(C)]
pub struct waterui_stepper {
    value: *const waterui_binding_int,
    step: *mut waterui_computed_int,
}

into_ffi!(StepperConfig, waterui_stepper, value, step);

native_view!(
    StepperConfig,
    waterui_stepper,
    waterui_view_force_as_stepper,
    waterui_view_stepper_id
);
