use waterui_core::AnyView;
use waterui_reactive::Binding;

use crate::utils::Color;

#[derive(Debug)]
pub struct ColorPickerConfig {
    pub label: AnyView,
    pub value: Binding<Color>,
}

configurable!(ColorPicker, ColorPickerConfig);
