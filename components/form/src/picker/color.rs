//! Color Picker Component

use nami::Binding;
use waterui_controls::IntoLabel;
use waterui_core::{AnyView, configurable};
use waterui_graphics::color::Color;

#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `ColorPicker` component.
pub struct ColorPickerConfig {
    /// The label of the color picker.
    pub label: AnyView,
    /// The binding to the color value.
    pub value: Binding<Color>,
    /// Whether to support alpha channel selection.
    pub support_alpha: bool,
    /// Whether to support HDR color selection.
    pub support_hdr: bool,
}

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
            support_alpha: false,
            support_hdr: false,
        })
    }

    /// Enables or disables alpha channel support.
    #[must_use]
    pub fn support_alpha(mut self, enable: bool) -> Self {
        self.0.support_alpha = enable;
        self
    }

    /// Enables or disables HDR color support.
    #[must_use]
    pub fn support_hdr(mut self, enable: bool) -> Self {
        self.0.support_hdr = enable;
        self
    }

    /// Sets the label of the color picker.
    #[must_use]
    pub fn label(mut self, label: impl IntoLabel) -> Self {
        self.0.label = AnyView::new(label.into_label());
        self
    }
}
