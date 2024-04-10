use crate::{Binding, Compute, Computed};

#[derive(Debug)]
pub struct TextField {
    pub _label: Computed<String>,
    pub _value: Binding<String>,
    pub _prompt: Computed<String>,
}

raw_view!(TextField);

impl TextField {
    pub fn new(label: impl Compute<Output = String>, value: &Binding<String>) -> Self {
        Self {
            _label: label.computed(),
            _value: value.clone(),
            _prompt: String::new().computed(),
        }
    }

    pub fn binding(value: &Binding<String>) -> Self {
        Self::new("", value)
    }

    pub fn prompt(mut self, prompt: impl Compute<Output = String>) -> Self {
        self._prompt = prompt.computed();
        self
    }
}

pub fn field(label: impl Compute<Output = String>, value: &Binding<String>) -> TextField {
    TextField::new(label, value)
}
