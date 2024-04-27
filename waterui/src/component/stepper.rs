use waterui_reactive::{compute::IntoComputed, Binding, Computed};

#[derive(Debug)]
#[non_exhaustive]
pub struct Stepper {
    pub _value: Binding<i32>,
    pub _step: Computed<i32>,
}

impl Stepper {
    pub fn new(value: &Binding<i32>) -> Self {
        Self {
            _value: value.clone(),
            _step: 1.into_computed(),
        }
    }

    pub fn step(mut self, step: impl IntoComputed<i32>) -> Self {
        self._step = step.into_computed();
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &Binding<i32>) -> Stepper {
    Stepper::new(value)
}
