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

impl<S: Signal<Output = AreaData>> AreaChart<S> {
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

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<AreaDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<AreaDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = AreaData> + Clone + 'static> View for AreaChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        interactive_cartesian_signal_canvas(
            _env,
            self.data,
            area_bounds,
            move |ctx, data, bounds| area_geometry(ctx, data, bounds),
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
