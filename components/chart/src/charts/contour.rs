//! Contour chart component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;

use crate::charts::SignalRenderer;
use crate::data::ContourData;
use crate::renderer::ContourRenderer;

/// Contour chart for isoline visualization.
///
/// Renders isolines from a 2D scalar field using the marching squares algorithm.
/// Ideal for topographic maps, weather data, and scientific visualization.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{ContourChart, ContourData};
///
/// // Create a 10x10 grid with 5 contour levels
/// let mut values = Vec::with_capacity(100);
/// for r in 0..10 {
///     for c in 0..10 {
///         // Create a radial gradient pattern
///         let x = (c as f32 - 5.0) / 5.0;
///         let y = (r as f32 - 5.0) / 5.0;
///         values.push((x * x + y * y).sqrt());
///     }
/// }
/// let data = binding(ContourData::new(10, 10, values, 5));
///
/// ContourChart::new(data)
/// ```
pub struct ContourChart<S: Signal<Output = ContourData>> {
    data: S,
    line_width: f32,
}

impl<S: Signal<Output = ContourData>> ContourChart<S> {
    /// Creates a new contour chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            line_width: 2.0,
        }
    }

    /// Sets the line width for contour lines.
    #[must_use]
    pub fn line_width(mut self, width: f32) -> Self {
        assert!(
            width.is_finite() && width > 0.0,
            "ContourChart::line_width(width) requires width > 0"
        );
        self.line_width = width;
        self
    }
}

impl<S: Signal<Output = ContourData> + Clone + 'static> View for ContourChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let renderer = ContourRenderer::new().line_width(self.line_width);
        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
