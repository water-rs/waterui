//! Axis configuration and tick computation for charts.
//!
//! Uses the "nice numbers" algorithm to compute aesthetically pleasing
//! tick values (e.g., 0, 20, 40, 60, 80, 100 instead of 0, 17, 34, 51, 68, 85).

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use num_traits::ToPrimitive as _;
use waterui_core::Str;
use waterui_text::{IntoText, Text};

/// Configuration for a chart axis.
///
/// Use the builder pattern to configure tick count, format, and grid lines.
///
/// # Example
///
/// ```rust
/// use waterui_chart::{AxisConfig, TickFormat};
///
/// let config = AxisConfig::new()
///     .tick_count(5)
///     .format(TickFormat::SI)
///     .show_grid()
///     .label("Revenue ($)");
/// ```
#[derive(Clone)]
pub struct AxisConfig {
    tick_count: usize,
    format: TickFormat,
    show_grid: bool,
    label: Option<Text>,
}

impl core::fmt::Debug for AxisConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AxisConfig")
            .field("tick_count", &self.tick_count)
            .field("format", &self.format)
            .field("show_grid", &self.show_grid)
            .field("label", &self.label.as_ref().map(|_| "Text"))
            .finish()
    }
}

impl AxisConfig {
    /// Creates a new axis config with defaults.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tick_count: 5,
            format: TickFormat::Auto,
            show_grid: false,
            label: None,
        }
    }

    /// Sets approximate number of ticks (algorithm may adjust for nice values).
    #[must_use]
    pub const fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = count;
        self
    }

    /// Sets the tick label format.
    #[must_use]
    pub fn format(mut self, format: TickFormat) -> Self {
        self.format = format;
        self
    }

    /// Enables grid lines at tick positions.
    #[must_use]
    pub const fn show_grid(mut self) -> Self {
        self.show_grid = true;
        self
    }

    /// Sets axis label (e.g., "Price ($)").
    #[must_use]
    pub fn label(mut self, label: impl IntoText) -> Self {
        self.label = Some(label.into_text());
        self
    }

    /// Returns true if grid lines should be shown.
    #[must_use]
    pub const fn has_grid(&self) -> bool {
        self.show_grid
    }

    /// Returns the axis label, if set.
    #[must_use]
    pub const fn axis_label(&self) -> Option<&Text> {
        self.label.as_ref()
    }

    /// Computes nice tick values for given bounds.
    #[must_use]
    pub fn compute_ticks(&self, min: f32, max: f32) -> Vec<Tick> {
        // Handle edge cases
        if min >= max || !min.is_finite() || !max.is_finite() {
            return Vec::new();
        }

        // Use nice numbers algorithm
        let range = nice_number(max - min, false);
        let Some(intervals) = (self.tick_count.max(2) - 1).to_f32() else {
            return Vec::new();
        };
        let step = nice_number(range / intervals, true);

        // Avoid division by zero
        if step <= 0.0 || !step.is_finite() {
            return Vec::new();
        }

        let nice_min = (min / step).floor() * step;
        let nice_max = (max / step).ceil() * step;

        let mut ticks = Vec::new();
        let mut value = nice_min;

        // Safety limit to prevent infinite loops
        let max_ticks = 100;

        while value <= step.mul_add(0.5, nice_max) && ticks.len() < max_ticks {
            // Normalize position to [0, 1] range
            let position = (value - min) / (max - min);

            // Only include ticks that are within or near the visible range
            if (-0.01..=1.01).contains(&position) {
                ticks.push(Tick {
                    value,
                    label: self.format_tick(value, step),
                    position: position.clamp(0.0, 1.0),
                });
            }

            value += step;
        }

        ticks
    }

    /// Formats a tick value based on the configured format.
    fn format_tick(&self, value: f32, step: f32) -> Str {
        match &self.format {
            TickFormat::Auto => format_auto(value, step),
            TickFormat::Fixed(decimals) => format!("{:.1$}", value, *decimals).into(),
            TickFormat::SI => format_si(value),
            TickFormat::Percent => format!("{:.0}%", value * 100.0).into(),
            TickFormat::Custom(formatter) => formatter(value),
        }
    }
}

impl Default for AxisConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Format for tick labels.
#[derive(Clone)]
pub enum TickFormat {
    /// Auto-detect based on value magnitude and step size.
    Auto,
    /// Fixed number of decimal places.
    Fixed(usize),
    /// SI prefixes (1K, 1M, 1B, etc.).
    SI,
    /// Percentage (value * 100 + %).
    Percent,
    /// Custom formatter function.
    Custom(Arc<dyn Fn(f32) -> Str + Send + Sync>),
}

impl core::fmt::Debug for TickFormat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Auto => f.write_str("Auto"),
            Self::Fixed(decimals) => f.debug_tuple("Fixed").field(decimals).finish(),
            Self::SI => f.write_str("SI"),
            Self::Percent => f.write_str("Percent"),
            Self::Custom(_) => f.write_str("Custom(..)"),
        }
    }
}

/// A computed tick mark.
#[derive(Debug, Clone)]
pub struct Tick {
    value: f32,
    label: Str,
    /// Position in normalized [0, 1] space.
    position: f32,
}

impl Tick {
    /// Returns the tick value.
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// Returns the formatted label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the normalized position (0.0 to 1.0).
    #[must_use]
    pub const fn position(&self) -> f32 {
        self.position
    }
}

/// Computes a "nice" number for axis scaling.
///
/// A nice number is one of: 1, 2, 5, 10, 20, 50, 100, etc.
/// This produces aesthetically pleasing axis tick values.
fn nice_number(x: f32, round: bool) -> f32 {
    if x <= 0.0 || !x.is_finite() {
        return 1.0;
    }

    let exp = x.log10().floor();
    let frac = x / 10_f32.powf(exp);

    let nice_frac = if round {
        if frac < 1.5 {
            1.0
        } else if frac < 3.0 {
            2.0
        } else if frac < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };

    nice_frac * 10_f32.powf(exp)
}

/// Auto-formats a value based on its magnitude and step size.
fn format_auto(value: f32, step: f32) -> Str {
    // Determine decimal places based on step size
    let decimals = if step >= 1.0 {
        0
    } else {
        let log = (-step.log10())
            .ceil()
            .to_usize()
            .expect("finite positive step logarithm should fit in usize");
        log.min(6) // Cap at 6 decimal places
    };

    // Clean up floating point artifacts
    let rounded = (value / step).round() * step;

    if decimals == 0 {
        format!("{rounded:.0}").into()
    } else {
        format!("{rounded:.decimals$}").into()
    }
}

/// Formats a value with SI prefixes (K, M, B, T).
fn format_si(value: f32) -> Str {
    let abs = value.abs();

    if abs >= 1_000_000_000_000.0 {
        format!("{:.1}T", value / 1_000_000_000_000.0).into()
    } else if abs >= 1_000_000_000.0 {
        format!("{:.1}B", value / 1_000_000_000.0).into()
    } else if abs >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0).into()
    } else if abs >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0).into()
    } else if abs >= 1.0 {
        format!("{value:.0}").into()
    } else {
        format!("{value:.2}").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() <= f32::EPSILON);
    }

    #[test]
    fn test_nice_number() {
        assert_close(nice_number(0.15, true), 0.2);
        assert_close(nice_number(0.7, true), 1.0);
        assert_close(nice_number(2.5, true), 2.0); // frac=2.5 < 3.0 -> 2.0
        assert_close(nice_number(3.0, true), 5.0); // frac=3.0 >= 3.0, < 7.0 -> 5.0
        assert_close(nice_number(7.0, true), 10.0);
        assert_close(nice_number(150.0, true), 200.0);
    }

    #[test]
    fn test_compute_ticks() {
        let config = AxisConfig::new().tick_count(5);
        let ticks = config.compute_ticks(0.0, 100.0);

        // Should have nice round numbers
        assert!(!ticks.is_empty());
        assert!(ticks.iter().all(|t| {
            let by_twenty = (t.value() % 20.0).abs() <= f32::EPSILON;
            let by_twenty_five = (t.value() % 25.0).abs() <= f32::EPSILON;
            by_twenty || by_twenty_five
        }));
    }

    #[test]
    fn test_format_si() {
        assert_eq!(format_si(1500.0), "1.5K");
        assert_eq!(format_si(2_500_000.0), "2.5M");
        assert_eq!(format_si(1_000_000_000.0), "1.0B");
    }

    #[test]
    fn test_empty_range() {
        let config = AxisConfig::new();
        let ticks = config.compute_ticks(50.0, 50.0);
        assert!(ticks.is_empty());
    }
}
