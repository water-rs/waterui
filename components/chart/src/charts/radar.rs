//! Radar/Spider chart component.

use core::num::NonZeroU32;

use nami::Signal;
use waterui_core::{Environment, View};

use crate::charts::canvas::{draw_radar, signal_canvas};
use crate::data::RadarData;
use crate::params::{ChartParamError, PositiveF32, UnitInterval};

/// Radar/Spider chart for multivariate data visualization.
///
/// Displays data on radial axes emanating from a center point.
/// Each data series forms a polygon connecting values on each axis.
/// Ideal for comparing multiple metrics across categories.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{RadarChart, RadarData, RadarSeries};
///
/// let data = binding(RadarData::new(5)
///     .labels(vec!["Speed", "Power", "Range", "Defense", "Magic"])
///     .series(RadarSeries::new("Player 1", vec![80.0, 90.0, 70.0, 60.0, 85.0])
///         .color_hex("#3B82F6"))
///     .series(RadarSeries::new("Player 2", vec![70.0, 60.0, 90.0, 80.0, 75.0])
///         .color_hex("#EF4444"))
///     .max_value(100.0));
///
/// RadarChart::new(data)
/// ```
pub struct RadarChart<S: Signal<Output = RadarData>> {
    data: S,
    ring_count: u32,
    line_width: f32,
    fill_opacity: f32,
}

impl<S: Signal<Output = RadarData>> RadarChart<S> {
    /// Creates a new radar chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            ring_count: 5,
            line_width: 2.0,
            fill_opacity: 0.3,
        }
    }

    /// Sets the number of concentric grid rings.
    #[must_use]
    pub fn ring_count(self, count: u32) -> Self {
        self.try_ring_count(count)
            .expect("RadarChart::ring_count(count) requires count >= 1")
    }

    /// Sets ring count using a validated strong type.
    #[must_use]
    pub fn with_ring_count(mut self, count: NonZeroU32) -> Self {
        self.ring_count = count.get();
        self
    }

    /// Fallible variant of [`Self::ring_count`].
    pub fn try_ring_count(self, count: u32) -> Result<Self, ChartParamError> {
        let count = NonZeroU32::new(count).ok_or(ChartParamError::OutOfRange {
            param: "ring_count",
            value: count as f32,
            min: 1.0,
            max: u32::MAX as f32,
        })?;
        Ok(self.with_ring_count(count))
    }

    /// Sets the line width for outlines and grid.
    #[must_use]
    pub fn line_width(self, width: f32) -> Self {
        self.try_line_width(width)
            .expect("RadarChart::line_width(width) requires finite width > 0")
    }

    /// Sets line width using a validated strong type.
    #[must_use]
    pub fn with_line_width(mut self, width: PositiveF32) -> Self {
        self.line_width = width.get();
        self
    }

    /// Fallible variant of [`Self::line_width`].
    pub fn try_line_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_line_width(PositiveF32::try_new(width)?))
    }

    /// Sets the fill opacity for data polygons.
    #[must_use]
    pub fn fill_opacity(self, opacity: f32) -> Self {
        self.try_fill_opacity(opacity)
            .expect("RadarChart::fill_opacity(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    /// Sets fill opacity using a validated strong type.
    #[must_use]
    pub fn with_fill_opacity(mut self, opacity: UnitInterval) -> Self {
        self.fill_opacity = opacity.get();
        self
    }

    /// Fallible variant of [`Self::fill_opacity`].
    pub fn try_fill_opacity(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_fill_opacity(UnitInterval::try_new(opacity)?))
    }
}

impl<S: Signal<Output = RadarData> + Clone + 'static> View for RadarChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let ring_count = self.ring_count;
        let line_width = self.line_width;
        let fill_opacity = self.fill_opacity;
        signal_canvas(self.data, move |ctx, data| {
            draw_radar(ctx, data, ring_count, line_width, fill_opacity);
        })
    }
}
