//! Line chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_line, interactive_cartesian_canvas, point_bounds};
use crate::data::DataPoint;
use crate::params::{ChartParamError, PositiveF32, UnitInterval};

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
    pub fn line_width(self, width: f32) -> Self {
        self.try_line_width(width)
            .expect("LineChart::line_width(width) requires finite width > 0")
    }

    /// Sets the line width using a validated strong type.
    #[must_use]
    pub fn with_line_width(mut self, width: PositiveF32) -> Self {
        self.line_width = width.get();
        self
    }

    /// Fallible variant of [`Self::line_width`].
    pub fn try_line_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_line_width(PositiveF32::try_new(width)?))
    }

    /// Enables area fill below the line.
    #[must_use]
    pub fn fill(self, opacity: f32) -> Self {
        self.try_fill(opacity)
            .expect("LineChart::fill(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    /// Enables area fill using a validated strong type.
    #[must_use]
    pub fn with_fill_opacity(mut self, opacity: UnitInterval) -> Self {
        self.show_fill = true;
        self.fill_opacity = opacity.get();
        self
    }

    /// Fallible variant of [`Self::fill`].
    pub fn try_fill(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_fill_opacity(UnitInterval::try_new(opacity)?))
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for LineChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        let line_width = self.line_width;
        let show_fill = self.show_fill;
        let fill_opacity = self.fill_opacity;
        interactive_cartesian_canvas(
            self.data,
            |data: &Vec<DataPoint>| point_bounds(data),
            move |ctx, data, bounds| {
                draw_line(
                    ctx,
                    data,
                    bounds,
                    color,
                    line_width,
                    show_fill,
                    fill_opacity,
                );
            },
        )
    }
}
