use alloc::string::String;
use waterui_reactive::Binding;

use super::{AnyView, Text};
use crate::{Compute, View, ViewExt};

#[derive(Debug)]
pub struct Toggle<Label> {
    label: Label,
    toggle: Binding<bool>,
    style: ToggleStyle,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct RawToggle {
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
impl Toggle<Text> {
    pub fn new(label: impl Compute<Output = String>, toggle: &Binding<bool>) -> Self {
        Self::label(Text::new(label), toggle)
    }
}

impl_label!(Toggle);

impl Toggle<()> {
    pub fn binding(toggle: &Binding<bool>) -> Self {
        Self::label((), toggle)
    }
}

impl<Label: View + 'static> Toggle<Label> {
    pub fn label(label: Label, toggle: &Binding<bool>) -> Self {
        Self {
            label,
            toggle: toggle.clone(),
            style: ToggleStyle::default(),
        }
    }
}

raw_view!(RawToggle);

impl<Label: View + 'static> View for Toggle<Label> {
    fn body(self, _env: crate::Environment) -> impl View {
        RawToggle {
            _label: self.label.anyview(),
            _toggle: self.toggle,
            _style: self.style,
        }
    }
}
