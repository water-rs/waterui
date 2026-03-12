//! Scatter chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    draw_scatter, interactive_signal_canvas, point_bounds, point_geometry,
};
use crate::data::DataPoint;
use crate::interaction::{HitResult, SelectionBindings};
use crate::params::{ChartParamError, PositiveF32};

/// Scatter chart visualization.
pub struct ScatterChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    radius: f32,
    selection: SelectionBindings<DataPoint>,
}

impl<S: Signal<Output = Vec<DataPoint>>> ScatterChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#8B5CF6"),
            radius: 4.0,
            selection: SelectionBindings::default(),
        }
    }

    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub fn radius(self, radius: f32) -> Self {
        self.try_radius(radius)
            .expect("ScatterChart::radius(radius) requires finite radius > 0")
    }

    #[must_use]
    pub fn with_radius(mut self, radius: PositiveF32) -> Self {
        self.radius = radius.get();
        self
    }

    pub fn try_radius(self, radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_radius(PositiveF32::try_new(radius)?))
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for ScatterChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        let radius = self.radius;
        interactive_signal_canvas(
            self.data,
            move |ctx, data| {
                let bounds = point_bounds(data);
                point_geometry(ctx, data, bounds, radius.max(8.0))
            },
            move |ctx, data, geometry| {
                draw_scatter(ctx, data, geometry.bounds, color, radius);
            },
            self.selection,
        )
    }
}
