use crate::{AttributedString, Binding};

pub struct TextField {
    pub(crate) label: AttributedString,
    pub(crate) value: Binding<String>,
    pub(crate) prompt: String,
}

raw_view!(TextField);

impl TextField {
    pub fn new(label: impl Into<AttributedString>, value: Binding<String>) -> Self {
        Self {
            label: label.into(),
            value,
            prompt: String::new(),
        }
    }

    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }
}
