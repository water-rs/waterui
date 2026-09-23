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

crate::charts::impl_chart_debug!(HeatmapChart, S, HeatmapData);

impl<S: Signal<Output = HeatmapData>> HeatmapChart<S> {
    /// Creates a heatmap chart from reactive grid data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(GridDatum);

    /// Tracks the currently focused heatmap cell in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<GridDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected heatmap cell in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<GridDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = HeatmapData> + Clone + 'static> View for HeatmapChart<S> {
    fn body(self, env: &Environment) -> impl View {
        interactive_signal_canvas(
            env,
            self.data,
            heatmap_geometry,
            move |ctx, data, _geometry, _transition_alpha| {
                draw_heatmap(ctx, data);
            },
            self.selection,
            self.composition,
        )
    }
}

/// Convenience constructor for [`HeatmapChart`]. Equivalent to [`HeatmapChart::new`].
#[must_use]
pub fn heatmap_chart<S: Signal<Output = HeatmapData>>(data: S) -> HeatmapChart<S> {
    HeatmapChart::new(data)
}
