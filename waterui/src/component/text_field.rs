use waterui_core::raw_view;
use waterui_reactive::{compute::ToComputed, Binding, Computed};
use waterui_str::Str;

use crate::{AnyView, View};

use super::Text;

#[non_exhaustive]
#[derive(Debug)]
pub struct TextField {
    pub _label: AnyView,
    pub _value: Binding<Str>,
    pub _prompt: Computed<Str>,
    pub _style: TextFieldStyle,
}

pub enum KeyboardType {
    Default,
    Text,
    Email,
    URL,
    Number,
    PhoneNumber,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum TextFieldStyle {
    Default,
    Plain,
    Outlined,
    Underlined,
}

impl Default for TextFieldStyle {
    fn default() -> Self {
        Self::Default
    }
}

raw_view!(TextField);

impl TextField {
    pub fn new(label: impl ToComputed<Str>, value: &Binding<Str>) -> Self {
        Self::label(Text::new(label), value)
    }

    pub fn label(label: impl View, value: &Binding<Str>) -> Self {
        TextField {
            _label: AnyView::new(label),
            _value: value.clone(),
            _prompt: "".to_computed(),
            _style: TextFieldStyle::default(),
        }
    }

    pub fn with_prompt(mut self, prompt: impl ToComputed<Str>) -> Self {
        self._prompt = prompt.to_computed();
        self
    }
}

pub fn field(label: impl ToComputed<Str>, value: &Binding<Str>) -> TextField {
    TextField::new(label, value)
}
