//! Pie chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;
use waterui_graphics::color::Srgb;

use crate::charts::SignalRenderer;
use crate::data::DataPoint;
use crate::renderer::PieChartRenderer;

/// Pie chart visualization.
///
/// Renders data as circular sectors with optional donut hole.
/// Supports smooth animations and hover interactions.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui_chart::{PieChart, DataPoint};
///
/// let data = binding(vec![
///     DataPoint::new(0.0, 30.0),  // 30%
///     DataPoint::new(1.0, 45.0),  // 45%
///     DataPoint::new(2.0, 25.0),  // 25%
/// ]);
///
/// PieChart::new(&data)
///     .donut(0.5)  // 50% inner radius for donut chart
/// ```
pub struct PieChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    colors: Vec<Srgb>,
    inner_radius: f32,
}

impl<S: Signal<Output = Vec<DataPoint>>> PieChart<S> {
    /// Creates a new pie chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            colors: Vec::new(),
            inner_radius: 0.0,
        }
    }

    /// Sets custom colors for slices.
    #[must_use]
    pub fn colors(mut self, colors: Vec<Srgb>) -> Self {
        self.colors = colors;
        self
    }

    /// Sets the inner radius to create a donut chart.
    /// Value is a fraction of the outer radius (0.0 to 1.0).
    #[must_use]
    pub fn donut(mut self, inner_radius: f32) -> Self {
        self.inner_radius = inner_radius;
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for PieChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        // Create base renderer with styling options
        let mut renderer = PieChartRenderer::new();
        renderer.set_inner_radius(self.inner_radius);
        if !self.colors.is_empty() {
            renderer.set_colors(self.colors);
        }

        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
