//! Choropleth chart view component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;
use waterui_graphics::GpuSurface;

use crate::data::ChoroplethData;
use crate::renderer::ChoroplethRenderer;

use super::SignalRenderer;

/// Choropleth map chart for geographic data visualization.
///
/// Displays regions colored by data values, ideal for showing
/// geographic distributions like population density, election results, etc.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{ChoroplethChart, ChoroplethData, GeoPolygon};
///
/// let data = binding(ChoroplethData::new(vec![
///     GeoPolygon::new(0, 100.0, vec![[-122.0, 37.0], [-121.0, 37.0], [-121.0, 38.0], [-122.0, 38.0]]),
///     GeoPolygon::new(1, 200.0, vec![[-121.0, 37.0], [-120.0, 37.0], [-120.0, 38.0], [-121.0, 38.0]]),
/// ]));
///
/// ChoroplethChart::new(&data)
///     .stroke_width(2.0)
///     .stroke_color(Srgb::new(1.0, 1.0, 1.0))
/// ```
pub struct ChoroplethChart<S: Signal<Output = ChoroplethData>> {
    data: S,
    stroke_width: f32,
    stroke_color: Srgb,
    show_stroke: bool,
}

impl<S: Signal<Output = ChoroplethData>> ChoroplethChart<S> {
    /// Creates a new choropleth chart with the given reactive data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            stroke_width: 1.0,
            stroke_color: Srgb::new(1.0, 1.0, 1.0),
            show_stroke: true,
        }
    }

    /// Sets the stroke width for polygon borders.
    #[must_use]
    pub const fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = width;
        self
    }

    /// Sets the stroke color for polygon borders.
    #[must_use]
    pub fn stroke_color(mut self, color: Srgb) -> Self {
        self.stroke_color = color;
        self
    }

    /// Sets whether to show polygon borders.
    #[must_use]
    pub const fn show_stroke(mut self, show: bool) -> Self {
        self.show_stroke = show;
        self
    }
}

impl<S: Signal<Output = ChoroplethData> + Clone + 'static> View for ChoroplethChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let renderer = ChoroplethRenderer::new()
            .stroke_width(self.stroke_width)
            .stroke_color(self.stroke_color)
            .show_stroke(self.show_stroke);
        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
