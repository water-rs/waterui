//! Slider control for adjusting numeric values within a range.
//!
//! This module provides a `Slider` widget that allows users to select a value
//! within a specified numeric range by sliding a handle along a track.
//!
//! ![Slider](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/slider.svg)

use core::ops::RangeInclusive;

use crate::label::{IntoLabel, Label, impl_label_style_methods};
use nami::Binding;
use waterui_core::{AnyView, Environment, configurable, layout::StretchAxis};

/// Configuration for the [`Slider`] widget.
#[derive(Debug)]
#[non_exhaustive]
pub struct SliderConfig {
    /// The label displayed for the slider.
    pub label: Label,
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
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::slider;
    /// # fn basic(volume: Binding<f64>) -> impl View {
    /// // Basic slider (label required for accessibility)
    /// slider("Volume", &volume).range(0.0..=100.0)
    /// # }
    ///
    /// # fn labelled_track(brightness: Binding<f64>) -> impl View {
    /// // With end-of-track value labels
    /// slider("Brightness", &brightness)
    ///     .min_value_label("Dark")
    ///     .max_value_label("Bright")
    /// # }
    ///
    /// # fn hidden(volume: Binding<f64>) -> impl View {
    /// // Visually hidden label, still announced by VoiceOver / TalkBack
    /// slider("Volume", &volume).hide_label()
    /// # }
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
    StretchAxis::Horizontal,
    resolve |config, env| config.resolve(env)
);

impl SliderConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        self
    }
}

impl Slider {
    /// Creates a new [`Slider`] with the given semantic label, bound to `value`.
    ///
    /// The default range is `0.0..=1.0`; use [`Slider::range`] to override.
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(label: Label, value: &Binding<f64>) -> Self {
        Self(SliderConfig {
            label,
            min_value_label: AnyView::default(),
            max_value_label: AnyView::default(),
            range: 0.0..=1.0,
            value: value.clone(),
        })
    }

    /// Sets the inclusive value range for this slider.
    #[must_use]
    pub const fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.0.range = range;
        self
    }

    /// Sets the label rendered next to the minimum end of the track.
    #[must_use]
    pub fn min_value_label(mut self, label: impl IntoLabel) -> Self {
        self.0.min_value_label = AnyView::new(label.into_label());
        self
    }

    /// Sets the label rendered next to the maximum end of the track.
    #[must_use]
    pub fn max_value_label(mut self, label: impl IntoLabel) -> Self {
        self.0.max_value_label = AnyView::new(label.into_label());
        self
    }
}

impl_label_style_methods!(Slider);

/// Convenience constructor for a [`Slider`] with the given label and value
/// binding, defaulting to the normalized range `0.0..=1.0`.
#[must_use]
pub fn slider(label: impl IntoLabel, value: &Binding<f64>) -> Slider {
    Slider::new(label.into_label(), value)
}
