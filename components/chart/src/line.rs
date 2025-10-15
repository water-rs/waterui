use alloc::sync::Arc;
use alloc::vec::Vec;

use waterui_color::{Color, Srgb};
use waterui_core::{Environment, View};
use waterui_graphics::{Path, PathBuilder};

use crate::{ChartCanvas, ensure_dimensions, to_arc, usize_to_f32};

/// A simple line chart rendered using the [`Canvas`](waterui_graphics::Canvas).
#[derive(Clone)]
pub struct LineChart {
    values: Arc<[f32]>,
    width: f32,
    height: f32,
    padding: f32,
    stroke_color: Srgb,
    stroke_width: f32,
    background: Option<Srgb>,
}

impl core::fmt::Debug for LineChart {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LineChart")
            .field("points", &self.values.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl LineChart {
    /// Creates a new `LineChart` from a list of Y values.
    ///
    /// # Panics
    ///
    /// Panics if fewer than two data points are provided.
    pub fn new(values: impl Into<Vec<f32>>) -> Self {
        let values = values.into();
        assert!(values.len() > 1, "line chart requires at least two points");
        let values = to_arc(values);
        Self {
            values,
            width: 320.0,
            height: 180.0,
            padding: 12.0,
            stroke_color: Srgb::new(0.2, 0.6, 0.86),
            stroke_width: 2.0,
            background: None,
        }
    }

    /// Sets the chart width.
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        ensure_dimensions(width, self.height);
        self.width = width;
        self
    }

    /// Sets the chart height.
    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        ensure_dimensions(self.width, height);
        self.height = height;
        self
    }

    /// Sets the padding applied to the plot area.
    #[must_use]
    pub const fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Sets the stroke color for the plotted line.
    #[must_use]
    pub const fn stroke_color(mut self, color: Srgb) -> Self {
        self.stroke_color = color;
        self
    }

    /// Sets the stroke width for the plotted line.
    #[must_use]
    pub const fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = width;
        self
    }

    /// Fills the chart background with the provided color.
    #[must_use]
    pub const fn background(mut self, color: Srgb) -> Self {
        self.background = Some(color);
        self
    }
}

impl View for LineChart {
    fn body(self, _env: &Environment) -> impl View {
        let Self {
            values,
            width,
            height,
            padding,
            stroke_color,
            stroke_width,
            background,
        } = self;

        let stroke_color_value = stroke_color;
        ChartCanvas::new(width, height, background).paint(move |ctx| {
            let path = line_chart_path(&values, width, height, padding);
            let color: Color = stroke_color_value.into();
            ctx.stroke(&path, &color, stroke_width);
        })
    }
}

/// Constructs a [`Path`] representing the plotted line for the provided values.
///
/// # Panics
///
/// Panics if fewer than two data points are supplied or if padding consumes the
/// entire chart size.
#[must_use]
pub fn line_chart_path(values: &[f32], width: f32, height: f32, padding: f32) -> Path {
    ensure_dimensions(width, height);
    assert!(values.len() > 1, "line chart requires at least two points");

    let (min, max) = values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &v| {
            (min.min(v), max.max(v))
        });

    let plot_width = padding.mul_add(-2.0, width);
    let plot_height = padding.mul_add(-2.0, height);
    assert!(
        plot_width > 0.0 && plot_height > 0.0,
        "line chart padding too large"
    );

    let range = max - min;
    let mut builder = PathBuilder::new();

    let first_normalized = if range > 0.0 {
        (values[0] - min) / range
    } else {
        0.5
    };
    let first_x = padding;
    let first_y = first_normalized.mul_add(-plot_height, height - padding);
    builder = builder.move_to([first_x, first_y]);

    let step = plot_width / usize_to_f32(values.len() - 1, "line chart point count");

    for (index, value) in values.iter().enumerate().skip(1) {
        let normalized = if range > 0.0 {
            (*value - min) / range
        } else {
            0.5
        };
        let index_f = usize_to_f32(index, "line chart index");
        let x = index_f.mul_add(step, padding);
        let y = normalized.mul_add(-plot_height, height - padding);
        builder = builder.line_to([x, y]);
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::shape::PathCommand;

    #[test]
    fn line_path_spans_chart_area() {
        let values = vec![0.0, 5.0, 10.0];
        let path = line_chart_path(&values, 100.0, 50.0, 10.0);
        assert_eq!(path.0.len(), 3);
        match path.0[0] {
            PathCommand::MoveTo([x, y]) => {
                assert!((x - 10.0).abs() < f32::EPSILON);
                assert!((y - 40.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected move_to"),
        }
        match path.0[2] {
            PathCommand::LineTo([x, y]) => {
                assert!((x - 90.0).abs() < f32::EPSILON);
                assert!((y - 10.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected line_to"),
        }
    }
}
