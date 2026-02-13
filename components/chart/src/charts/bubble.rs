//! Bubble chart component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;
use waterui_graphics::color::Srgb;

use crate::charts::SignalRenderer;
use crate::data::BubblePoint;
use crate::renderer::BubbleRenderer;

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
    pub fn min_radius(mut self, radius: f32) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0,
            "BubbleChart::min_radius(radius) requires radius > 0"
        );
        self.min_radius = radius;
        if self.max_radius < self.min_radius {
            self.max_radius = self.min_radius;
        }
        self
    }

    /// Sets the maximum bubble radius in pixels.
    #[must_use]
    pub fn max_radius(mut self, radius: f32) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0,
            "BubbleChart::max_radius(radius) requires radius > 0"
        );
        self.max_radius = radius;
        if self.max_radius < self.min_radius {
            self.min_radius = self.max_radius;
        }
        self
    }

    /// Sets the bubble opacity.
    #[must_use]
    pub fn opacity(mut self, opacity: f32) -> Self {
        assert!(
            opacity.is_finite() && (0.0..=1.0).contains(&opacity),
            "BubbleChart::opacity(opacity) requires 0.0 <= opacity <= 1.0"
        );
        self.opacity = opacity;
        self
    }
}

impl<S: Signal<Output = Vec<BubblePoint>> + Clone + 'static> View for BubbleChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let renderer = BubbleRenderer::new()
            .color(self.color)
            .min_radius(self.min_radius)
            .max_radius(self.max_radius)
            .opacity(self.opacity);
        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
