//! Heatmap chart component.

use nami::Signal;
use waterui_canvas::Canvas;
use waterui_core::{Environment, View};

use crate::charts::canvas::{draw_heatmap, reactive_canvas};
use crate::data::HeatmapData;

/// Heatmap chart for matrix visualization.
///
/// Renders a 2D grid of cells colored by value using the Viridis color scale.
/// Ideal for correlation matrices, time-series grids, and density plots.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{HeatmapChart, HeatmapData};
///
/// // Create a 5x5 heatmap
/// let data = binding(HeatmapData::new(5, 5, vec![
///     0.0, 0.2, 0.4, 0.6, 0.8,
///     0.2, 0.4, 0.6, 0.8, 1.0,
///     0.4, 0.6, 0.8, 1.0, 0.8,
///     0.6, 0.8, 1.0, 0.8, 0.6,
///     0.8, 1.0, 0.8, 0.6, 0.4,
/// ]));
///
/// HeatmapChart::new(data)
/// ```
pub struct HeatmapChart<S: Signal<Output = HeatmapData>> {
    data: S,
}

impl<S: Signal<Output = HeatmapData>> HeatmapChart<S> {
    /// Creates a new heatmap chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self { data }
    }
}

impl<S: Signal<Output = HeatmapData> + Clone + 'static> View for HeatmapChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        reactive_canvas(self.data, move |data| {
            Canvas::new(move |ctx| {
                draw_heatmap(ctx, &data);
            })
        })
    }
}
