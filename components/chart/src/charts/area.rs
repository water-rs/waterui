//! Area chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{area_bounds, area_geometry, draw_area, interactive_signal_canvas};
use crate::data::AreaData;
use crate::interaction::{AreaDatum, HitResult, SelectionBindings};

/// Stacked area chart for cumulative data visualization.
pub struct AreaChart<S: Signal<Output = AreaData>> {
    data: S,
    selection: SelectionBindings<AreaDatum>,
}

impl<S: Signal<Output = AreaData>> AreaChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            selection: SelectionBindings::new(),
        }
    }

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
        interactive_signal_canvas(
            self.data,
            move |ctx, data| {
                let bounds = area_bounds(data);
                area_geometry(ctx, data, bounds)
            },
            move |ctx, data, geometry| {
                draw_area(ctx, data, geometry.bounds);
            },
            self.selection,
        )
    }
}
