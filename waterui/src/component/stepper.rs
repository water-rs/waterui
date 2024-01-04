use crate::{view::IntoView, Binding, ViewExt};

use super::{text, AnyView};

#[derive(Debug)]
pub struct Stepper {
    pub(crate) text: AnyView,
    pub(crate) value: Binding<i64>,
    pub(crate) step: u64,
}

impl Stepper {
    pub fn new(value: &Binding<i64>) -> Self {
        Self {
            text: text(value.display()).anyview(),
            value: value.clone(),
            step: 1,
        }
    }

    pub fn text(mut self, text: impl IntoView) -> Self {
        self.text = text.into_anyview();
        self
    }

    pub fn step(mut self, step: u64) -> Self {
        self.step = step;
        self
    }
}

raw_view!(Stepper);

pub fn stepper(value: &Binding<i64>) -> Stepper {
    Stepper::new(value)
}
