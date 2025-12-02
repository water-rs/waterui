//! Color Picker Component

use nami::Binding;
use waterui_color::Color;
use waterui_core::{AnyView, NativeView, View, configurable};

#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `ColorPicker` component.
pub struct ColorPickerConfig {
    /// The label of the color picker.
    pub label: AnyView,
    /// The binding to the color value.
    pub value: Binding<Color>,
}

impl NativeView for ColorPickerConfig {}

configurable!(
    /// A control for selecting colors.
    ///
    /// ColorPicker provides a platform-native color selection interface.
    ///
    /// # Layout Behavior
    ///
    /// ColorPicker sizes itself to fit its content and never stretches to fill extra space.
    /// In a stack, it takes only the space it needs.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Stretch Axis: `None` - ColorPicker never expands to fill available space.
    // Size: Determined by platform color picker UI
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    ColorPicker,
    ColorPickerConfig
);

impl ColorPicker {
    /// Creates a new `ColorPicker` with the given value.
    #[must_use]
    pub fn new(value: &Binding<Color>) -> Self {
        Self(ColorPickerConfig {
            label: AnyView::default(),
            value: value.clone(),
        })
    }

    /// Sets the label of the color picker.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }
}
