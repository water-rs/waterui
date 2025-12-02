//! Slider control for adjusting numeric values within a range.
//!
//! This module provides a `Slider` widget that allows users to select a value
//! within a specified numeric range by sliding a handle along a track.

use core::ops::RangeInclusive;

use nami::Binding;
use waterui_core::{AnyView, NativeView, View, configurable, layout::StretchAxis};
use waterui_text::text;

/// Configuration for the [`Slider`] widget.
#[derive(Debug)]
#[non_exhaustive]
pub struct SliderConfig {
    /// The label to display for the slider.
    pub label: AnyView,
    /// The label for the minimum value of the slider.
    pub min_value_label: AnyView,
    /// The label for the maximum value of the slider.
    pub max_value_label: AnyView,
    /// The range of values the slider can take.
    pub range: RangeInclusive<f64>,
    /// The binding to the current value of the slider.
    pub value: Binding<f64>,
}

impl NativeView for SliderConfig {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

configurable!(
    /// A control for selecting a value from a continuous range.
    ///
    /// Slider lets users select a value by dragging a thumb along a track.
    ///
    /// # Layout Behavior
    ///
    /// Slider **expands horizontally** to fill available space, but has a fixed height.
    /// In an `HStack`, it will take up all remaining width after other views are sized.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Basic slider (0 to 100)
    /// slider(0.0..=100.0, &volume)
    ///
    /// // With custom labels
    /// slider(0.0..=1.0, &brightness)
    ///     .label("Brightness")
    ///     .min_value_label("Dark")
    ///     .max_value_label("Bright")
    ///
    /// // In a form (slider fills remaining width)
    /// hstack((
    ///     text("Volume"),
    ///     slider(0.0..=100.0, &volume),
    /// ))
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Stretch Axis: `Horizontal` - Expands to fill available width.
    // Height: Fixed intrinsic (platform-determined)
    // Width: Reports minimum usable width, expands during layout phase
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Slider,
    SliderConfig
);

impl Slider {
    /// Creates a new [`Slider`] widget.
    #[must_use]
    pub fn new(range: RangeInclusive<f64>, value: &Binding<f64>) -> Self {
        Self(SliderConfig {
            label: AnyView::new(text!("{:.2}", value)),
            min_value_label: AnyView::default(),
            max_value_label: AnyView::default(),
            range,
            value: value.clone(),
        })
    }
}

macro_rules! labels {
    ($($name:ident),*) => {
        $(
            #[must_use]
            /// Sets the label for the slider.
            pub fn $name(mut self, $name: impl View) -> Self {
                self.0.$name = AnyView::new($name);
                self
            }
        )*
    };
}

impl Slider {
    labels!(label, min_value_label, max_value_label);
}

/// Creates a new [`Slider`] with the specified range and value binding.
#[must_use]
pub fn slider(range: RangeInclusive<f64>, value: &Binding<f64>) -> Slider {
    Slider::new(range, value)
}
