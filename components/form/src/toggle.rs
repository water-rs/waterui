use waterui_core::configurable;
use nami::Binding;

use waterui_core::{AnyView, View};

#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `Toggle` component.
pub struct ToggleConfig {
    /// The label to display for the toggle.
    pub label: AnyView,
    /// The binding to the toggle state.
    pub toggle: Binding<bool>,
}

configurable!(Toggle, ToggleConfig);

impl Toggle {
    #[must_use]
    /// Creates a new `Toggle` with the specified binding for the toggle state.
    pub fn new(toggle: &Binding<bool>) -> Self {
        Self(ToggleConfig {
            label: AnyView::default(),
            toggle: toggle.clone(),
        })
    }
    #[must_use]
    /// Sets the label for the toggle.
    pub fn label(mut self, view: impl View) -> Self {
        self.0.label = AnyView::new(view);
        self
    }
}

/// Creates a new `Toggle` with the specified label and binding for the toggle state.
#[must_use]
pub fn toggle(label: impl View, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(toggle).label(label)
}
