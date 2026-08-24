//! Progress indicators for the `WaterUI` framework.
//!
//! This module provides components for showing progress indicators to users,
//! both as linear (bar) and circular progress indicators.
//!
//! # Examples
//!
//! ```
//! use waterui::prelude::*;
//!
//! // Create a basic linear progress indicator
//! let basic_progress = progress(0.75); // 75% complete
//!
//! // Create a circular progress indicator
//! let circular_progress = progress(0.5).circular();
//!
//! // Create a progress with custom label
//! let labeled_progress = progress(0.3).label("Downloading...");
//!
//! // Create an indeterminate loading indicator
//! let loading_indicator = Progress::infinity().circular();
//! ```

use crate::AnyView;
use crate::SignalExt;
use crate::ViewExt;
use nami::Computed;
use nami::signal::IntoComputed;
use waterui_controls::IntoLabel;
use waterui_core::View;
use waterui_core::configurable;
use waterui_core::layout::StretchAxis;
use waterui_macros::text;

fn progress_value_label(value: &Computed<f64>) -> Computed<String> {
    value
        .clone()
        .map(|value| {
            if value.is_finite() {
                format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0)
            } else {
                String::new()
            }
        })
        .computed()
}
/// Configuration for progress indicators.
///
/// Contains the visual and behavioral properties of a progress indicator.
#[non_exhaustive]
#[derive(Debug)]
pub struct ProgressConfig {
    /// The label displayed alongside the progress indicator.
    pub label: AnyView,
    /// The label displaying the numerical value of the progress.
    pub value_label: AnyView,
    /// The computed progress value between 0.0 and 1.0.
    pub value: Computed<f64>,
    /// The visual style of the progress indicator (linear or circular).
    pub style: ProgressStyle,
    /// Whether indeterminate progress uses the Material four-color cycle.
    pub four_color: bool,
}

/// Visual style options for progress indicators.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum ProgressStyle {
    /// A circular spinner-style progress indicator.
    Circular,
    /// A linear bar-style progress indicator.
    Linear,
    /// Material's loading indicator: a shape that morphs and rotates while work
    /// is in flight.
    ///
    /// This is a style rather than its own component because it is the same
    /// semantic thing as any other indeterminate indicator — work is happening,
    /// and how long it will take is unknown. It carries no determinate reading:
    /// a value set on a loading-style progress is used for accessibility but is
    /// not drawn, because the indicator has no track to fill.
    Loading,
}

configurable!(
    /// A view that shows the progress of a task.
    ///
    /// Progress can be displayed as a linear bar or circular spinner.
    ///
    /// # Layout Behavior
    ///
    /// - **Linear style:** Expands horizontally to fill available space, fixed height
    /// - **Circular style:** Sizes itself to fit the spinner, never stretches
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Determinate progress (75% complete)
    /// progress(0.75)
    ///
    /// // Circular spinner
    /// progress(0.5).circular()
    ///
    /// // Indeterminate loading spinner
    /// loading()
    ///
    /// // With custom label
    /// progress(0.3).label("Downloading...")
    /// ```
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Linear style:
    //   Stretch Axis: `Horizontal` - Expands to fill available width.
    //   Height: Fixed intrinsic (platform-determined track height)
    //
    // Circular and Loading styles:
    //   Stretch Axis: `None` - Content-sized, does not expand.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Progress,
    ProgressConfig,
    |config| match config.style {
        ProgressStyle::Linear => StretchAxis::Horizontal,
        ProgressStyle::Circular | ProgressStyle::Loading => StretchAxis::None,
    }
);

/// A progress indicator with a calculated total.
///
/// Created by calling `total()` on a `Progress` instance.
#[derive(Debug)]
pub struct ProgressWithTotal(Progress);

impl ProgressWithTotal {
    /// Sets a custom label for the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `label` - The view to use as the progress label.
    #[must_use]
    pub fn label(self, label: impl IntoLabel) -> Self {
        Self(self.0.label(label))
    }

    /// Changes the progress indicator to a circular style.
    #[must_use]
    pub fn circular(self) -> Self {
        Self(self.0.circular())
    }

    /// Changes the progress indicator to a linear style.
    #[must_use]
    pub fn linear(self) -> Self {
        Self(self.0.linear())
    }
}

impl View for ProgressWithTotal {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.0
    }
}

impl Progress {
    /// Creates a new progress indicator with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The progress value between 0.0 and 1.0.
    pub fn new(value: impl IntoComputed<f64>) -> Self {
        let value = value.into_signal().computed();
        let value_label = progress_value_label(&value);
        Self(ProgressConfig {
            label: text!("Please wait...").anyview(),
            value_label: text!("{value_label}").anyview(),
            value,
            style: ProgressStyle::Linear,
            four_color: false,
        })
    }

    /// Creates a progress indicator that calculates its value as a fraction of the total.
    ///
    /// # Arguments
    ///
    /// * `total` - The total value against which progress is measured.
    pub fn total(mut self, total: impl IntoComputed<f64>) -> ProgressWithTotal {
        let total = total.into_signal();
        let value = self.0.value;
        self.0.value = total.zip(&value).map(|(t, v)| v / t).computed();
        let value_label = progress_value_label(&self.0.value);
        self.0.value_label = text!("{value_label}").anyview();

        ProgressWithTotal(self)
    }

    /// Creates an infinite progress indicator, typically shown as an indeterminate spinner.
    /// Note: This uses `f32::INFINITY` as the progress value.
    #[must_use]
    pub fn infinity() -> Self {
        Self::new(f32::INFINITY)
    }

    /// Sets a custom label for the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `label` - The view to use as the progress label.
    #[must_use]
    pub fn label(mut self, label: impl IntoLabel) -> Self {
        self.0.label = label.into_label().anyview();
        self
    }

    /// Changes the visual style of the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `style` - The style to apply to the progress indicator.
    const fn style(mut self, style: ProgressStyle) -> Self {
        self.0.style = style;
        self
    }

    /// Changes the progress indicator to a circular style.
    #[must_use]
    pub const fn circular(self) -> Self {
        self.style(ProgressStyle::Circular)
    }

    /// Changes the progress indicator to a linear style.
    #[must_use]
    pub const fn linear(self) -> Self {
        self.style(ProgressStyle::Linear)
    }

    /// Changes the progress indicator to the platform's loading indicator.
    ///
    /// This says "content is on its way", where the circular and linear styles
    /// report how far along an operation is. Each backend draws its own: the
    /// Material theme morphs a shape while it turns, and platforms whose
    /// loading indicator is a spinner draw that.
    #[must_use]
    pub const fn loading(self) -> Self {
        self.style(ProgressStyle::Loading)
    }

    /// Render indeterminate progress with the Material four-color cycle.
    #[must_use]
    pub const fn four_color(mut self) -> Self {
        self.0.four_color = true;
        self
    }
}

/// Creates a new progress indicator with the specified value.
///
/// # Arguments
///
/// * `value` - The progress value between 0.0 and 1.0.
pub fn progress(value: impl IntoComputed<f64>) -> Progress {
    Progress::new(value)
}

/// Creates the platform's indeterminate loading indicator.
#[must_use]
pub fn loading() -> Progress {
    Progress::infinity().loading()
}

#[cfg(test)]
mod tests {
    use nami::{Computed, Signal as _};

    use super::progress_value_label;

    #[test]
    fn progress_value_label_formats_fraction_as_percent() {
        assert_eq!(progress_value_label(&Computed::constant(0.42)).get(), "42%");
        assert_eq!(
            progress_value_label(&Computed::constant(0.755)).get(),
            "76%"
        );
    }

    #[test]
    fn progress_value_label_clamps_to_valid_progress_range() {
        assert_eq!(progress_value_label(&Computed::constant(-0.25)).get(), "0%");
        assert_eq!(
            progress_value_label(&Computed::constant(1.25)).get(),
            "100%"
        );
    }
}
