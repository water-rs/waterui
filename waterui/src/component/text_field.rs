use alloc::string::String;

use crate::{Binding, Compute, Computed, View, ViewExt};

use super::{AnyView, Text};

#[derive(Debug)]
pub struct TextField<Label> {
    label: Label,
    value: Binding<String>,
    prompt: Computed<String>,
    style: TextFieldStyle,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct RawTextField {
    pub _label: AnyView,
    pub _value: Binding<String>,
    pub _prompt: Computed<String>,
    pub _style: TextFieldStyle,
}

#[derive(Debug)]
#[repr(C)]
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

raw_view!(RawTextField);

impl TextField<()> {
    pub fn binding(value: &Binding<String>) -> Self {
        Self::label((), value)
    }
}

impl TextField<Text> {
    pub fn new(label: impl Compute<Output = String>, value: &Binding<String>) -> Self {
        Self::label(Text::new(label), value)
    }
}

impl_label!(TextField);

impl<V: View + 'static> TextField<V> {
    pub fn label(label: V, value: &Binding<String>) -> Self {
        Self {
            label,
            value: value.clone(),
            prompt: String::new().computed(),
            style: TextFieldStyle::default(),
        }
    }

    pub fn prompt(mut self, prompt: impl Compute<Output = String>) -> Self {
        self.prompt = prompt.computed();
        self
    }
}

impl<V: View + 'static> View for TextField<V> {
    fn body(self, _env: crate::Environment) -> impl View {
        RawTextField {
            _label: self.label.anyview(),
            _value: self.value,
            _prompt: self.prompt,
            _style: self.style,
        }
    }
}
