//! Color Picker Component

use nami::Binding;
use waterui_controls::label::Label;
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::configurable;
use waterui_graphics::color::Color;

#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `ColorPicker` component.
pub struct ColorPickerConfig {
    /// The label of the color picker.
    pub label: Label,
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
    /// Creates a new `ColorPicker` with the given semantic label and value
    /// binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(label: impl IntoLabel, value: &Binding<Color>) -> Self {
        Self(ColorPickerConfig {
            label: label.into_label(),
            value: value.clone(),
            support_alpha: false,
            support_hdr: false,
        })
    }

    /// Enables alpha channel support.
    #[must_use]
    pub const fn with_alpha(mut self) -> Self {
        self.0.support_alpha = true;
        self
    }

    /// Enables HDR color support.
    #[must_use]
    pub const fn with_hdr(mut self) -> Self {
        self.0.support_hdr = true;
        self
    }
}

impl_label_style_methods!(ColorPicker);
