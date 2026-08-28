//! A boolean toggle switch backed by a reactive binding.
//!
//! ![Toggle](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/toggle.svg)

use nami::Binding;
use waterui_core::{Environment, configurable};

use crate::label::{IntoLabel, Label, impl_label_style_methods};

/// Visual style options for toggle controls.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToggleStyle {
    /// The default toggle style, determined by the platform.
    #[default]
    Automatic,
    /// A switch-style toggle (sliding pill).
    Switch,
    /// A checkbox-style toggle (square with checkmark).
    Checkbox,
}

#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `Toggle` component.
pub struct ToggleConfig {
    /// The label displayed for the toggle.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    pub label: Label,
    /// The binding to the toggle state.
    pub toggle: Binding<bool>,
    /// The visual style of the toggle.
    pub style: ToggleStyle,
}

configurable!(
    /// A control that toggles between on and off states.
    ///
    /// Toggle displays a switch with a label. It's commonly used
    /// for settings that can be turned on or off.
    ///
    /// The label is required at construction; see
    /// [the label module](crate::label) for why. Use
    /// [`Self::hide_label`] when the surrounding context already explains the
    /// control — the label stays in the accessibility tree.
    ///
    /// # Layout Behavior
    ///
    /// With a visible label: Toggle expands horizontally to fill available
    /// space, placing the label on the left and switch on the right.
    /// With a hidden label: Toggle is content-sized (just the switch).
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::{Toggle, toggle};
    /// # fn labelled(is_enabled: Binding<bool>) -> impl View {
    /// // Simple toggle with label
    /// toggle("Wi-Fi", &is_enabled)
    /// # }
    ///
    /// # fn hidden_label(dark_mode: Binding<bool>) -> impl View {
    /// // Toggle whose label is announced but not drawn
    /// Toggle::new("Dark mode", &dark_mode).hide_label()
    /// # }
    ///
    /// # fn settings(notifications: Binding<bool>, sound: Binding<bool>) -> impl View {
    /// // In a settings list
    /// vstack((
    ///     toggle("Notifications", &notifications),
    ///     toggle("Sound", &sound),
    /// ))
    /// # }
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
    waterui_core::layout::StretchAxis::Horizontal,
    resolve |config, env| config.resolve(env)
);

impl ToggleConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        self
    }
}

impl Toggle {
    #[must_use]
    /// Creates a new `Toggle` with the specified label and binding for the
    /// toggle state.
    ///
    /// The label is mandatory. To keep it out of the visual chrome without
    /// losing its accessibility name, chain [`Self::hide_label`].
    pub fn new(label: impl IntoLabel, toggle: &Binding<bool>) -> Self {
        Self(ToggleConfig {
            label: label.into_label(),
            toggle: toggle.clone(),
            style: ToggleStyle::default(),
        })
    }
    #[must_use]
    /// Sets the visual style of the toggle.
    pub const fn style(mut self, style: ToggleStyle) -> Self {
        self.0.style = style;
        self
    }

    /// Changes the toggle to switch style.
    #[must_use]
    pub const fn switch(self) -> Self {
        self.style(ToggleStyle::Switch)
    }

    /// Changes the toggle to checkbox style.
    #[must_use]
    pub const fn checkbox(self) -> Self {
        self.style(ToggleStyle::Checkbox)
    }
}

impl_label_style_methods!(Toggle);

/// Creates a new `Toggle` with the specified label and binding for the toggle state.
#[must_use]
pub fn toggle(label: impl IntoLabel, toggle: &Binding<bool>) -> Toggle {
    Toggle::new(label, toggle)
}
