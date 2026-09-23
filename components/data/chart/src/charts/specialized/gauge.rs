//! Gauge chart component.

use core::f32::consts::PI;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_gauge, gauge_geometry, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::GaugeData;
use crate::interaction::{HitResult, SelectionBindings, SliceDatum};
use crate::params::{ArcAngles, GaugeRadii};

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

crate::charts::impl_chart_debug!(GaugeChart, S, GaugeData);

impl<S: Signal<Output = GaugeData>> GaugeChart<S> {
    /// Creates a gauge chart from reactive gauge data.
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

    /// Sets the gauge arc.
    ///
    /// Accepts any value convertible into [`ArcAngles`]. Use
    /// [`ArcAngles::from_degrees`] / [`ArcAngles::from_radians`] for
    /// fail-fast factories that panic on `end <= start` or non-finite
    /// inputs, or [`ArcAngles::try_radians`] / [`ArcAngles::try_degrees`]
    /// for the fallible variants.
    #[must_use]
    pub fn arc(mut self, angles: impl Into<ArcAngles>) -> Self {
        let angles = angles.into();
        self.start_angle = angles.start_radians();
        self.end_angle = angles.end_radians();
        self
    }

    /// Sets the gauge ring radii.
    ///
    /// Accepts any value convertible into [`GaugeRadii`]. Use
    /// [`GaugeRadii::new`] for fail-fast construction or
    /// [`GaugeRadii::try_new`] for the fallible variant.
    #[must_use]
    pub fn radii(mut self, radii: impl Into<GaugeRadii>) -> Self {
        let radii = radii.into();
        self.inner_radius = radii.inner();
        self.outer_radius = radii.outer();
        self
    }

    /// Sets the background arc color.
    #[must_use]
    pub const fn background_color(mut self, color: Srgb) -> Self {
        self.background_color = color;
        self
    }

    /// Sets the active value arc color.
    #[must_use]
    pub const fn value_color(mut self, color: Srgb) -> Self {
        self.value_color = color;
        self
    }

    /// Sets the needle color.
    #[must_use]
    pub const fn needle_color(mut self, color: Srgb) -> Self {
        self.needle_color = color;
        self
    }

    /// Tracks the currently focused gauge sector in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected gauge sector in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = GaugeData> + Clone + 'static> View for GaugeChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let start_angle = self.start_angle;
        let end_angle = self.end_angle;
        let inner_radius = self.inner_radius;
        let outer_radius = self.outer_radius;
        let background_color = self.background_color;
        let value_color = self.value_color;
        let needle_color = self.needle_color;
        interactive_signal_canvas(
            env,
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
            move |ctx, data, _geometry, _transition_alpha| {
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

/// Convenience constructor for [`GaugeChart`]. Equivalent to [`GaugeChart::new`].
#[must_use]
pub fn gauge_chart<S: Signal<Output = GaugeData>>(data: S) -> GaugeChart<S> {
    GaugeChart::new(data)
}
