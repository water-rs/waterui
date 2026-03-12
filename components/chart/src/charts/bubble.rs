//! Bubble chart component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{bubble_bounds, draw_bubble, interactive_cartesian_canvas};
use crate::data::BubblePoint;
use crate::params::{ChartParamError, PositiveF32, UnitInterval};

/// Bubble chart for 3D data visualization.
///
/// Similar to scatter plot but each point has a variable radius
/// proportional to a third dimension (size value).
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{BubbleChart, BubblePoint};
///
/// let data = binding(vec![
///     BubblePoint::new(1.0, 2.0, 10.0),  // x, y, size
///     BubblePoint::new(3.0, 4.0, 25.0),
///     BubblePoint::new(5.0, 1.0, 15.0),
/// ]);
///
/// BubbleChart::new(data)
///     .min_radius(5.0)
///     .max_radius(30.0)
/// ```
pub struct BubbleChart<S: Signal<Output = Vec<BubblePoint>>> {
    data: S,
    color: Srgb,
    min_radius: f32,
    max_radius: f32,
    opacity: f32,
}

impl<S: Signal<Output = Vec<BubblePoint>>> BubbleChart<S> {
    /// Creates a new bubble chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::new(0.23, 0.51, 0.96),
            min_radius: 5.0,
            max_radius: 30.0,
            opacity: 0.7,
        }
    }

    /// Sets the default bubble color.
    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Sets the minimum bubble radius in pixels.
    #[must_use]
    pub fn min_radius(self, radius: f32) -> Self {
        self.try_min_radius(radius)
            .expect("BubbleChart::min_radius(radius) requires finite radius > 0")
    }

    /// Sets minimum radius using a validated strong type.
    #[must_use]
    pub fn with_min_radius(mut self, radius: PositiveF32) -> Self {
        self.min_radius = radius.get();
        if self.max_radius < self.min_radius {
            self.max_radius = self.min_radius;
        }
        self
    }

    /// Fallible variant of [`Self::min_radius`].
    pub fn try_min_radius(self, radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_min_radius(PositiveF32::try_new(radius)?))
    }

    /// Sets the maximum bubble radius in pixels.
    #[must_use]
    pub fn max_radius(self, radius: f32) -> Self {
        self.try_max_radius(radius)
            .expect("BubbleChart::max_radius(radius) requires finite radius > 0")
    }

    /// Sets maximum radius using a validated strong type.
    #[must_use]
    pub fn with_max_radius(mut self, radius: PositiveF32) -> Self {
        self.max_radius = radius.get();
        if self.max_radius < self.min_radius {
            self.min_radius = self.max_radius;
        }
        self
    }

    /// Fallible variant of [`Self::max_radius`].
    pub fn try_max_radius(self, radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_max_radius(PositiveF32::try_new(radius)?))
    }

    /// Sets the bubble opacity.
    #[must_use]
    pub fn opacity(self, opacity: f32) -> Self {
        self.try_opacity(opacity)
            .expect("BubbleChart::opacity(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    /// Sets bubble opacity using a validated strong type.
    #[must_use]
    pub fn with_opacity(mut self, opacity: UnitInterval) -> Self {
        self.opacity = opacity.get();
        self
    }

    /// Fallible variant of [`Self::opacity`].
    pub fn try_opacity(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_opacity(UnitInterval::try_new(opacity)?))
    }
}

impl<S: Signal<Output = Vec<BubblePoint>> + Clone + 'static> View for BubbleChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        let min_radius = self.min_radius;
        let max_radius = self.max_radius;
        let opacity = self.opacity;
        interactive_cartesian_canvas(
            self.data,
            |data: &Vec<BubblePoint>| bubble_bounds(data),
            move |ctx, data, bounds| {
                draw_bubble(ctx, data, bounds, color, min_radius, max_radius, opacity);
            },
        )
    }
}
