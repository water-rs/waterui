use waterui_core::Str;
use waterui_core::configurable;
use waterui_core::{AnyView, View};
use waterui_reactive::Binding;

use waterui_text::Text;

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
        self.0.label = AnyView::new(label);
        self
    }

    pub fn prompt(mut self, prompt: impl Into<Text>) -> Self {
        self.0.prompt = prompt.into();
        self
    }
}

pub fn field(label: impl View, value: &Binding<Str>) -> TextField {
    TextField::new(value).label(label)
}
