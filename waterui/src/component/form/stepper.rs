use waterui_core::AnyView;
use waterui_reactive::{compute::ToComputed, Binding, Computed};

#[derive(Debug)]
#[non_exhaustive]
pub struct StepperConfig {
    pub value: Binding<i32>,
    pub step: Computed<i32>,
    pub label: AnyView,
}

configurable!(Stepper, StepperConfig);

impl Stepper {
    pub fn new(value: &Binding<i32>) -> Self {
        Self(StepperConfig {
            value: value.clone(),
            step: 1i32.to_computed(),
            label: AnyView::default(),
        })
    }

    pub fn step(mut self, step: impl ToComputed<i32>) -> Self {
        self.0.step = step.to_computed();
        self
    }
}

pub fn stepper(value: &Binding<i32>) -> Stepper {
    Stepper::new(value)
}
