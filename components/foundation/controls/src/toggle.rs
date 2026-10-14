//! A boolean toggle switch backed by a reactive binding.
//!
//! ![Toggle](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/toggle.svg)

use nami::Binding;
use waterui_core::{Environment, configurable, layout::StretchAxis};

use crate::label::{IntoLabel, Label, LabelDisplayMode, impl_label_style_methods};

/// Visual style options for toggle controls.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToggleStyle {
    /// The platform's default style: [`Self::Switch`] on iOS and Android,
    /// [`Self::Checkbox`] on macOS. See [`Self::resolved`].
    #[default]
    Automatic,
    /// A switch-style toggle (sliding pill).
    Switch,
    /// A checkbox-style toggle (square with checkmark).
    Checkbox,
}

impl ToggleStyle {
    /// The concrete style this style draws on the target platform.
    ///
    /// [`Self::Automatic`] resolves to the platform's default — a checkbox on
    /// macOS, a switch everywhere else, iOS and Android included; every
    /// other style is already concrete. The default is a property of the
    /// target, not of the backend, because a toggle's stretch axis depends on
    /// it and is read before `body` runs, without an environment.
    #[must_use]
    pub const fn resolved(self) -> Self {
        match self {
            Self::Automatic => {
                if cfg!(target_os = "macos") {
                    Self::Checkbox
                } else {
                    Self::Switch
                }
            }
            style => style,
        }
    }
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
    ///
    /// The native payload a backend receives is resolved: its style is never
    /// [`ToggleStyle::Automatic`], and its label's display mode is the
    /// effective one.
    pub style: ToggleStyle,
}

configurable!(
    /// A control that toggles between on and off states.
    ///
    /// Toggle displays a switch or a checkbox with a label. It's commonly
    /// used for settings that can be turned on or off.
    ///
    /// The label is required at construction; see
    /// [the label module](crate::label) for why. Use
    /// [`Self::hide_label`] when the surrounding context already explains the
    /// control — the label stays in the accessibility tree.
    ///
    /// # Layout Behavior
    ///
    /// A switch with a visible label expands horizontally to fill the
    /// available width, placing the label at the leading edge and the switch
    /// at the trailing edge with the free space between. A checkbox (box,
    /// then label) and a toggle with a hidden label are content-sized.
    /// [`ToggleStyle::Automatic`] takes the platform's default style first —
    /// a switch on iOS and Android, a checkbox on macOS.
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
    // - The payload arrives resolved: `style` is `Switch` or `Checkbox`, never
    //   `Automatic`, and the label's display mode is the effective one.
    // - Switch with a visible label: stretchAxis `.horizontal`; sizeThatFits
    //   answers the proposed width (never below the row's intrinsic width)
    //   and the intrinsic height; label leading, switch trailing, flexible
    //   space between.
    // - Checkbox, or any toggle with a hidden label: stretchAxis `.none`;
    //   sizeThatFits answers the intrinsic size; box first, then the label.
    // - Read the axis from `NativeView::stretch_axis` on the payload; do not
    //   restate the rule in the backend.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Toggle,
    ToggleConfig,
    |config| config.layout_stretch_axis(),
    resolve | config,
    env | config.resolve(env)
);

impl ToggleConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        self.style = self.style.resolved();
        self
    }

    /// `docs/layout-spec.md` §3: a toggle is `Horizontal` when it draws the
    /// switch style with a visible label, and `None` otherwise.
    ///
    /// Before resolution only the label's own display mode is known — an
    /// environment-wide [`LabelDisplayMode`] is not — so an unresolved
    /// label counts as visible; the resolved payload answers exactly.
    const fn layout_stretch_axis(&self) -> StretchAxis {
        let label_visible = !matches!(
            self.label.display_mode_preference(),
            LabelDisplayMode::Hidden
        );
        match self.style.resolved() {
            ToggleStyle::Switch if label_visible => StretchAxis::Horizontal,
            ToggleStyle::Switch | ToggleStyle::Checkbox => StretchAxis::None,
            ToggleStyle::Automatic => panic!("ToggleStyle::resolved never answers Automatic"),
        }
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

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::{Environment, NativeView, View};
    use waterui_locale::locales;

    use super::{Toggle, ToggleConfig, ToggleStyle};
    use crate::label::LabelDisplayMode;

    /// `docs/layout-spec.md` §3 over the style × label-visibility matrix,
    /// both as the static answer a container reads before `body` and as the
    /// resolved payload a backend receives.
    #[test]
    fn stretch_axis_follows_style_and_label_visibility() {
        let on = Binding::bool(false);
        let switch_axis = |visible: bool| {
            if visible {
                StretchAxis::Horizontal
            } else {
                StretchAxis::None
            }
        };
        let automatic_axis = |visible: bool| match ToggleStyle::Automatic.resolved() {
            ToggleStyle::Switch => switch_axis(visible),
            _ => StretchAxis::None,
        };
        let cases = [
            (ToggleStyle::Switch, true, switch_axis(true)),
            (ToggleStyle::Switch, false, switch_axis(false)),
            (ToggleStyle::Checkbox, true, StretchAxis::None),
            (ToggleStyle::Checkbox, false, StretchAxis::None),
            (ToggleStyle::Automatic, true, automatic_axis(true)),
            (ToggleStyle::Automatic, false, automatic_axis(false)),
        ];
        let env = test_env();
        for (style, visible, expected) in cases {
            let make = || {
                let toggle = Toggle::new("Wi-Fi", &on).style(style);
                if visible { toggle } else { toggle.hide_label() }
            };
            assert_eq!(
                View::stretch_axis(&make()),
                expected,
                "{style:?}, label visible: {visible} (static)"
            );
            let resolved = make().0.resolve(&env);
            assert_eq!(
                NativeView::stretch_axis(&resolved),
                expected,
                "{style:?}, label visible: {visible} (resolved payload)"
            );
        }
    }

    /// The payload a backend receives never carries `Automatic`: it is the
    /// target's default, a checkbox on macOS and a switch elsewhere.
    #[test]
    fn resolution_replaces_automatic_with_the_platform_default() {
        let expected = if cfg!(target_os = "macos") {
            ToggleStyle::Checkbox
        } else {
            ToggleStyle::Switch
        };
        assert_eq!(ToggleStyle::Automatic.resolved(), expected);
        for style in [ToggleStyle::Switch, ToggleStyle::Checkbox] {
            assert_eq!(style.resolved(), style);
        }
        let on = Binding::bool(false);
        let resolved: ToggleConfig = Toggle::new("Wi-Fi", &on).0.resolve(&test_env());
        assert_eq!(resolved.style, expected);
    }

    /// An environment-wide hidden display mode reaches the resolved payload:
    /// a switch whose label the environment hides is content-sized.
    #[test]
    fn an_environment_hidden_label_stops_a_switch_stretching() {
        let on = Binding::bool(false);
        let mut env = test_env();
        env.insert(LabelDisplayMode::Hidden);
        let resolved = Toggle::new("Wi-Fi", &on).switch().0.resolve(&env);
        assert_eq!(NativeView::stretch_axis(&resolved), StretchAxis::None);
    }

    fn test_env() -> Environment {
        let mut env = Environment::new();
        env.insert(locales::EN);
        env
    }
}
