use waterui_core::raw_view;
use waterui_reactive::{compute::ToComputed, Binding};
use waterui_str::Str;

use crate::{AnyView, View};

use super::Text;

#[derive(Debug)]
#[non_exhaustive]

pub struct Toggle {
    pub _label: AnyView,
    pub _toggle: Binding<bool>,
    pub _style: ToggleStyle,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ToggleStyle {
    Default,
    CheckBox,
    Switch,
}

impl Default for ToggleStyle {
    fn default() -> Self {
        Self::Default
    }
}

impl Toggle {
    pub fn new(label: impl ToComputed<Str>, toggle: &Binding<bool>) -> Self {
        Self::label(Text::new(label), toggle)
    }

    pub fn label(label: impl View, toggle: &Binding<bool>) -> Self {
        Self {
            _label: AnyView::new(label),
            _toggle: toggle.clone(),
            _style: ToggleStyle::default(),
        }
    }

    pub fn with_label(mut self, label: impl View) -> Self {
        self._label = AnyView::new(label);
        self
    }
}

raw_view!(Toggle);

pub fn toggle(label: impl ToComputed<Str>, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(label, toggle)
}
