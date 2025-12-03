//! A boolean toggle switch backed by a reactive binding.
//!
//! ![Toggle](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/toggle.svg)

use nami::Binding;
use waterui_core::configurable;

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

configurable!(
    /// A control that toggles between on and off states.
    ///
    /// Toggle displays a switch with an optional label. It's commonly used
    /// for settings that can be turned on or off.
    ///
    /// # Layout Behavior
    ///
    /// With a label: Toggle expands horizontally to fill available space,
    /// placing the label on the left and switch on the right.
    /// Without a label: Toggle is content-sized (just the switch).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Simple toggle with label
    /// toggle("Wi-Fi", &is_enabled)
    ///
    /// // Toggle without label
    /// Toggle::new(&dark_mode)
    ///
    /// // In a settings list
    /// vstack((
    ///     toggle("Notifications", &notifications),
    ///     toggle("Sound", &sound),
    /// ))
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // - stretchAxis: .horizontal (toggle expands to fill available width)
    // - sizeThatFits: Returns proposed width (or minimum), intrinsic height
    // - Layout: label on left, switch on right, flexible space between
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Toggle,
    ToggleConfig,
    waterui_core::layout::StretchAxis::Horizontal
);

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
