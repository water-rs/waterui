use waterui_reactive::{compute::IntoComputed, Binding, Computed};

#[derive(Debug)]
#[non_exhaustive]
pub struct Stepper {
    pub _value: Binding<isize>,
    pub _step: Computed<isize>,
}

impl Stepper {
    pub fn new(value: &Binding<isize>) -> Self {
        Self {
            _value: value.clone(),
            _step: 1.into_computed(),
        }
    }

    pub fn step(mut self, step: impl IntoComputed<isize>) -> Self {
        self._step = step.into_computed();
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &Binding<isize>) -> Stepper {
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
