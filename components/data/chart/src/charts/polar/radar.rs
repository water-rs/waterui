//! Radar/Spider chart component.

use core::num::NonZeroU32;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{draw_radar, interactive_signal_canvas, radar_geometry};
use crate::composition::ChartComposition;
use crate::data::RadarData;
use crate::interaction::{HitResult, RadarDatum, SelectionBindings};
use crate::params::{ChartParamError, PositiveF32, UnitInterval};

/// Radar/Spider chart for multivariate data visualization.
pub struct RadarChart<S: Signal<Output = RadarData>> {
    data: S,
    ring_count: u32,
    line_width: f32,
    fill_opacity: f32,
    selection: SelectionBindings<RadarDatum>,
    composition: ChartComposition<RadarDatum>,
}

crate::charts::impl_chart_debug!(RadarChart, S, RadarData);

impl<S: Signal<Output = RadarData>> RadarChart<S> {
    /// Creates a radar chart from reactive radar data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            ring_count: 5,
            line_width: 2.0,
            fill_opacity: 0.3,
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(RadarDatum);

    /// Tracks the currently focused radar axis in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<RadarDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected radar axis in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<RadarDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }

    /// Sets the number of guide rings and panics if the value is invalid.
    ///
    /// # Panics
    ///
    /// Panics when `count` is zero.
    #[must_use]
    pub fn ring_count(self, count: u32) -> Self {
        self.try_ring_count(count)
            .expect("RadarChart::ring_count(count) requires count >= 1")
    }

    /// Sets the number of guide rings using an already-validated count.
    #[must_use]
    pub const fn with_ring_count(mut self, count: NonZeroU32) -> Self {
        self.ring_count = count.get();
        self
    }

    /// Attempts to set the number of guide rings.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when `count` is zero.
    pub fn try_ring_count(self, count: u32) -> Result<Self, ChartParamError> {
        let count = NonZeroU32::new(count).ok_or(ChartParamError::OutOfRange {
            param: "ring_count",
            value: 0.0,
            min: 1.0,
            max: f32::INFINITY,
        })?;
        Ok(self.with_ring_count(count))
    }

    /// Sets the radar line width.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values.
    #[must_use]
    pub fn line_width(mut self, width: impl Into<PositiveF32>) -> Self {
        self.line_width = width.into().get();
        self
    }

    /// Sets the radar fill opacity.
    ///
    /// Accepts any value convertible into [`UnitInterval`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or values outside `[0.0, 1.0]`.
    #[must_use]
    pub fn fill_opacity(mut self, opacity: impl Into<UnitInterval>) -> Self {
        self.fill_opacity = opacity.into().get();
        self
    }
}

impl<S: Signal<Output = RadarData> + Clone + 'static> View for RadarChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let ring_count = self.ring_count;
        let line_width = self.line_width;
        let fill_opacity = self.fill_opacity;
        interactive_signal_canvas(
            env,
            self.data,
            radar_geometry,
            move |ctx, data, _geometry, transition_alpha| {
                draw_radar(
                    ctx,
                    data,
                    ring_count,
                    line_width,
                    fill_opacity,
                    transition_alpha,
                );
            },
            self.selection,
            self.composition,
        )
    }
}

/// Convenience constructor for [`RadarChart`]. Equivalent to [`RadarChart::new`].
#[must_use]
pub fn radar_chart<S: Signal<Output = RadarData>>(data: S) -> RadarChart<S> {
    RadarChart::new(data)
}
