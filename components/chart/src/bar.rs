use alloc::sync::Arc;
use alloc::vec::Vec;

use waterui_color::{Color, Srgb};
use waterui_core::{Environment, View};
use waterui_graphics::{Path, PathBuilder};

use crate::{ChartCanvas, ensure_dimensions, to_arc, usize_to_f32};

/// A bar chart rendered on the `WaterUI` canvas.
#[derive(Clone)]
pub struct BarChart {
    values: Arc<[f32]>,
    width: f32,
    height: f32,
    padding: f32,
    spacing: f32,
    fill_color: Srgb,
    background: Option<Srgb>,
}

impl core::fmt::Debug for BarChart {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BarChart")
            .field("bars", &self.values.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl BarChart {
    /// Creates a new bar chart from the provided values.
    pub fn new(values: impl Into<Vec<f32>>) -> Self {
        let values = values.into();
        let values = to_arc(values);
        Self {
            values,
            width: 320.0,
            height: 180.0,
            padding: 12.0,
            spacing: 0.2,
            fill_color: Srgb::new(0.2, 0.6, 0.86),
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

    /// Sets the padding around the chart plot area.
    #[must_use]
    pub const fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Controls the fraction of each bar reserved as spacing.
    ///
    /// # Panics
    ///
    /// Panics if the provided spacing lies outside the range [0, 1).
    #[must_use]
    pub fn spacing(mut self, spacing: f32) -> Self {
        assert!(
            (0.0..1.0).contains(&spacing),
            "bar spacing must be in [0, 1)"
        );
        self.spacing = spacing;
        self
    }

    /// Sets the fill color used for each bar.
    #[must_use]
    pub const fn fill_color(mut self, color: Srgb) -> Self {
        self.fill_color = color;
        self
    }

    /// Fills the chart background with the provided color.
    #[must_use]
    pub const fn background(mut self, color: Srgb) -> Self {
        self.background = Some(color);
        self
    }
}

impl View for BarChart {
    fn body(self, _env: &Environment) -> impl View {
        let Self {
            values,
            width,
            height,
            padding,
            spacing,
            fill_color,
            background,
        } = self;

        let fill_color_value = fill_color;
        ChartCanvas::new(width, height, background).paint(move |ctx| {
            let rectangles = bar_rectangles(&values, width, height, padding, spacing);
            let color: Color = fill_color_value.into();
            for rect in rectangles {
                ctx.fill(&rect, &color);
            }
        })
    }
}

/// Builds rectangle paths for each bar value within the chart bounds.
///
/// # Panics
///
/// Panics if the values slice is empty, all values are identical, or if the
/// padding consumes the chart dimensions.
#[must_use]
pub fn bar_rectangles(
    values: &[f32],
    width: f32,
    height: f32,
    padding: f32,
    spacing: f32,
) -> Vec<Path> {
    ensure_dimensions(width, height);
    assert!(!values.is_empty(), "bar chart requires values");

    let (min, max) = values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &v| {
            (min.min(v), max.max(v))
        });
    assert!(max > min, "bar chart expects a non-uniform dataset");

    let plot_width = padding.mul_add(-2.0, width);
    let plot_height = padding.mul_add(-2.0, height);
    assert!(
        plot_width > 0.0 && plot_height > 0.0,
        "bar chart padding too large"
    );

    let range = max - min;
    let zero_offset = if min < 0.0 && max > 0.0 {
        (-min) / range
    } else if min >= 0.0 {
        0.0
    } else {
        1.0
    };

    let zero_y = zero_offset.mul_add(-plot_height, height - padding);

    let bar_count = values.len();
    let bar_count_f32 = usize_to_f32(bar_count, "bar count");
    let bar_band = plot_width / bar_count_f32;
    assert!(
        (0.0..1.0).contains(&spacing),
        "bar spacing must be in [0, 1)"
    );
    let gap = bar_band * spacing;
    let bar_width = bar_band - gap;

    let mut rectangles = Vec::with_capacity(bar_count);

    for (index, value) in values.iter().enumerate() {
        let normalized = (value - min) / range;
        let value_y = normalized.mul_add(-plot_height, height - padding);

        let index_f = usize_to_f32(index, "bar index");
        let start_x = index_f.mul_add(bar_band, padding) + (gap / 2.0);
        let end_x = start_x + bar_width;

        let (top, bottom) = if value_y < zero_y {
            (value_y, zero_y)
        } else {
            (zero_y, value_y)
        };

        let rect = PathBuilder::new()
            .move_to([start_x, bottom])
            .line_to([start_x, top])
            .line_to([end_x, top])
            .line_to([end_x, bottom])
            .close()
            .build();
        rectangles.push(rect);
    }

    rectangles
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::shape::PathCommand;

    #[test]
    fn rectangles_cover_positive_values() {
        let values = vec![1.0, 2.0, 3.0];
        let rects = bar_rectangles(&values, 120.0, 60.0, 10.0, 0.2);
        assert_eq!(rects.len(), 3);
        let first = &rects[0];
        match first.0[0] {
            PathCommand::MoveTo([x, y]) => {
                assert!(x > 10.0);
                assert!((y - 50.0).abs() < 1.0);
            }
            _ => panic!("expected move_to"),
        }
    }
}
