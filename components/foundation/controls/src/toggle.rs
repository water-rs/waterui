//! A boolean toggle switch backed by a reactive binding.
//!
//! ![Toggle](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/toggle.svg)

use core::fmt;

use nami::{Binding, Computed, Signal as _, SignalExt as _};
use waterui_accessibility::{
    AccessibilityRole, AccessibilityState, AccessibilityStateSignal, default_role,
};
use waterui_core::backend::{
    Backend, ChildSlot, HookSlot, HookableParent, MapSlots, ParentHookScope, offer_hookable_parent,
};
use waterui_core::env::Installed;
use waterui_core::layout::StretchAxis;
use waterui_core::plugin::Plugin;
use waterui_core::view::hook::Hook;
use waterui_core::{AnyView, Environment, Metadata, Native, NativeView, View, contract_violation};
use waterui_interaction::gesture::{GestureObserver, TapGesture};
use waterui_interaction::interaction::{Disabled, InteractionReport, InteractionState};
use waterui_text::styled::StyledStr;

use crate::label::{
    ControlLabel, IntoLabel, Label, LabelDisplayMode, LabelKind, impl_label_style_methods,
};
use crate::size::ControlSize;

/// Visual style options for toggle controls.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToggleStyle {
    /// The platform's default style, resolved from the environment
    /// ([`Self::Switch`] on Hydrolysis and iOS, [`Self::Checkbox`] on macOS
    /// `AppKit`, or [`Self::Switch`] when no backend default is installed).
    /// See [`Self::resolve`].
    #[default]
    Automatic,
    /// A switch-style toggle (sliding pill).
    Switch,
    /// A checkbox-style toggle (square with checkmark).
    Checkbox,
}

/// A toggle style installed on a subtree becomes the default for toggles
/// that did not pick one, the same way [`LabelDisplayMode`] and
/// [`ButtonStyle`](crate::button::ButtonStyle) adapt controls to their
/// surroundings.
impl Plugin for ToggleStyle {}

/// Backend-installed default [`ToggleStyle`] read when [`ToggleStyle::Automatic`]
/// is resolved without an ambient [`ToggleStyle`] override in the environment.
#[derive(Debug, Clone, Copy)]
struct DefaultToggleStyle(ToggleStyle);

impl ToggleStyle {
    /// Registers `style` as the platform's default [`ToggleStyle`] in `env`
    /// for descendants whose style is [`Self::Automatic`].
    ///
    /// A `.with(ToggleStyle::...)` ancestor or explicit `.style(...)` wins over
    /// the registered default, while an explicit `.with(ToggleStyle::Automatic)`
    /// restores it.
    pub fn install_default(style: Self, env: &mut Environment) {
        let resolved = match style {
            Self::Automatic => {
                contract_violation!(
                    "ToggleStyle::install_default requires a concrete style; \
                     using ToggleStyle::Switch"
                );
                Self::Switch
            }
            concrete => concrete,
        };
        env.insert(DefaultToggleStyle(resolved));
    }

    /// Resolves this style in `env`.
    ///
    /// [`Self::Automatic`] reads an ambient [`ToggleStyle`] installed in `env`
    /// first, then falls back to the backend's registered platform default
    /// ([`Self::install_default`]), or [`Self::Switch`] when neither is present.
    /// Concrete styles resolve to themselves.
    #[must_use]
    pub fn resolve(self, env: &Environment) -> Self {
        match self {
            Self::Automatic => match env.get::<Self>().copied() {
                Some(Self::Automatic) | None => env
                    .get::<DefaultToggleStyle>()
                    .map_or(Self::Switch, |default| default.0),
                Some(style) => style,
            },
            style => style,
        }
    }
}

/// The label view passed to a custom [`Toggle`] style.
pub struct ToggleLabel<L = ControlLabel<HookSlot>>(L);

impl<L> fmt::Debug for ToggleLabel<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ToggleLabel")
    }
}

impl<L: View> View for ToggleLabel<L> {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.0.dispatch(backend, env, cx)
    }
}

/// Semantic configuration passed to [`CustomToggleStyle::make_body`].
pub struct ToggleStyleConfiguration<L> {
    label: L,
    is_on: Binding<bool>,
    size: ControlSize,
    interaction: Computed<InteractionState>,
}

impl<L> fmt::Debug for ToggleStyleConfiguration<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToggleStyleConfiguration")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<L> ToggleStyleConfiguration<L> {
    /// Borrows the toggle's label view.
    #[must_use]
    pub const fn label(&self) -> &L {
        &self.label
    }

    /// Extracts the toggle's label view.
    #[must_use]
    pub fn into_label(self) -> L {
        self.label
    }

    /// Borrows the reactive binding controlling the toggle's on/off state.
    #[must_use]
    pub const fn is_on(&self) -> &Binding<bool> {
        &self.is_on
    }

    /// Takes the configuration apart into `(label, is_on, size, interaction_state)`.
    #[must_use]
    pub fn into_parts(self) -> (L, Binding<bool>, ControlSize, Computed<InteractionState>) {
        (self.label, self.is_on, self.size, self.interaction)
    }

    /// The control size requested for this toggle.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// Reactive interaction state flags for this toggle
    /// (hovered, focused, pressed, disabled, etc.).
    #[must_use]
    pub const fn interaction_state(&self) -> &Computed<InteractionState> {
        &self.interaction
    }

    /// Whether a pointer or key is currently pressing the toggle.
    #[must_use]
    pub fn is_pressed(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::PRESSED))
            .computed()
    }

    /// Whether a pointer is hovering over the toggle.
    #[must_use]
    pub fn is_hovered(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::HOVERED))
            .computed()
    }

    /// Whether the toggle currently shows keyboard focus.
    #[must_use]
    pub fn is_focused(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::FOCUSED))
            .computed()
    }

    /// Whether the toggle is currently disabled.
    #[must_use]
    pub fn is_disabled(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::DISABLED))
            .computed()
    }
}

/// A custom toggle style that defines visual appearance while preserving
/// standard tap-to-toggle behavior, disabled gating, interaction reporting,
/// and checked accessibility semantics.
///
/// Apply a custom style to one toggle with [`Toggle::style`] or to an entire
/// view subtree with `ViewExt::toggle_style`.
pub trait CustomToggleStyle: 'static {
    /// Builds the visual body of a toggle from its semantic `configuration`.
    fn make_body<L: View>(
        &self,
        configuration: ToggleStyleConfiguration<L>,
    ) -> impl View + use<Self, L>;
}

mod sealed {
    pub trait Sealed {}
}

#[doc(hidden)]
#[derive(Debug)]
pub struct BuiltinStyleMarker;

#[doc(hidden)]
#[derive(Debug)]
pub struct CustomStyleMarker;

/// Wrapper applying a [`CustomToggleStyle`].
#[derive(Debug, Clone)]
pub struct CustomToggle<S>(S);

impl sealed::Sealed for ToggleStyle {}
impl<S: CustomToggleStyle> sealed::Sealed for CustomToggle<S> {}

/// Types accepted by [`Toggle::style`] and `ViewExt::toggle_style`:
/// built-in [`ToggleStyle`] or [`CustomToggleStyle`].
pub trait IntoToggleStyle<Marker>: Sized + 'static {
    /// Concrete stored style representation.
    #[doc(hidden)]
    type Style: SealedToggleStyle;

    /// Converts this value into the stored style representation.
    #[doc(hidden)]
    fn into_toggle_style(self) -> Self::Style;
}

impl IntoToggleStyle<BuiltinStyleMarker> for ToggleStyle {
    type Style = Self;

    fn into_toggle_style(self) -> Self::Style {
        self
    }
}

impl<S: CustomToggleStyle> IntoToggleStyle<CustomStyleMarker> for S {
    type Style = CustomToggle<Self>;

    fn into_toggle_style(self) -> Self::Style {
        CustomToggle(self)
    }
}

#[doc(hidden)]
pub trait SealedToggleStyle: sealed::Sealed + Sized + 'static {
    fn toggle_body<L: LabelKind>(toggle: Toggle<L, Self>, env: &Environment) -> impl View;

    fn dispatch_toggle<B: Backend, L: LabelKind>(
        toggle: Toggle<L, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output;

    fn hook_apply(
        &self,
        target: ToggleStyleTarget,
        env: &Environment,
    ) -> Result<impl View, ToggleStyleTarget>;
}

fn compose_custom_toggle<L: View, S: CustomToggleStyle>(
    style: &S,
    label: L,
    accessibility_label: &Computed<StyledStr>,
    toggle: &Binding<bool>,
    size: ControlSize,
    env: &Environment,
) -> impl View + use<L, S> {
    let (report, interaction) = crate::button::interaction_signals(env);
    let body = style.make_body(ToggleStyleConfiguration {
        label,
        is_on: toggle.clone(),
        size,
        interaction,
    });
    let tap_toggle = toggle.clone();
    let gesture =
        GestureObserver::without_event(TapGesture::new(), move |action_env: Environment| {
            if !Disabled::resolve(&action_env, false).snapshot() {
                tap_toggle.set(!tap_toggle.snapshot());
            }
        });
    let with_gesture = Metadata::new(body, gesture);
    let checked_state = AccessibilityStateSignal::resolve(
        env,
        toggle.map(|on| AccessibilityState::new().checked(on)),
    );
    let with_state = Metadata::new(with_gesture, checked_state);
    let with_label =
        crate::button::wrap_default_accessibility_label(env, with_state, accessibility_label);
    let with_role = default_role(env, with_label, AccessibilityRole::Switch);
    Installed::new(with_role, InteractionReport(report))
}

impl SealedToggleStyle for ToggleStyle {
    fn toggle_body<L: LabelKind>(toggle: Toggle<L, Self>, env: &Environment) -> impl View {
        Native::new(toggle.into_config::<AnyView>(env))
    }

    fn dispatch_toggle<B: Backend, L: LabelKind>(
        toggle: Toggle<L, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        backend.native(toggle.into_config::<B::Slot>(env), env, cx)
    }

    fn hook_apply(
        &self,
        target: ToggleStyleTarget,
        _env: &Environment,
    ) -> Result<impl View, ToggleStyleTarget> {
        let data = *target.0;
        let mode = data.label.display_mode_preference();
        Ok(Toggle {
            label: Label::from_control_label(data.label, mode),
            toggle: data.toggle,
            style: *self,
            explicit_style: true,
            size: Some(data.size),
        })
    }
}

impl<S: CustomToggleStyle> SealedToggleStyle for CustomToggle<S> {
    fn toggle_body<L: LabelKind>(toggle: Toggle<L, Self>, env: &Environment) -> impl View {
        let label = toggle.label.resolve(env);
        let a11y_label = label.accessibility_label();
        compose_custom_toggle(
            &toggle.style.0,
            label,
            &a11y_label,
            &toggle.toggle,
            ControlSize::resolve(toggle.size, env, ControlSize::Small),
            env,
        )
    }

    fn dispatch_toggle<B: Backend, L: LabelKind>(
        toggle: Toggle<L, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        let label = toggle.label.resolve(env);
        let a11y_label = label.accessibility_label();
        compose_custom_toggle(
            &toggle.style.0,
            label,
            &a11y_label,
            &toggle.toggle,
            ControlSize::resolve(toggle.size, env, ControlSize::Small),
            env,
        )
        .dispatch(backend, env, cx)
    }

    fn hook_apply(
        &self,
        target: ToggleStyleTarget,
        env: &Environment,
    ) -> Result<impl View, ToggleStyleTarget> {
        let data = *target.0;
        let a11y_label = data.label.accessibility_label();
        let label = ToggleLabel(data.label);
        Ok(compose_custom_toggle(
            &self.0,
            label,
            &a11y_label,
            &data.toggle,
            data.size,
            env,
        ))
    }
}

struct ToggleStyleTargetData {
    label: ControlLabel<HookSlot>,
    toggle: Binding<bool>,
    size: ControlSize,
}

/// Internal hook target representing a [`Toggle`] eligible for subtree
/// styling via `ViewExt::toggle_style`.
#[doc(hidden)]
pub struct ToggleStyleTarget(alloc::boxed::Box<ToggleStyleTargetData>);

impl fmt::Debug for ToggleStyleTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToggleStyleTarget")
            .field("size", &self.0.size)
            .finish_non_exhaustive()
    }
}

impl NativeView for ToggleStyleTarget {}

/// Hook that applies a toggle style to every non-explicitly-styled [`Toggle`]
/// in a subtree.
#[doc(hidden)]
#[derive(Debug)]
pub struct ToggleStyleHook<S>(alloc::rc::Rc<S>);

impl<S> Clone for ToggleStyleHook<S> {
    fn clone(&self) -> Self {
        Self(alloc::rc::Rc::clone(&self.0))
    }
}

impl<S: SealedToggleStyle> ToggleStyleHook<S> {
    /// Creates a subtree toggle-style hook.
    #[must_use]
    pub fn new(style: S) -> Self {
        Self(alloc::rc::Rc::new(style))
    }
}

impl<S: SealedToggleStyle> Hook for ToggleStyleHook<S> {
    type Target = ToggleStyleTarget;

    fn apply(&self, target: Self::Target, env: &Environment) -> Result<impl View, Self::Target> {
        self.0.hook_apply(target, env)
    }
}

/// Pending toggle configuration saved while offering [`ToggleStyleTarget`] to hooks,
/// allowing exact restoration without type erasure if every hook declines.
#[doc(hidden)]
pub struct ToggleStylePending<S> {
    label: ControlLabel<S>,
    toggle: Binding<bool>,
    style: ToggleStyle,
    explicit_style: bool,
    size: ControlSize,
}

impl<S> fmt::Debug for ToggleStylePending<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToggleStylePending")
            .field("style", &self.style)
            .field("explicit_style", &self.explicit_style)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<S: 'static> ToggleStylePending<S> {
    /// Restores the original [`ToggleConfig<S>`] after all hooks declined.
    #[must_use]
    pub fn restore(self) -> ToggleConfig<S> {
        ToggleConfig {
            label: self.label,
            toggle: self.toggle,
            style: self.style,
            explicit_style: self.explicit_style,
            size: self.size,
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
    /// Whether the visual style was set explicitly on the toggle.
    explicit_style: bool,
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

    /// Whether the toggle chose its style explicitly.
    #[doc(hidden)]
    #[must_use]
    pub const fn has_explicit_style(&self) -> bool {
        self.explicit_style
    }

    /// `docs/layout-spec.md` §3: a toggle is `Horizontal` when it draws the
    /// switch style with a visible label, and `None` otherwise. Backends ask
    /// the resolved payload, whose style and [`LabelDisplayMode`] have already
    /// been resolved against the environment.
    fn layout_stretch_axis(&self) -> StretchAxis {
        let label_visible = !matches!(
            self.label.display_mode_preference(),
            LabelDisplayMode::Hidden
        );
        match self.style {
            ToggleStyle::Switch if label_visible => StretchAxis::Horizontal,
            ToggleStyle::Switch | ToggleStyle::Checkbox => StretchAxis::None,
            ToggleStyle::Automatic => {
                contract_violation!(
                    "ToggleConfig::stretch_axis received an unresolved Automatic style; \
                     resolve through the environment before measuring"
                );
                StretchAxis::None
            }
        }
    }
}

impl<S: 'static> HookableParent<S> for ToggleConfig<S> {
    type StyleTarget = ToggleStyleTarget;
    type Pending = ToggleStylePending<S>;

    fn is_style_hookable(&self) -> bool {
        !self.explicit_style
    }

    fn into_style_pending(
        self,
        map_slot: impl Fn(S) -> HookSlot + Clone + 'static,
    ) -> (ToggleStyleTarget, ToggleStylePending<S>) {
        let style_label = self.label.clone().map_slots(map_slot);
        let target = ToggleStyleTarget(alloc::boxed::Box::new(ToggleStyleTargetData {
            label: style_label,
            toggle: self.toggle.clone(),
            size: self.size,
        }));
        let pending = ToggleStylePending {
            label: self.label,
            toggle: self.toggle,
            style: self.style,
            explicit_style: self.explicit_style,
            size: self.size,
        };
        (target, pending)
    }

    fn restore_pending(pending: Self::Pending) -> Self {
        pending.restore()
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
            explicit_style: self.explicit_style,
            size: self.size,
        }
    }

    fn offer_style_hook<Scope: ParentHookScope<S>>(
        self,
        scope: &mut Scope,
        env: &Environment,
        cx: Scope::Ctx,
    ) -> Result<Scope::Output, (Self, Scope::Ctx)> {
        offer_hookable_parent(self, scope, env, cx)
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
/// [`ToggleStyle::Automatic`] takes the ambient style installed in the
/// environment (a `.with(ToggleStyle::...)` ancestor or the backend's
/// platform default).
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
pub struct Toggle<L, Style = ToggleStyle> {
    label: Label<L>,
    toggle: Binding<bool>,
    style: Style,
    explicit_style: bool,
    size: Option<ControlSize>,
}

impl<L> Toggle<L, ToggleStyle> {
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
            explicit_style: false,
            size: None,
        }
    }

    /// Changes the toggle to switch style.
    pub const fn switch(mut self) -> Self {
        self.style = ToggleStyle::Switch;
        self.explicit_style = true;
        self
    }

    /// Changes the toggle to checkbox style.
    pub const fn checkbox(mut self) -> Self {
        self.style = ToggleStyle::Checkbox;
        self.explicit_style = true;
        self
    }
}

impl<L, Style> Toggle<L, Style> {
    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the visual style of the toggle.
    ///
    /// Accepts a built-in [`ToggleStyle`] or a [`CustomToggleStyle`].
    pub fn style<NewStyle, Marker>(
        self,
        style: NewStyle,
    ) -> Toggle<L, <NewStyle as IntoToggleStyle<Marker>>::Style>
    where
        NewStyle: IntoToggleStyle<Marker>,
    {
        Toggle {
            label: self.label,
            toggle: self.toggle,
            style: style.into_toggle_style(),
            explicit_style: true,
            size: self.size,
        }
    }

    /// Sets how large the toggle is drawn. When omitted, the toggle inherits
    /// the nearest ambient [`ControlSize`] or defaults to
    /// [`ControlSize::Small`], like a [`Button`](crate::button::Button); a
    /// theme decides what each size changes.
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = Some(size);
        self
    }
}

impl<L: LabelKind> Toggle<L, ToggleStyle> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> ToggleConfig<S> {
        ToggleConfig {
            label: self.label.into_control_label::<S>(env),
            toggle: self.toggle,
            style: self.style.resolve(env),
            explicit_style: self.explicit_style,
            size: ControlSize::resolve(self.size, env, ControlSize::Small),
        }
    }
}

impl<L: LabelKind> Toggle<L> {
    /// Returns the configuration for this toggle.
    #[must_use]
    pub fn config(self) -> ToggleConfig<Label<L>> {
        ToggleConfig {
            label: self.label.into_unscoped_control_label(),
            toggle: self.toggle,
            style: self.style,
            explicit_style: self.explicit_style,
            size: self.size.unwrap_or(ControlSize::Small),
        }
    }
}

impl<S: View> From<ToggleConfig<S>> for Toggle<ControlLabel<S>> {
    fn from(value: ToggleConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            toggle: value.toggle,
            style: value.style,
            explicit_style: value.explicit_style,
            size: Some(value.size),
        }
    }
}

impl<L: LabelKind, Style: SealedToggleStyle> View for Toggle<L, Style> {
    fn body(self, env: &Environment) -> impl View {
        Style::toggle_body(self, env)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        Style::dispatch_toggle(self, backend, env, cx)
    }
}

impl_label_style_methods!(Toggle<L, Style>);

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
        let on = Binding::new(false);
        let switch_axis = |visible: bool| {
            if visible {
                StretchAxis::Horizontal
            } else {
                StretchAxis::None
            }
        };
        let cases = [
            (ToggleStyle::Switch, true, switch_axis(true)),
            (ToggleStyle::Switch, false, switch_axis(false)),
            (ToggleStyle::Checkbox, true, StretchAxis::None),
            (ToggleStyle::Checkbox, false, StretchAxis::None),
            (ToggleStyle::Automatic, true, switch_axis(true)),
            (ToggleStyle::Automatic, false, switch_axis(false)),
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

        let mut checkbox_default = test_env();
        ToggleStyle::install_default(ToggleStyle::Checkbox, &mut checkbox_default);
        let resolved = Toggle::new("Wi-Fi", &on).into_config::<AnyView>(&checkbox_default);
        assert_eq!(NativeView::stretch_axis(&resolved), StretchAxis::None);

        let switch_override = checkbox_default.extending(ToggleStyle::Switch);
        let resolved = Toggle::new("Wi-Fi", &on).into_config::<AnyView>(&switch_override);
        assert_eq!(NativeView::stretch_axis(&resolved), StretchAxis::Horizontal);
    }

    /// The payload a backend receives never carries `Automatic`: it resolves
    /// from the ambient `ToggleStyle` in the environment first, then the
    /// backend's installed default, or `Switch` when unconfigured.
    #[test]
    fn resolution_replaces_automatic_with_the_environment_or_default() {
        let empty = test_env();
        assert_eq!(ToggleStyle::Automatic.resolve(&empty), ToggleStyle::Switch);

        let mut checkbox_default = test_env();
        ToggleStyle::install_default(ToggleStyle::Checkbox, &mut checkbox_default);
        assert_eq!(
            ToggleStyle::Automatic.resolve(&checkbox_default),
            ToggleStyle::Checkbox
        );
        for style in [ToggleStyle::Switch, ToggleStyle::Checkbox] {
            assert_eq!(style.resolve(&checkbox_default), style);
        }

        let on = Binding::new(false);
        let resolved: ToggleConfig<AnyView> =
            Toggle::new("Wi-Fi", &on).into_config(&checkbox_default);
        assert_eq!(resolved.style, ToggleStyle::Checkbox);

        let subtree_switch = checkbox_default.extending(ToggleStyle::Switch);
        assert_eq!(
            ToggleStyle::Automatic.resolve(&subtree_switch),
            ToggleStyle::Switch
        );
        let resolved: ToggleConfig<AnyView> =
            Toggle::new("Wi-Fi", &on).into_config(&subtree_switch);
        assert_eq!(resolved.style, ToggleStyle::Switch);

        let reset_automatic = subtree_switch.extending(ToggleStyle::Automatic);
        assert_eq!(
            ToggleStyle::Automatic.resolve(&reset_automatic),
            ToggleStyle::Checkbox
        );
    }

    /// An environment-wide hidden display mode reaches the resolved payload:
    /// a switch whose label the environment hides is content-sized.
    #[test]
    fn an_environment_hidden_label_stops_a_switch_stretching() {
        let on = Binding::new(false);
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

#[cfg(test)]
mod custom_style_tests {
    use alloc::rc::Rc;
    use core::cell::RefCell;
    use nami::{Binding, Signal as _, binding};
    use waterui_accessibility::{
        AccessibilityChecked, AccessibilityLabel, AccessibilityRole, AccessibilityStateSignal,
    };
    use waterui_core::backend::peel_metadata;
    use waterui_core::{AnyView, Environment, View};
    use waterui_interaction::gesture::GestureObserver;
    use waterui_interaction::interaction::{Disabled, InteractionReport, InteractionState};

    use super::{CustomToggleStyle, Toggle, ToggleStyleConfiguration};
    use crate::size::ControlSize;

    struct RecordedToggleConfig {
        is_on: Binding<bool>,
        size: ControlSize,
        disabled: nami::Computed<bool>,
        interaction: nami::Computed<InteractionState>,
    }

    struct RecordingToggleStyle {
        recorded: Rc<RefCell<Option<RecordedToggleConfig>>>,
    }

    impl CustomToggleStyle for RecordingToggleStyle {
        fn make_body<L: View>(
            &self,
            configuration: ToggleStyleConfiguration<L>,
        ) -> impl View + use<L> {
            *self.recorded.borrow_mut() = Some(RecordedToggleConfig {
                is_on: configuration.is_on().clone(),
                size: configuration.size(),
                disabled: configuration.is_disabled(),
                interaction: configuration.interaction_state().clone(),
            });
            configuration.into_label()
        }
    }

    #[test]
    fn custom_toggle_style_exposes_binding_and_toggles_on_tap() {
        let on = Binding::new(false);
        let recorded = Rc::new(RefCell::new(None));
        let disabled_binding = binding(false);
        let mut env = Environment::new();
        Disabled::install(&mut env, disabled_binding.clone());

        let view = Toggle::new("Airplane Mode", &on)
            .size(ControlSize::Large)
            .hide_label()
            .style(RecordingToggleStyle {
                recorded: Rc::clone(&recorded),
            });

        let body = View::body(view, &env);
        let recorded_cfg = recorded.borrow_mut().take().expect("toggle style must run");
        assert_eq!(recorded_cfg.size, ControlSize::Large);
        assert!(!recorded_cfg.is_on.snapshot());
        assert!(!recorded_cfg.disabled.snapshot());

        disabled_binding.set(true);
        assert!(recorded_cfg.disabled.snapshot());
        assert!(
            recorded_cfg
                .interaction
                .snapshot()
                .contains(InteractionState::DISABLED)
        );
        disabled_binding.set(false);

        let env_view = peel_metadata::<Environment>(AnyView::new(body), &env)
            .expect("custom-styled toggle installs InteractionReport");
        assert!(env_view.value.get::<InteractionReport>().is_some());

        let role_view = peel_metadata::<AccessibilityRole>(env_view.content, &env_view.value)
            .expect("custom-styled toggle attaches AccessibilityRole::Switch");
        assert_eq!(role_view.value, AccessibilityRole::Switch);

        let a11y_label = peel_metadata::<AccessibilityLabel>(role_view.content, &env)
            .expect("custom-styled toggle keeps hidden label in accessibility tree");
        assert_eq!(
            a11y_label.value.resolve(&env).snapshot().as_str(),
            "Airplane Mode"
        );

        let state_signal = peel_metadata::<AccessibilityStateSignal>(a11y_label.content, &env)
            .expect("custom-styled toggle publishes checked AccessibilityStateSignal");
        assert_eq!(
            state_signal.value.state().snapshot().checked_state(),
            Some(AccessibilityChecked::False)
        );

        let observer = peel_metadata::<GestureObserver>(state_signal.content, &env)
            .expect("custom-styled toggle attaches TapGesture observer");
        let (_, mut action) = observer.value.into_parts();

        let mut action_env = env.clone();
        action(&action_env).expect("toggle tap succeeds");
        assert!(on.snapshot());
        assert_eq!(
            state_signal.value.state().snapshot().checked_state(),
            Some(AccessibilityChecked::True)
        );

        // Disabled toggle does not flip on tap.
        Disabled::install(&mut action_env, true);
        action(&action_env).expect("disabled toggle tap check succeeds");
        assert!(on.snapshot());
    }

    #[test]
    fn control_size_inherits_from_environment_and_explicit_size_wins() {
        use waterui_core::plugin::Plugin as _;

        let on = Binding::new(false);
        let base_env = Environment::new();
        let mut ambient_env = Environment::new();
        ControlSize::ExtraSmall.install(&mut ambient_env);

        assert_eq!(
            Toggle::new("A", &on)
                .into_config::<AnyView>(&base_env)
                .size(),
            ControlSize::Small
        );
        assert_eq!(
            Toggle::new("A", &on)
                .into_config::<AnyView>(&ambient_env)
                .size(),
            ControlSize::ExtraSmall
        );
        let explicit = Toggle::new("A", &on)
            .size(ControlSize::Large)
            .into_config::<AnyView>(&ambient_env);
        assert_eq!(explicit.size(), ControlSize::Large);
        assert_eq!(
            Toggle::from(explicit)
                .into_config::<AnyView>(&ambient_env)
                .size(),
            ControlSize::Large
        );

        let recorded = Rc::new(RefCell::new(None));
        let _ = View::body(
            Toggle::new("Custom", &on).style(RecordingToggleStyle {
                recorded: Rc::clone(&recorded),
            }),
            &ambient_env,
        );
        assert_eq!(
            recorded.borrow().as_ref().expect("custom toggle ran").size,
            ControlSize::ExtraSmall
        );
    }
}
