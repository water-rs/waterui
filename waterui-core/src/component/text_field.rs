use crate::{reactive::IntoReactive, AttributedString, Reactive};

pub struct TextField {
    pub(crate) label: Reactive<AttributedString>,
    pub(crate) value: Reactive<String>,
    pub(crate) prompt: Reactive<String>,
}

raw_view!(TextField);

impl TextField {
    pub fn new(label: impl IntoReactive<AttributedString>, value: &Reactive<String>) -> Self {
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
