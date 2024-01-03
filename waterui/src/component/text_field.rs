use waterui_reactive::{binding::Binding, reactive::IntoReactive};

use crate::Reactive;

pub struct TextField {
    pub(crate) label: Reactive<String>,
    pub(crate) value: Binding<String>,
    pub(crate) prompt: Reactive<String>,
}

raw_view!(TextField);

impl TextField {
    pub fn new(label: impl IntoReactive<String>, value: &Binding<String>) -> Self {
        Self {
            label: label.into_reactive(),
            value: value.clone(),
            prompt: Reactive::default(),
        }
    }

    pub fn prompt(mut self, prompt: impl IntoReactive<String>) -> Self {
        self.prompt = prompt.into_reactive();
        self
    }
}

pub fn field(label: impl IntoReactive<String>, value: &Binding<String>) -> TextField {
    TextField::new(label, value)
}
