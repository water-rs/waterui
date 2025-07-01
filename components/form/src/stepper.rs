use waterui_core::{AnyView, View, configurable};
use waterui_reactive::{Binding, Computed, compute::IntoComputed};

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
            step: 1i32.into_computed(),
            label: AnyView::default(),
        })
    }

    pub fn step(mut self, step: impl IntoComputed<i32>) -> Self {
        self.0.step = step.into_computed();
        self
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }
}

pub fn stepper(value: &Binding<i32>) -> Stepper {
    Stepper::new(value)
}
