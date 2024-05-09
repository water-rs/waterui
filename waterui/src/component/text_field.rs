use waterui_core::raw_view;
use waterui_reactive::{compute::IntoComputed, Binding, Computed};

use crate::{AnyView, CowStr, View};

use super::Text;

#[non_exhaustive]
#[derive(Debug)]
pub struct TextField {
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

raw_view!(TextField);

impl TextField {
    pub fn new(label: impl IntoComputed<CowStr>, value: &Binding<CowStr>) -> Self {
        Self::label(Text::new(label), value)
    }

    pub fn label(label: impl View, value: &Binding<CowStr>) -> Self {
        TextField {
            _label: AnyView::new(label),
            _value: value.clone(),
            _prompt: Computed::constant("".into()),
            _style: TextFieldStyle::default(),
        }
    }

    pub fn with_prompt(mut self, prompt: impl IntoComputed<CowStr>) -> Self {
        self._prompt = prompt.into_computed();
        self
    }
}

pub fn field(label: impl IntoComputed<CowStr>, value: &Binding<CowStr>) -> TextField {
    TextField::new(label, value)
}
