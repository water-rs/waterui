//! Area chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{
    area_bounds, area_geometry, draw_area, interactive_cartesian_signal_canvas,
};
use crate::composition::ChartComposition;
use crate::data::AreaData;
use crate::interaction::{
    AreaDatum, CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings,
};

/// Stacked area chart for cumulative data visualization.
pub struct AreaChart<S: Signal<Output = AreaData>> {
    data: S,
    selection: SelectionBindings<AreaDatum>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<AreaDatum>,
}

crate::charts::impl_chart_debug!(AreaChart, S, AreaData);

impl<S: Signal<Output = AreaData>> AreaChart<S> {
    /// Creates an area chart from reactive area-series data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            selection: SelectionBindings::new(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(AreaDatum);

    /// Tracks the currently focused area datum in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<AreaDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected area datum in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<AreaDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = AreaData> + Clone + 'static> View for AreaChart<S> {
    fn body(self, env: &Environment) -> impl View {
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            area_bounds,
            area_geometry,
            move |ctx, data, geometry| {
                draw_area(ctx, data, geometry.bounds);
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`AreaChart`]. Equivalent to [`AreaChart::new`].
#[must_use]
pub fn area_chart<S: Signal<Output = AreaData>>(data: S) -> AreaChart<S> {
    AreaChart::new(data)
}
