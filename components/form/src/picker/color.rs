use waterui_core::AnyView;
use waterui_reactive::Binding;

use crate::color::Color;

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
}
