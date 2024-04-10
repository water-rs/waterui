use crate::Binding;

#[derive(Debug)]
pub struct Stepper {
    pub _value: Binding<i64>,
    pub _step: u64,
}

impl Stepper {
    pub fn new(value: &Binding<i64>) -> Self {
        Self {
            _value: value.clone(),
            _step: 1,
        }
    }

    pub fn step(mut self, step: u64) -> Self {
        self._step = step;
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &Binding<i64>) -> Stepper {
    Stepper::new(value)
}
