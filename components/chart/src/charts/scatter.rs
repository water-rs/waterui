//! Scatter plot component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_scatter, interactive_cartesian_canvas, point_bounds};
use crate::data::DataPoint;
use crate::params::{ChartParamError, PositiveF32};

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
    pub fn radius(self, radius: f32) -> Self {
        self.try_radius(radius)
            .expect("ScatterChart::radius(radius) requires finite radius > 0")
    }

    /// Sets the point radius using a validated strong type.
    #[must_use]
    pub fn with_radius(mut self, radius: PositiveF32) -> Self {
        self.radius = radius.get();
        self
    }

    /// Fallible variant of [`Self::radius`].
    pub fn try_radius(self, radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_radius(PositiveF32::try_new(radius)?))
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for ScatterChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        let radius = self.radius;
        interactive_cartesian_canvas(
            self.data,
            |data: &Vec<DataPoint>| point_bounds(data),
            move |ctx, data, bounds| {
                draw_scatter(ctx, data, bounds, color, radius);
            },
        )
    }
}
