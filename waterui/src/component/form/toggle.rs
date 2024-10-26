use waterui_reactive::Binding;

use crate::{AnyView, View};

#[derive(Debug)]
#[non_exhaustive]
pub struct ToggleConfig {
    pub label: AnyView,
    pub toggle: Binding<bool>,
}

configurable!(Toggle, ToggleConfig);

impl Toggle {
    pub fn new(label: impl View, toggle: &Binding<bool>) -> Self {
        Self(ToggleConfig {
            label: AnyView::new(label),
            toggle: toggle.clone(),
        })
    }
}

pub fn toggle(label: impl View, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(label, toggle)
}
