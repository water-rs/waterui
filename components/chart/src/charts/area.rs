//! Area chart component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_canvas::Canvas;

use crate::charts::canvas::{draw_area, reactive_canvas};
use crate::data::AreaData;

/// Stacked area chart for cumulative data visualization.
///
/// Displays multiple data series as filled areas stacked on top of each other,
/// showing both individual contributions and the total over time.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{AreaChart, AreaData, AreaSeries};
///
/// let data = binding(
///     AreaData::new(vec![1.0, 2.0, 3.0, 4.0, 5.0])
///         .series(AreaSeries::new("Sales", vec![10.0, 20.0, 15.0, 25.0, 30.0])
///             .color(0.23, 0.51, 0.96, 0.7))
///         .series(AreaSeries::new("Marketing", vec![5.0, 10.0, 8.0, 12.0, 15.0])
///             .color(0.94, 0.27, 0.27, 0.7))
/// );
///
/// AreaChart::new(data)
///     .stacked(true)
/// ```
pub struct AreaChart<S: Signal<Output = AreaData>> {
    data: S,
}

impl<S: Signal<Output = AreaData>> AreaChart<S> {
    /// Creates a new area chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self { data }
    }
}

impl<S: Signal<Output = AreaData> + Clone + 'static> View for AreaChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        reactive_canvas(self.data, move |data| {
            Canvas::new(move |ctx| {
                draw_area(ctx, &data);
            })
        })
    }
}
