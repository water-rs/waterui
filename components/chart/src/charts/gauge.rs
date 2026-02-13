//! Gauge chart component.

use core::f32::consts::PI;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;
use waterui_graphics::color::Srgb;

use crate::charts::SignalRenderer;
use crate::data::GaugeData;
use crate::renderer::GaugeRenderer;

/// Gauge chart for speedometer-style value visualization.
///
/// Displays a single value within a range using a circular arc gauge.
/// Supports colored threshold regions and an optional needle indicator.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{GaugeChart, GaugeData, GaugeRegion};
///
/// let data = binding(
///     GaugeData::new(75.0, 0.0, 100.0)
///         .region(GaugeRegion::hex(30.0, "#22C55E"))  // Green: 0-30
///         .region(GaugeRegion::hex(70.0, "#EAB308"))  // Yellow: 30-70
///         .region(GaugeRegion::hex(100.0, "#EF4444")) // Red: 70-100
///         .show_needle(true)
/// );
///
/// GaugeChart::new(data)
///     .arc_degrees(-135.0, 135.0)
///     .radii(0.3, 0.45)
/// ```
pub struct GaugeChart<S: Signal<Output = GaugeData>> {
    data: S,
    start_angle: f32,
    end_angle: f32,
    inner_radius: f32,
    outer_radius: f32,
    background_color: Srgb,
    value_color: Srgb,
    needle_color: Srgb,
}

impl<S: Signal<Output = GaugeData>> GaugeChart<S> {
    /// Creates a new gauge chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            start_angle: -PI * 0.75,
            end_angle: PI * 0.75,
            inner_radius: 0.3,
            outer_radius: 0.45,
            background_color: Srgb::new(0.2, 0.2, 0.2),
            value_color: Srgb::new(0.23, 0.51, 0.96),
            needle_color: Srgb::new(0.9, 0.9, 0.9),
        }
    }

    /// Sets the arc angle range in degrees.
    /// Default is -135° to 135° (270° arc).
    #[must_use]
    pub fn arc_degrees(mut self, start: f32, end: f32) -> Self {
        assert!(
            start.is_finite() && end.is_finite() && end > start,
            "GaugeChart::arc_degrees(start, end) requires finite end > start"
        );
        self.start_angle = start * PI / 180.0;
        self.end_angle = end * PI / 180.0;
        self
    }

    /// Sets the arc angle range in radians.
    #[must_use]
    pub fn arc_radians(mut self, start: f32, end: f32) -> Self {
        assert!(
            start.is_finite() && end.is_finite() && end > start,
            "GaugeChart::arc_radians(start, end) requires finite end > start"
        );
        self.start_angle = start;
        self.end_angle = end;
        self
    }

    /// Sets the inner and outer radius (0.0 to 0.5, relative to widget size).
    #[must_use]
    pub fn radii(mut self, inner: f32, outer: f32) -> Self {
        assert!(
            inner.is_finite() && outer.is_finite() && inner >= 0.0 && outer > inner && outer <= 0.5,
            "GaugeChart::radii(inner, outer) requires 0.0 <= inner < outer <= 0.5"
        );
        self.inner_radius = inner;
        self.outer_radius = outer;
        self
    }

    /// Sets the background arc color.
    #[must_use]
    pub fn background_color(mut self, color: Srgb) -> Self {
        self.background_color = color;
        self
    }

    /// Sets the value arc color (used when no regions are defined).
    #[must_use]
    pub fn value_color(mut self, color: Srgb) -> Self {
        self.value_color = color;
        self
    }

    /// Sets the needle color.
    #[must_use]
    pub fn needle_color(mut self, color: Srgb) -> Self {
        self.needle_color = color;
        self
    }
}

impl<S: Signal<Output = GaugeData> + Clone + 'static> View for GaugeChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let renderer = GaugeRenderer::new()
            .arc_angles(self.start_angle, self.end_angle)
            .radii(self.inner_radius, self.outer_radius)
            .background_color(self.background_color)
            .value_color(self.value_color)
            .needle_color(self.needle_color);
        GpuSurface::new(SignalRenderer::new(renderer, self.data))
    }
}
