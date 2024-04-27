use waterui_reactive::{compute::IntoComputed, Binding, CowStr};

use crate::{AnyView, View, ViewExt};

use super::Text;

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

impl_label!(Toggle);

impl Toggle<()> {
    pub fn new(toggle: &Binding<bool>) -> Self {
        Self {
            label: (),
            toggle: toggle.clone(),
            style: ToggleStyle::Default,
        }
    }
}

impl<Label: View> Toggle<Label> {
    pub fn label(self, label: impl IntoComputed<CowStr>) -> Toggle<Text> {
        self.label_view(Text::new(label))
    }
    pub fn label_view<V: View>(self, label: V) -> Toggle<V> {
        Toggle {
            label,
            toggle: self.toggle,
            style: self.style,
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

pub fn toggle(label: impl IntoComputed<CowStr>, toggle: &Binding<bool>) -> Toggle<Text> {
    Toggle::new(toggle).label(label)
}
