//! Radar/Spider chart component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;

use crate::charts::SignalRenderer;
use crate::data::RadarData;
use crate::renderer::RadarRenderer;

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
    pub const fn ring_count(mut self, count: u32) -> Self {
        self.ring_count = count;
        self
    }

    /// Sets the line width for outlines and grid.
    #[must_use]
    pub const fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    /// Sets the fill opacity for data polygons.
    #[must_use]
    pub const fn fill_opacity(mut self, opacity: f32) -> Self {
        self.fill_opacity = opacity;
        self
    }
}

impl<S: Signal<Output = RadarData> + Clone + 'static> View for RadarChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let renderer = RadarRenderer::new()
            .ring_count(self.ring_count)
            .line_width(self.line_width)
            .fill_opacity(self.fill_opacity);
        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
