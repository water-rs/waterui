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
use nami::Computed;
use nami::signal::IntoComputed;
use waterui_controls::{IntoLabel, Label};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{Environment, Native, NativeView, View};
use waterui_macros::text;
use waterui_text::Text;

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
#[derive(Debug, Clone)]
pub struct ProgressConfig<S> {
    /// The label displayed alongside the progress indicator.
    label: S,
    /// The label displaying the numerical value of the progress.
    value_label: S,
    /// The computed progress value between 0.0 and 1.0.
    value: Computed<f64>,
    /// The visual style of the progress indicator (linear or circular).
    style: ProgressStyle,
    /// Whether indeterminate progress uses the Material four-color cycle.
    four_color: bool,
}

impl<S> ProgressConfig<S> {
    /// Takes the value apart into its fields, in declaration order.
    pub fn into_parts(self) -> (S, S, Computed<f64>, ProgressStyle, bool) {
        (
            self.label,
            self.value_label,
            self.value,
            self.style,
            self.four_color,
        )
    }

    /// Creates a `ProgressConfig` from its parts.
    #[must_use]
    pub const fn new(
        label: S,
        value_label: S,
        value: Computed<f64>,
        style: ProgressStyle,
        four_color: bool,
    ) -> Self {
        Self {
            label,
            value_label,
            value,
            style,
            four_color,
        }
    }

    /// The label displayed alongside the progress indicator.
    #[must_use]
    pub const fn label(&self) -> &S {
        &self.label
    }

    /// The label displaying the numerical value of the progress.
    #[must_use]
    pub const fn value_label(&self) -> &S {
        &self.value_label
    }

    /// The computed progress value between 0.0 and 1.0.
    #[must_use]
    pub const fn value(&self) -> &Computed<f64> {
        &self.value
    }

    /// The visual style of the progress indicator (linear or circular).
    #[must_use]
    pub const fn style(&self) -> ProgressStyle {
        self.style
    }

    /// Whether indeterminate progress uses the Material four-color cycle.
    #[must_use]
    pub const fn four_color(&self) -> bool {
        self.four_color
    }
}

/// Visual style options for progress indicators.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl<S: 'static> NativeView for ProgressConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        match self.style {
            ProgressStyle::Linear => StretchAxis::Horizontal,
            ProgressStyle::Circular | ProgressStyle::Loading => StretchAxis::None,
        }
    }
}

impl<S: 'static> MapSlots<S> for ProgressConfig<S> {
    type With<T: 'static> = ProgressConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ProgressConfig<T> {
        ProgressConfig {
            label: f(self.label),
            value_label: f(self.value_label),
            value: self.value,
            style: self.style,
            four_color: self.four_color,
        }
    }
}

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
/// use waterui::prelude::*;
/// use waterui::component::progress::{loading, progress};
///
/// // Determinate progress (75% complete)
/// let bar = progress(0.75);
///
/// // Circular spinner
/// let ring = progress(0.5).circular();
///
/// // Indeterminate loading spinner
/// let spinner = loading();
///
/// // With custom label
/// let labelled = progress(0.3).label("Downloading...");
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct Progress<L, V> {
    label: L,
    value_label: V,
    value: Computed<f64>,
    style: ProgressStyle,
    four_color: bool,
}

/// A progress indicator with a calculated total.
///
/// Created by calling `total()` on a `Progress` instance.
#[derive(Debug, Clone)]
#[must_use]
pub struct ProgressWithTotal<L, V>(Progress<L, V>);

impl<L, V> ProgressWithTotal<L, V> {
    /// Sets a custom label for the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `label` - The view to use as the progress label.
    pub fn label<NewL: IntoLabel>(self, label: NewL) -> ProgressWithTotal<Label<NewL::Kind>, V> {
        ProgressWithTotal(self.0.label(label))
    }

    /// Changes the progress indicator to a circular style.
    pub const fn circular(mut self) -> Self {
        self.0.style = ProgressStyle::Circular;
        self
    }

    /// Changes the progress indicator to a linear style.
    pub const fn linear(mut self) -> Self {
        self.0.style = ProgressStyle::Linear;
        self
    }
}

impl<L: View, V: View> View for ProgressWithTotal<L, V> {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.0.dispatch(backend, env, cx)
    }
}

impl Progress<Text, Text> {
    /// Creates a new progress indicator with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The progress value between 0.0 and 1.0.
    pub fn new(value: impl IntoComputed<f64>) -> Self {
        let value = value.into_signal().computed();
        let value_label = progress_value_label(&value);
        Self {
            label: text!("Please wait..."),
            value_label: text!("{value_label}"),
            value,
            style: ProgressStyle::Linear,
            four_color: false,
        }
    }

    /// Creates an infinite progress indicator, typically shown as an indeterminate spinner.
    /// Note: This uses `f32::INFINITY` as the progress value.
    pub fn infinity() -> Self {
        Self::new(f32::INFINITY)
    }
}

impl<L, V> Progress<L, V> {
    /// Creates a progress indicator that calculates its value as a fraction of the total.
    ///
    /// # Arguments
    ///
    /// * `total` - The total value against which progress is measured.
    pub fn total(self, total: impl IntoComputed<f64>) -> ProgressWithTotal<L, Text> {
        let total = total.into_signal();
        let value = total.zip(&self.value).map(|(t, v)| v / t).computed();
        let value_label = progress_value_label(&value);
        ProgressWithTotal(Progress {
            label: self.label,
            value_label: text!("{value_label}"),
            value,
            style: self.style,
            four_color: self.four_color,
        })
    }

    /// Sets a custom label for the progress indicator.
    ///
    /// # Arguments
    ///
    /// * `label` - The view to use as the progress label.
    pub fn label<NewL: IntoLabel>(self, label: NewL) -> Progress<Label<NewL::Kind>, V> {
        Progress {
            label: label.into_label(),
            value_label: self.value_label,
            value: self.value,
            style: self.style,
            four_color: self.four_color,
        }
    }

    /// Changes the visual style of the progress indicator.
    const fn style(mut self, style: ProgressStyle) -> Self {
        self.style = style;
        self
    }

    /// Changes the progress indicator to a circular style.
    pub const fn circular(self) -> Self {
        self.style(ProgressStyle::Circular)
    }

    /// Changes the progress indicator to a linear style.
    pub const fn linear(self) -> Self {
        self.style(ProgressStyle::Linear)
    }

    /// Changes the progress indicator to the platform's loading indicator.
    ///
    /// This says "content is on its way", where the circular and linear styles
    /// report how far along an operation is. Each backend draws its own: the
    /// Material theme morphs a shape while it turns, and platforms whose
    /// loading indicator is a spinner draw that.
    pub const fn loading(self) -> Self {
        self.style(ProgressStyle::Loading)
    }

    /// Render indeterminate progress with the Material four-color cycle.
    pub const fn four_color(mut self) -> Self {
        self.four_color = true;
        self
    }
}

impl<L: View, V: View> Progress<L, V> {
    fn into_config<S: ChildSlot>(self) -> ProgressConfig<S> {
        ProgressConfig {
            label: S::from_view(self.label),
            value_label: S::from_view(self.value_label),
            value: self.value,
            style: self.style,
            four_color: self.four_color,
        }
    }
}

impl<S: View> Progress<S, S> {
    /// Returns the configuration for this progress view.
    #[must_use]
    pub fn config(self) -> ProgressConfig<S> {
        ProgressConfig {
            label: self.label,
            value_label: self.value_label,
            value: self.value,
            style: self.style,
            four_color: self.four_color,
        }
    }
}

impl<S: View> From<ProgressConfig<S>> for Progress<S, S> {
    fn from(value: ProgressConfig<S>) -> Self {
        Self {
            label: value.label,
            value_label: value.value_label,
            value: value.value,
            style: value.style,
            four_color: value.four_color,
        }
    }
}

impl<L: View, V: View> View for Progress<L, V> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>())
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(), env, cx)
    }
}

/// Creates a new progress indicator with the specified value.
///
/// # Arguments
///
/// * `value` - The progress value between 0.0 and 1.0.
pub fn progress(value: impl IntoComputed<f64>) -> Progress<Text, Text> {
    Progress::new(value)
}

/// Creates the platform's indeterminate loading indicator.
pub fn loading() -> Progress<Text, Text> {
    Progress::infinity().loading()
}

#[cfg(test)]
mod tests {
    use nami::{Computed, Signal as _};

    use super::progress_value_label;

    #[test]
    fn progress_value_label_formats_fraction_as_percent() {
        assert_eq!(
            progress_value_label(&Computed::constant(0.42)).snapshot(),
            "42%"
        );
        assert_eq!(
            progress_value_label(&Computed::constant(0.755)).snapshot(),
            "76%"
        );
    }

    #[test]
    fn progress_value_label_clamps_to_valid_progress_range() {
        assert_eq!(
            progress_value_label(&Computed::constant(-0.25)).snapshot(),
            "0%"
        );
        assert_eq!(
            progress_value_label(&Computed::constant(1.25)).snapshot(),
            "100%"
        );
    }
}
