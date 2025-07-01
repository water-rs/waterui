use waterui_color::Color;
use waterui_core::{AnyView, View, configurable};
use waterui_reactive::Binding;

#[derive(Debug)]
#[non_exhaustive]
pub struct ColorPickerConfig {
    pub label: AnyView,
    pub value: Binding<Color>,
}

configurable!(ColorPicker, ColorPickerConfig);

impl ColorPicker {
    pub fn new(value: &Binding<Color>) -> Self {
        Self(ColorPickerConfig {
            label: AnyView::default(),
            value: value.clone(),
        })
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }
}
