use waterui_reactive::{Compute, Computed};

use crate::Binding;

#[derive(Debug)]
#[non_exhaustive]
pub struct Stepper {
    pub _value: Binding<i64>,
    pub _step: Computed<u64>,
}

impl Stepper {
    pub fn new(value: &Binding<i64>) -> Self {
        Self {
            _value: value.clone(),
            _step: Computed::constant(1),
        }
    }

    pub fn step(mut self, step: impl Compute<Output = u64>) -> Self {
        self._step = step.computed();
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &Binding<i64>) -> Stepper {
    Stepper::new(value)
}
