use crate::view::ViewExt;
use crate::{AnyView, View};
use waterui_reactive::Binding;
use waterui_str::Str;

use super::Text;

configurable!(TextField, TextFieldConfig);
#[non_exhaustive]
#[derive(Debug)]
pub struct TextFieldConfig {
    pub label: AnyView,
    pub value: Binding<Str>,
    pub prompt: Text,
    pub keyboard: KeyboardType,
}

#[derive(Debug, Default)]
#[non_exhaustive]
pub enum KeyboardType {
    #[default]
    Text,
    Email,
    URL,
    Number,
    PhoneNumber,
}

impl TextField {
    pub fn new(label: impl View, value: &Binding<Str>) -> Self {
        Self(TextFieldConfig {
            label: label.anyview(),
            value: value.clone(),
            prompt: Text::default(),
            keyboard: KeyboardType::default(),
        })
    }

    pub fn prompt(mut self, prompt: impl Into<Text>) -> Self {
        self.0.prompt = prompt.into();
        self
    }
}

pub fn field(label: impl View, value: &Binding<Str>) -> TextField {
    TextField::new(label, value)
}
