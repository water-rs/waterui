//! Color Picker Component

use waterui_color::Color;
use waterui_core::{AnyView, View, configurable};
use waterui_reactive::Binding;

#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `ColorPicker` component.
pub struct ColorPickerConfig {
    /// The label of the color picker.
    pub label: AnyView,
    /// The binding to the color value.
    pub value: Binding<Color>,
}

configurable!(ColorPicker, ColorPickerConfig);

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
