//! Gauge chart component.

use core::f32::consts::PI;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_gauge, gauge_geometry, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::GaugeData;
use crate::interaction::{HitResult, SelectionBindings, SliceDatum};
use crate::params::{ArcAngles, ChartParamError, GaugeRadii};

/// Gauge chart for speedometer-style value visualization.
pub struct GaugeChart<S: Signal<Output = GaugeData>> {
    data: S,
    start_angle: f32,
    end_angle: f32,
    inner_radius: f32,
    outer_radius: f32,
    background_color: Srgb,
    value_color: Srgb,
    needle_color: Srgb,
    selection: SelectionBindings<SliceDatum>,
    composition: ChartComposition<SliceDatum>,
}

impl<S: Signal<Output = GaugeData>> GaugeChart<S> {
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
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(SliceDatum);

    #[must_use]
    pub fn arc_degrees(self, start: f32, end: f32) -> Self {
        self.try_arc_degrees(start, end)
            .expect("GaugeChart::arc_degrees(start, end) requires finite end > start")
    }

    pub fn try_arc_degrees(self, start: f32, end: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_arc_angles(ArcAngles::try_degrees(start, end)?))
    }

    #[must_use]
    pub fn with_arc_angles(mut self, angles: ArcAngles) -> Self {
        self.start_angle = angles.start_radians();
        self.end_angle = angles.end_radians();
        self
    }

    #[must_use]
    pub fn arc_radians(self, start: f32, end: f32) -> Self {
        self.try_arc_radians(start, end)
            .expect("GaugeChart::arc_radians(start, end) requires finite end > start")
    }

    pub fn try_arc_radians(self, start: f32, end: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_arc_angles(ArcAngles::try_radians(start, end)?))
    }

    #[must_use]
    pub fn radii(self, inner: f32, outer: f32) -> Self {
        self.try_radii(inner, outer)
            .expect("GaugeChart::radii(inner, outer) requires finite 0.0 <= inner < outer <= 0.5")
    }

    #[must_use]
    pub fn with_radii(mut self, radii: GaugeRadii) -> Self {
        self.inner_radius = radii.inner();
        self.outer_radius = radii.outer();
        self
    }

    pub fn try_radii(self, inner: f32, outer: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_radii(GaugeRadii::try_new(inner, outer)?))
    }

    #[must_use]
    pub fn background_color(mut self, color: Srgb) -> Self {
        self.background_color = color;
        self
    }

    #[must_use]
    pub fn value_color(mut self, color: Srgb) -> Self {
        self.value_color = color;
        self
    }

    #[must_use]
    pub fn needle_color(mut self, color: Srgb) -> Self {
        self.needle_color = color;
        self
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
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
        interactive_signal_canvas(
            _env,
            self.data,
            move |ctx, data| {
                gauge_geometry(
                    ctx,
                    data,
                    start_angle,
                    end_angle,
                    inner_radius,
                    outer_radius,
                )
            },
            move |ctx, data, _geometry| {
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
            },
            self.selection,
            self.composition,
        )
    }
}
