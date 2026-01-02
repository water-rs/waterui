//! Line chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;
use waterui_graphics::GpuSurface;

use crate::charts::SignalRenderer;
use crate::data::DataPoint;
use crate::renderer::LineChartRenderer;

/// Line chart visualization.
///
/// Renders data as connected lines with optional area fill.
/// Supports smooth animations and hover interactions.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui_chart::{LineChart, DataPoint};
///
/// let data = binding(vec![
///     DataPoint::new(0.0, 10.0),
///     DataPoint::new(1.0, 25.0),
///     DataPoint::new(2.0, 15.0),
///     DataPoint::new(3.0, 30.0),
/// ]);
///
/// LineChart::new(&data)
///     .color(Srgb::from_hex("#22C55E"))
///     .line_width(2.0)
/// ```
pub struct LineChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    line_width: f32,
    show_fill: bool,
    fill_opacity: f32,
}

impl<S: Signal<Output = Vec<DataPoint>>> LineChart<S> {
    /// Creates a new line chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#22C55E"), // Default green
            line_width: 2.0,
            show_fill: false,
            fill_opacity: 0.3,
        }
    }

    /// Sets the line color.
    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Sets the line width in pixels.
    #[must_use]
    pub fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    /// Enables area fill below the line.
    #[must_use]
    pub fn fill(mut self, opacity: f32) -> Self {
        self.show_fill = true;
        self.fill_opacity = opacity;
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for LineChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        // Create base renderer with styling options
        let mut renderer = LineChartRenderer::new();
        renderer.set_color(self.color);
        renderer.set_line_width(self.line_width);
        renderer.set_fill(self.show_fill, self.fill_opacity);

        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
