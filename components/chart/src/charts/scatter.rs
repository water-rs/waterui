//! Scatter plot component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;
use waterui_graphics::GpuSurface;

use crate::charts::SignalRenderer;
use crate::data::DataPoint;
use crate::renderer::ScatterChartRenderer;

/// Scatter plot visualization.
///
/// Renders data as points, optimized for 1M+ data points via GPU instancing.
/// Supports smooth animations and hover interactions.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui_chart::{ScatterChart, DataPoint};
///
/// let data = binding(vec![
///     DataPoint::new(1.0, 2.5),
///     DataPoint::new(2.0, 4.0),
///     DataPoint::new(3.0, 3.5),
///     DataPoint::new(4.0, 5.0),
/// ]);
///
/// ScatterChart::new(&data)
///     .color(Srgb::from_hex("#3B82F6"))
///     .radius(6.0)
/// ```
pub struct ScatterChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    radius: f32,
}

impl<S: Signal<Output = Vec<DataPoint>>> ScatterChart<S> {
    /// Creates a new scatter chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#3B82F6"), // Default blue
            radius: 4.0,
        }
    }

    /// Sets the point color.
    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Sets the point radius in pixels.
    #[must_use]
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for ScatterChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        // Create base renderer with styling options
        let mut renderer = ScatterChartRenderer::new();
        renderer.set_color(self.color);
        renderer.set_radius(self.radius);

        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
