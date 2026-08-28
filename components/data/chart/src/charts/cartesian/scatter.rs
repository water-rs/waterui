//! Scatter chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    draw_scatter, interactive_cartesian_signal_canvas, point_bounds, point_geometry,
};
use crate::composition::ChartComposition;
use crate::data::DataPoint;
use crate::interaction::{
    CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings,
};
use crate::params::PositiveF32;

/// Scatter chart visualization.
pub struct ScatterChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    radius: f32,
    selection: SelectionBindings<DataPoint>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<DataPoint>,
}

crate::charts::impl_chart_debug!(ScatterChart, S, Vec<DataPoint>);

impl<S: Signal<Output = Vec<DataPoint>>> ScatterChart<S> {
    /// Creates a scatter chart from reactive point data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#8B5CF6"),
            radius: 4.0,
            selection: SelectionBindings::default(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(DataPoint);

    /// Sets the point color.
    #[must_use]
    pub const fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Sets the point radius.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values.
    #[must_use]
    pub fn radius(mut self, radius: impl Into<PositiveF32>) -> Self {
        self.radius = radius.into().get();
        self
    }

    /// Tracks the currently focused point in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected point in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for ScatterChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let color = self.color;
        let radius = self.radius;
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            |data: &Vec<DataPoint>| point_bounds(data),
            move |ctx, data, bounds| point_geometry(ctx, data, bounds, radius.max(8.0)),
            move |ctx, data, geometry, _transition_alpha| {
                draw_scatter(ctx, data, geometry.bounds, color, radius);
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`ScatterChart`]. Equivalent to [`ScatterChart::new`].
#[must_use]
pub fn scatter_chart<S: Signal<Output = Vec<DataPoint>>>(data: S) -> ScatterChart<S> {
    ScatterChart::new(data)
}
