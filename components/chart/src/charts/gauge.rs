//! Gauge chart component.

use core::f32::consts::PI;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_gauge, signal_canvas};
use crate::data::GaugeData;
use crate::params::{ArcAngles, ChartParamError, GaugeRadii};

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
    pub fn arc_degrees(self, start: f32, end: f32) -> Self {
        self.try_arc_degrees(start, end)
            .expect("GaugeChart::arc_degrees(start, end) requires finite end > start")
    }

    /// Sets the arc angle range in degrees (fallible).
    pub fn try_arc_degrees(self, start: f32, end: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_arc_angles(ArcAngles::try_degrees(start, end)?))
    }

    /// Sets the arc angle range in radians using a validated strong type.
    #[must_use]
    pub fn with_arc_angles(mut self, angles: ArcAngles) -> Self {
        self.start_angle = angles.start_radians();
        self.end_angle = angles.end_radians();
        self
    }

    /// Sets the arc angle range in radians.
    #[must_use]
    pub fn arc_radians(self, start: f32, end: f32) -> Self {
        self.try_arc_radians(start, end)
            .expect("GaugeChart::arc_radians(start, end) requires finite end > start")
    }

    /// Sets the arc angle range in radians (fallible).
    pub fn try_arc_radians(self, start: f32, end: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_arc_angles(ArcAngles::try_radians(start, end)?))
    }

    /// Sets the inner and outer radius (0.0 to 0.5, relative to widget size).
    #[must_use]
    pub fn radii(self, inner: f32, outer: f32) -> Self {
        self.try_radii(inner, outer)
            .expect("GaugeChart::radii(inner, outer) requires finite 0.0 <= inner < outer <= 0.5")
    }

    /// Sets validated gauge radii.
    #[must_use]
    pub fn with_radii(mut self, radii: GaugeRadii) -> Self {
        self.inner_radius = radii.inner();
        self.outer_radius = radii.outer();
        self
    }

    /// Fallible variant of [`Self::radii`].
    pub fn try_radii(self, inner: f32, outer: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_radii(GaugeRadii::try_new(inner, outer)?))
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
        let start_angle = self.start_angle;
        let end_angle = self.end_angle;
        let inner_radius = self.inner_radius;
        let outer_radius = self.outer_radius;
        let background_color = self.background_color;
        let value_color = self.value_color;
        let needle_color = self.needle_color;
        signal_canvas(self.data, move |ctx, data| {
            draw_gauge(
                ctx,
                data,
                start_angle,
                end_angle,
                inner_radius,
                outer_radius,
                background_color,
                value_color,
                needle_color,
            );
        })
    }
}
