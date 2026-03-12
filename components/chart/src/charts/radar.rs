//! Radar/Spider chart component.

use core::num::NonZeroU32;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{draw_radar, interactive_signal_canvas, radar_geometry};
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
}

impl<S: Signal<Output = RadarData>> RadarChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            ring_count: 5,
            line_width: 2.0,
            fill_opacity: 0.3,
            selection: SelectionBindings::new(),
        }
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<RadarDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<RadarDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }

    #[must_use]
    pub fn ring_count(self, count: u32) -> Self {
        self.try_ring_count(count)
            .expect("RadarChart::ring_count(count) requires count >= 1")
    }

    #[must_use]
    pub fn with_ring_count(mut self, count: NonZeroU32) -> Self {
        self.ring_count = count.get();
        self
    }

    pub fn try_ring_count(self, count: u32) -> Result<Self, ChartParamError> {
        let count = NonZeroU32::new(count).ok_or(ChartParamError::OutOfRange {
            param: "ring_count",
            value: count as f32,
            min: 1.0,
            max: u32::MAX as f32,
        })?;
        Ok(self.with_ring_count(count))
    }

    #[must_use]
    pub fn line_width(self, width: f32) -> Self {
        self.try_line_width(width)
            .expect("RadarChart::line_width(width) requires finite width > 0")
    }

    #[must_use]
    pub fn with_line_width(mut self, width: PositiveF32) -> Self {
        self.line_width = width.get();
        self
    }

    pub fn try_line_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_line_width(PositiveF32::try_new(width)?))
    }

    #[must_use]
    pub fn fill_opacity(self, opacity: f32) -> Self {
        self.try_fill_opacity(opacity)
            .expect("RadarChart::fill_opacity(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    #[must_use]
    pub fn with_fill_opacity(mut self, opacity: UnitInterval) -> Self {
        self.fill_opacity = opacity.get();
        self
    }

    pub fn try_fill_opacity(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_fill_opacity(UnitInterval::try_new(opacity)?))
    }
}

impl<S: Signal<Output = RadarData> + Clone + 'static> View for RadarChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let ring_count = self.ring_count;
        let line_width = self.line_width;
        let fill_opacity = self.fill_opacity;
        interactive_signal_canvas(
            self.data,
            move |ctx, data| radar_geometry(ctx, data),
            move |ctx, data, _geometry| {
                draw_radar(ctx, data, ring_count, line_width, fill_opacity);
            },
            self.selection,
        )
    }
}
