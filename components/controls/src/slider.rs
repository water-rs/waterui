//! Slider control for adjusting numeric values within a range.
//!
//! This module provides a `Slider` widget that allows users to select a value
//! within a specified numeric range by sliding a handle along a track.
//!
//! ![Slider](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/slider.svg)

use core::ops::RangeInclusive;

use crate::label::IntoLabel;
use nami::{Binding, s};
use waterui_core::{AnyView, configurable, layout::StretchAxis};
use waterui_text::Text;

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
    /// slider(&volume).range(0.0..=100.0)
    ///
    /// // With custom labels (default range 0.0..=1.0)
    /// slider(&brightness)
    ///     .label("Brightness")
    ///     .min_value_label("Dark")
    ///     .max_value_label("Bright")
    ///
    /// // In a form (slider fills remaining width)
    /// hstack((
    ///     text("Volume"),
    ///     slider(&volume).range(0.0..=100.0),
    /// ))
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //

    // Height: Fixed intrinsic (platform-determined)
    // Width: Reports minimum usable width, expands during layout phase
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Slider,
    SliderConfig,
    StretchAxis::Horizontal
);

impl Slider {
    /// Creates a new [`Slider`] bound to `value` with the default
    /// normalized range `0.0..=1.0`. Use [`Slider::range`] to override.
    #[must_use]
    pub fn new(value: &Binding<f64>) -> Self {
        Self(SliderConfig {
            label: AnyView::new(Text::computed(s!("{:.2}", value))),
            min_value_label: AnyView::default(),
            max_value_label: AnyView::default(),
            range: 0.0..=1.0,
            value: value.clone(),
        })
    }

    /// Sets the inclusive value range for this slider.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.0.range = range;
        self
    }
}

macro_rules! labels {
    ($($name:ident),*) => {
        $(
            #[must_use]
            /// Sets the label for the slider.
            pub fn $name(mut self, $name: impl IntoLabel) -> Self {
                self.0.$name = AnyView::new($name.into_label());
                self
            }
        )*
    };
}

impl Slider {
    labels!(label, min_value_label, max_value_label);
}

/// Convenience constructor for a [`Slider`] bound to `value` with the default
/// normalized range `0.0..=1.0`. Chain [`Slider::range`] to override.
#[must_use]
pub fn slider(value: &Binding<f64>) -> Slider {
    Slider::new(value)
}
