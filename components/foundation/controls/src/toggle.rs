//! A boolean toggle switch backed by a reactive binding.
//!
//! ![Toggle](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/toggle.svg)

use nami::Binding;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::view::{ConfigurableView, ViewConfiguration};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::label::{
    ControlLabel, IntoLabel, Label, LabelDisplayMode, LabelKind, impl_label_style_methods,
};
use crate::size::ControlSize;

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
    /// target, not of the backend, so every backend on one target answers
    /// the same stretch axis for it.
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

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Configuration for the `Toggle` component.
pub struct ToggleConfig<S> {
    /// The label displayed for the toggle.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    label: ControlLabel<S>,
    /// The binding to the toggle state.
    toggle: Binding<bool>,
    /// The visual style of the toggle.
    ///
    /// The native payload a backend receives is resolved: its style is never
    /// [`ToggleStyle::Automatic`], and its label's display mode is the
    /// effective one.
    style: ToggleStyle,
    /// How large the toggle is drawn.
    size: ControlSize,
}

impl<S> ToggleConfig<S> {
    /// The label displayed for the toggle.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: ControlLabel<S>) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut ControlLabel<S> {
        &mut self.label
    }

    /// The binding to the toggle state.
    #[must_use]
    pub const fn toggle(&self) -> &Binding<bool> {
        &self.toggle
    }

    /// The visual style of the toggle.
    ///
    /// The native payload a backend receives is resolved: its style is never
    /// [`ToggleStyle::Automatic`], and its label's display mode is the
    /// effective one.
    #[must_use]
    pub const fn style(&self) -> ToggleStyle {
        self.style
    }

    /// How large the toggle is drawn.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (ControlLabel<S>, Binding<bool>, ToggleStyle, ControlSize) {
        (self.label, self.toggle, self.style, self.size)
    }

    /// `docs/layout-spec.md` §3: a toggle is `Horizontal` when it draws the
    /// switch style with a visible label, and `None` otherwise. Backends ask
    /// the resolved payload, whose label carries an environment-wide
    /// [`LabelDisplayMode`] too.
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

impl<S: 'static> NativeView for ToggleConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout_stretch_axis()
    }
}

impl<S: 'static> MapSlots<S> for ToggleConfig<S> {
    type With<T: 'static> = ToggleConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ToggleConfig<T> {
        ToggleConfig {
            label: self.label.map_slots(f),
            toggle: self.toggle,
            style: self.style,
            size: self.size,
        }
    }
}

/// A control that toggles between on and off states.
///
/// Toggle displays a switch or a checkbox with a label. It's commonly
/// used for settings that can be turned on or off.
///
/// The label is required at construction; see
/// [the label module](mod@crate::label) for why. Use
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
#[derive(Debug, Clone)]
#[must_use]
#[expect(
    clippy::struct_field_names,
    reason = "stores the boolean toggle binding for `Toggle`"
)]
pub struct Toggle<L> {
    label: Label<L>,
    toggle: Binding<bool>,
    style: ToggleStyle,
    size: ControlSize,
}

impl<L> Toggle<L> {
    /// Creates a new `Toggle` with the specified label and binding for the
    /// toggle state.
    ///
    /// The label is mandatory. To keep it out of the visual chrome without
    /// losing its accessibility name, chain [`Self::hide_label`].
    pub fn new<I: IntoLabel<Kind = L>>(label: I, toggle: &Binding<bool>) -> Self {
        Self {
            label: label.into_label(),
            toggle: toggle.clone(),
            style: ToggleStyle::default(),
            size: ControlSize::Small,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the visual style of the toggle.
    pub const fn style(mut self, style: ToggleStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets how large the toggle is drawn. A toggle is
    /// [`ControlSize::Small`], like a [`Button`](crate::button::Button), when
    /// none is chosen; a theme decides what each size changes.
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Changes the toggle to switch style.
    pub const fn switch(self) -> Self {
        self.style(ToggleStyle::Switch)
    }

    /// Changes the toggle to checkbox style.
    pub const fn checkbox(self) -> Self {
        self.style(ToggleStyle::Checkbox)
    }
}

impl<L: LabelKind> Toggle<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> ToggleConfig<S> {
        ToggleConfig {
            label: self.label.into_control_label::<S>(env),
            toggle: self.toggle,
            style: self.style.resolved(),
            size: self.size,
        }
    }
}

impl<L: LabelKind> ConfigurableView for Toggle<L> {
    type Config = ToggleConfig<Label<L>>;

    fn config(self) -> Self::Config {
        ToggleConfig {
            label: self.label.into_unscoped_control_label(),
            toggle: self.toggle,
            style: self.style,
            size: self.size,
        }
    }
}

impl<S: View> ViewConfiguration for ToggleConfig<S> {
    type View = Toggle<ControlLabel<S>>;

    fn render(self) -> Self::View {
        let mode = self.label.display_mode_preference();
        Toggle {
            label: Label::from_control_label(self.label, mode),
            toggle: self.toggle,
            style: self.style,
            size: self.size,
        }
    }
}

impl<S: View> From<ToggleConfig<S>> for Toggle<ControlLabel<S>> {
    fn from(value: ToggleConfig<S>) -> Self {
        value.render()
    }
}

impl<L: LabelKind> View for Toggle<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(Toggle<L>);

/// Creates a new `Toggle` with the specified label and binding for the toggle state.
pub fn toggle<L: IntoLabel>(label: L, toggle: &Binding<bool>) -> Toggle<L::Kind> {
    Toggle::new(label, toggle)
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::{AnyView, Environment, NativeView};
    use waterui_locale::locales;

    use super::{Toggle, ToggleConfig, ToggleStyle};
    use crate::label::LabelDisplayMode;

    /// `docs/layout-spec.md` §3 over the style × label-visibility matrix, on
    /// the resolved payload a backend receives.
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
            let resolved = make().into_config::<AnyView>(&env);
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
        let resolved: ToggleConfig<AnyView> = Toggle::new("Wi-Fi", &on).into_config(&test_env());
        assert_eq!(resolved.style, expected);
    }

    /// An environment-wide hidden display mode reaches the resolved payload:
    /// a switch whose label the environment hides is content-sized.
    #[test]
    fn an_environment_hidden_label_stops_a_switch_stretching() {
        let on = Binding::bool(false);
        let mut env = test_env();
        env.insert(LabelDisplayMode::Hidden);
        let resolved = Toggle::new("Wi-Fi", &on)
            .switch()
            .into_config::<AnyView>(&env);
        assert_eq!(NativeView::stretch_axis(&resolved), StretchAxis::None);
    }

    fn test_env() -> Environment {
        let mut env = Environment::new();
        env.insert(locales::EN);
        env
    }
}
