use waterui_reactive::Binding;

use crate::view::IntoView;

use super::AnyView;

#[derive(Debug)]
pub struct Toggle {
    pub(crate) label: AnyView,
    pub(crate) toggle: Binding<bool>,
}

impl Toggle {
    pub fn new(label: impl IntoView, toggle: &Binding<bool>) -> Self {
        Self {
            label: label.into_anyview(),
            toggle: toggle.clone(),
        }
    }
}

raw_view!(Toggle);

pub fn toggle(label: impl IntoView, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(label, toggle)
}
