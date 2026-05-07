//! Bar chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    bar_bounds, bar_geometry, draw_bar, interactive_cartesian_signal_canvas,
};
use crate::composition::ChartComposition;
use crate::data::DataPoint;
use crate::interaction::{
    CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings,
};

/// Bar chart visualization.
pub struct BarChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    selection: SelectionBindings<DataPoint>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<DataPoint>,
}

crate::charts::impl_chart_debug!(BarChart, S, Vec<DataPoint>);

impl<S: Signal<Output = Vec<DataPoint>>> BarChart<S> {
    /// Creates a bar chart from reactive point data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#3B82F6"),
            selection: SelectionBindings::default(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(DataPoint);

    /// Sets the fill color used for all bars.
    #[must_use]
    pub const fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Tracks the currently focused bar datum in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected bar datum in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for BarChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let color = self.color;
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            |data: &Vec<DataPoint>| bar_bounds(data),
            move |ctx, data, bounds| bar_geometry(ctx, data, bounds),
            move |ctx, data, geometry| {
                draw_bar(ctx, data, geometry.bounds, color);
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`BarChart`]. Equivalent to [`BarChart::new`].
#[must_use]
pub fn bar_chart<S: Signal<Output = Vec<DataPoint>>>(data: S) -> BarChart<S> {
    BarChart::new(data)
}
