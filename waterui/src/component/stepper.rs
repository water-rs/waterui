use waterui_reactive::{compute::IntoComputed, Binding, Computed, Int};

#[derive(Debug)]
#[non_exhaustive]
pub struct Stepper {
    pub _value: Binding<Int>,
    pub _step: Computed<Int>,
}

impl Stepper {
    pub fn new(value: &Binding<Int>) -> Self {
        Self {
            _value: value.clone(),
            _step: 1.into_computed(),
        }
    }

    pub fn step(mut self, step: impl IntoComputed<Int>) -> Self {
        self._step = step.into_computed();
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &Binding<Int>) -> Stepper {
    Stepper::new(value)
}

mod ffi {
    use waterui_ffi::{
        binding::waterui_binding_int, computed::waterui_computed_int, ffi_view, IntoFFI,
    };

    #[repr(C)]
    pub struct Stepper {
        value: *const waterui_binding_int,
        step: *mut waterui_computed_int,
    }

    impl IntoFFI for super::Stepper {
        type FFI = Stepper;
        fn into_ffi(self) -> Self::FFI {
            Stepper {
                value: self._value.into_ffi(),
                step: self._step.into_ffi(),
            }
        }
    }

    ffi_view!(
        super::Stepper,
        Stepper,
        waterui_view_force_as_stepper,
        waterui_view_stepper_id
    );
}
