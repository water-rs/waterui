use waterui_reactive::{compute::IntoComputed, Binding, Computed};

use crate::{AnyView, CowStr, Environment, View, ViewExt};

use super::Text;

pub struct TextField<Label> {
    label: Label,
    value: Binding<CowStr>,
    prompt: Computed<CowStr>,
    style: TextFieldStyle,
}

#[non_exhaustive]
pub struct RawTextField {
    pub _label: AnyView,
    pub _value: Binding<CowStr>,
    pub _prompt: Computed<CowStr>,
    pub _style: TextFieldStyle,
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

raw_view!(RawTextField);

impl TextField<()> {
    pub fn new(value: &Binding<CowStr>) -> Self {
        Self {
            label: (),
            value: value.clone(),
            prompt: "".into_computed(),
            style: TextFieldStyle::default(),
        }
    }
}

impl<Label> TextField<Label> {
    pub fn label(self, label: impl IntoComputed<CowStr>) -> TextField<Text> {
        self.label_view(Text::new(label))
    }

    pub fn label_view<V: 'static>(self, label: V) -> TextField<V> {
        TextField {
            label,
            value: self.value,
            prompt: self.prompt,
            style: self.style,
        }
    }
}

impl_label!(TextField);

impl<V: View> TextField<V> {
    pub fn prompt(mut self, prompt: impl IntoComputed<CowStr>) -> Self {
        self.prompt = prompt.into_computed();
        self
    }
}

impl<V: View + 'static> View for TextField<V> {
    fn body(self, _env: Environment) -> impl View {
        RawTextField {
            _label: self.label.anyview(),
            _value: self.value,
            _prompt: self.prompt,
            _style: self.style,
        }
    }
}

pub fn field(label: impl IntoComputed<CowStr>, value: &Binding<CowStr>) -> TextField<Text> {
    TextField::new(value).label(label)
}
