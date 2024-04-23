use waterui_reactive::{binding::BindingInt, compute::ComputedInt, Compute};

#[derive(Debug)]
#[non_exhaustive]
pub struct Stepper {
    pub _value: BindingInt,
    pub _step: ComputedInt,
}

impl Stepper {
    pub fn new(value: &BindingInt) -> Self {
        Self {
            _value: value.clone(),
            _step: 1.computed(),
        }
    }

    pub fn step(mut self, step: impl Compute<Output = isize>) -> Self {
        self._step = step.computed();
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &BindingInt) -> Stepper {
    Stepper::new(value)
}

mod ffi {
    use waterui_ffi::{binding::BindingInt, computed::ComputedInt, ffi_view, IntoFFI};

    #[repr(C)]
    pub struct Stepper {
        value: BindingInt,
        step: ComputedInt,
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
