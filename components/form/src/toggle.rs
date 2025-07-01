use waterui_core::configurable;
use waterui_reactive::Binding;

use waterui_core::{AnyView, View};

#[derive(Debug)]
#[non_exhaustive]
pub struct ToggleConfig {
    pub label: AnyView,
    pub toggle: Binding<bool>,
}

configurable!(Toggle, ToggleConfig);

impl Toggle {
    pub fn new(toggle: &Binding<bool>) -> Self {
        Self(ToggleConfig {
            label: AnyView::default(),
            toggle: toggle.clone(),
        })
    }

    pub fn label(mut self, view: impl View) -> Self {
        self.0.label = AnyView::new(view);
        self
    }
}

pub fn toggle(label: impl View, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(toggle).label(label)
}
