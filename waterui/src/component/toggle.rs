use alloc::string::String;
use waterui_reactive::Binding;

use super::{AnyView, Text};
use crate::{Compute, View, ViewExt};

#[derive(Debug)]
#[non_exhaustive]
pub struct Toggle {
    pub _label: AnyView,
    pub _toggle: Binding<bool>,
    pub _style: ToggleStyle,
}

#[derive(Debug)]
#[repr(C)]
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
    pub fn new(label: impl Compute<Output = String>, toggle: &Binding<bool>) -> Self {
        Self {
            _label: Text::new(label).anyview(),
            _toggle: toggle.clone(),
            _style: ToggleStyle::default(),
        }
    }

    pub fn binding(toggle: &Binding<bool>) -> Self {
        Self::new("", toggle)
    }

    pub fn label(mut self, label: impl View + 'static) -> Self {
        self._label = label.anyview();
        self
    }
}

raw_view!(Toggle);

pub fn toggle(label: impl Compute<Output = String>, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(label, toggle)
}
