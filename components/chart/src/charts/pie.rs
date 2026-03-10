//! Pie chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_pie, signal_canvas};
use crate::data::DataPoint;
use crate::params::{ChartParamError, DonutInnerRadius};

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
    pub fn donut(self, inner_radius: f32) -> Self {
        self.try_donut(inner_radius)
            .expect("PieChart::donut(inner_radius) requires finite 0.0 <= inner_radius <= 0.95")
    }

    /// Sets inner radius using a validated strong type.
    #[must_use]
    pub fn with_donut(mut self, inner_radius: DonutInnerRadius) -> Self {
        self.inner_radius = inner_radius.get();
        self
    }

    /// Fallible variant of [`Self::donut`].
    pub fn try_donut(self, inner_radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_donut(DonutInnerRadius::try_new(inner_radius)?))
    }

    /// Disables donut mode and renders a full pie.
    #[must_use]
    pub fn full_pie(mut self) -> Self {
        self.inner_radius = 0.0;
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for PieChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let colors = self.colors;
        let inner_radius = self.inner_radius;
        signal_canvas(self.data, move |ctx, data| {
            let colors = colors.clone();
            draw_pie(ctx, data, &colors, inner_radius);
        })
    }
}
