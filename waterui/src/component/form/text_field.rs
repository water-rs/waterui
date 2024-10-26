use crate::view::ViewExt;
use crate::{AnyView, View};
use waterui_reactive::Binding;
use waterui_str::Str;

use crate::component::Text;

configurable!(TextField, TextFieldConfig);
configurable!(SecureField, TextFieldConfig);

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
    pub fn new(value: &Binding<Str>) -> Self {
        Self(TextFieldConfig {
            label: AnyView::default(),
            value: value.clone(),
            prompt: Text::default(),
            keyboard: KeyboardType::default(),
        })
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = label.anyview();
        self
    }

    pub fn prompt(mut self, prompt: impl Into<Text>) -> Self {
        self.0.prompt = prompt.into();
        self
    }
}

pub fn field(value: &Binding<Str>) -> TextField {
    TextField::new(value)
}
