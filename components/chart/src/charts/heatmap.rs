//! Heatmap chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{draw_heatmap, heatmap_geometry, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::HeatmapData;
use crate::interaction::{GridDatum, HitResult, SelectionBindings};

/// Heatmap chart for matrix visualization.
pub struct HeatmapChart<S: Signal<Output = HeatmapData>> {
    data: S,
    selection: SelectionBindings<GridDatum>,
    composition: ChartComposition<GridDatum>,
}

impl<S: Signal<Output = HeatmapData>> HeatmapChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(GridDatum);

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<GridDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<GridDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = HeatmapData> + Clone + 'static> View for HeatmapChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        interactive_signal_canvas(
            _env,
            self.data,
            move |ctx, data| heatmap_geometry(ctx, data),
            move |ctx, data, _geometry| {
                draw_heatmap(ctx, data);
            },
            self.selection,
            self.composition,
        )
    }
}
