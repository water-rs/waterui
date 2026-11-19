//! Color Picker Component

use nami::Binding;
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::view::{ConfigurableView, ViewConfiguration};
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_graphics::color::Color;

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Configuration for the `ColorPicker` component.
pub struct ColorPickerConfig<S> {
    /// The label of the color picker.
    label: ControlLabel<S>,
    /// The binding to the color value.
    value: Binding<Color>,
    /// Whether to support alpha channel selection.
    support_alpha: bool,
    /// Whether to support HDR color selection.
    support_hdr: bool,
}

impl<S> ColorPickerConfig<S> {
    /// The label of the color picker.
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

    /// The binding to the color value.
    #[must_use]
    pub const fn value(&self) -> &Binding<Color> {
        &self.value
    }

    /// Whether to support alpha channel selection.
    #[must_use]
    pub const fn support_alpha(&self) -> bool {
        self.support_alpha
    }

    /// Whether to support HDR color selection.
    #[must_use]
    pub const fn support_hdr(&self) -> bool {
        self.support_hdr
    }
}

impl<S: 'static> NativeView for ColorPickerConfig<S> {}

impl<S: 'static> MapSlots<S> for ColorPickerConfig<S> {
    type With<T: 'static> = ColorPickerConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ColorPickerConfig<T> {
        ColorPickerConfig {
            label: self.label.map_slots(f),
            value: self.value,
            support_alpha: self.support_alpha,
            support_hdr: self.support_hdr,
        }
    }
}

/// A control for selecting colors.
///
/// `ColorPicker` provides a platform-native color selection interface.
///
/// # Layout Behavior
///
/// `ColorPicker` sizes itself to fit its content and never stretches to fill extra space.
/// In a stack, it takes only the space it needs.
#[derive(Debug, Clone)]
#[must_use]
pub struct ColorPicker<L> {
    label: Label<L>,
    value: Binding<Color>,
    support_alpha: bool,
    support_hdr: bool,
}

impl<L> ColorPicker<L> {
    /// Creates a new `ColorPicker` with the given semantic label and value
    /// binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<Color>) -> Self {
        Self {
            label: label.into_label(),
            value: value.clone(),
            support_alpha: false,
            support_hdr: false,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Enables alpha channel support.
    pub const fn with_alpha(mut self) -> Self {
        self.support_alpha = true;
        self
    }

    /// Enables HDR color support.
    pub const fn with_hdr(mut self) -> Self {
        self.support_hdr = true;
        self
    }
}

impl<L: LabelKind> ColorPicker<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> ColorPickerConfig<S> {
        ColorPickerConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
            support_alpha: self.support_alpha,
            support_hdr: self.support_hdr,
        }
    }
}

impl<L: LabelKind> ConfigurableView for ColorPicker<L> {
    type Config = ColorPickerConfig<Label<L>>;

    fn config(self) -> Self::Config {
        ColorPickerConfig {
            label: self.label.into_unscoped_control_label(),
            value: self.value,
            support_alpha: self.support_alpha,
            support_hdr: self.support_hdr,
        }
    }
}

impl<S: View> ViewConfiguration for ColorPickerConfig<S> {
    type View = ColorPicker<ControlLabel<S>>;

    fn render(self) -> Self::View {
        let mode = self.label.display_mode_preference();
        ColorPicker {
            label: Label::from_control_label(self.label, mode),
            value: self.value,
            support_alpha: self.support_alpha,
            support_hdr: self.support_hdr,
        }
    }
}

impl<S: View> From<ColorPickerConfig<S>> for ColorPicker<ControlLabel<S>> {
    fn from(value: ColorPickerConfig<S>) -> Self {
        value.render()
    }
}

impl<L: LabelKind> View for ColorPicker<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(ColorPicker<L>);
