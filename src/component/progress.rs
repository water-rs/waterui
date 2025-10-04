//! Progress indicators for the `WaterUI` framework.
//!
//! This module provides components for showing progress indicators to users,
//! both as linear (bar) and circular progress indicators.
//!
//! # Examples
//!
//! ```
//! use waterui::progress;
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
//! let loading_indicator = loading();
//! ```

use crate::AnyView;
use crate::SignalExt;
use crate::ViewExt;
use nami::Computed;
use nami::signal::IntoComputed;
use nami::zip::FlattenMap;
use waterui_core::View;
use waterui_core::configurable;
use waterui_text::text;

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
}

/// Visual style options for progress indicators.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum ProgressStyle {
    /// A circular spinner-style progress indicator.
    Circular,
    /// A linear bar-style progress indicator.
    Linear,
}

configurable!(Progress, ProgressConfig);

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
    pub fn label(self, label: impl View) -> Self {
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
        let value = value.into_computed();
        Self(ProgressConfig {
            label: text!("Please wait...").anyview(),
            value_label: text!("{value:.2} %").anyview(),
            value,
            style: ProgressStyle::Linear,
        })
    }

    /// Creates a progress indicator that calculates its value as a fraction of the total.
    ///
    /// # Arguments
    ///
    /// * `total` - The total value against which progress is measured.
    pub fn total(mut self, total: impl IntoComputed<f64>) -> ProgressWithTotal {
        self.0.value = total
            .into_signal()
            .zip(self.0.value)
            .flatten_map(|total, value| value / total)
            .computed();

        ProgressWithTotal(self)
    }

    /// Creates an infinite progress indicator, typically shown as an indeterminate spinner.
    #[must_use]
    pub fn infinity() -> Self {
        Self::new(f64::NAN)
    }

    /// Sets a custom label for the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `label` - The view to use as the progress label.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = label.anyview();
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
}

/// Creates a new progress indicator with the specified value.
///
/// # Arguments
///
/// * `value` - The progress value between 0.0 and 1.0.
pub fn progress(value: impl IntoComputed<f64>) -> Progress {
    Progress::new(value)
}

/// Creates an indeterminate loading indicator displayed as a circular spinner.
#[must_use]
pub fn loading() -> Progress {
    Progress::infinity().circular()
}
