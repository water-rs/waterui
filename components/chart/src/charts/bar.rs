//! Bar chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{bar_bounds, draw_bar, interactive_cartesian_canvas};
use crate::data::DataPoint;

/// Bar chart visualization.
///
/// Renders data as vertical bars with GPU-accelerated rendering.
/// Supports smooth animations and hover interactions.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
/// use waterui_chart::{BarChart, DataPoint};
///
/// let data = binding(vec![
///     DataPoint::new(0.0, 100.0),
///     DataPoint::new(1.0, 150.0),
///     DataPoint::new(2.0, 80.0),
/// ]);
///
/// BarChart::new(&data)
///     .color(Srgb::from_hex("#3B82F6"))
/// ```
pub struct BarChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
}

impl<S: Signal<Output = Vec<DataPoint>>> BarChart<S> {
    /// Creates a new bar chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#3B82F6"), // Default blue
        }
    }

    /// Sets the bar color.
    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for BarChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        interactive_cartesian_canvas(
            self.data,
            |data: &Vec<DataPoint>| bar_bounds(data),
            move |ctx, data, bounds| {
                draw_bar(ctx, data, bounds, color);
            },
        )
    }
}
