//! Pie chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_pie, interactive_signal_canvas, pie_geometry};
use crate::composition::ChartComposition;
use crate::data::DataPoint;
use crate::interaction::{HitResult, SelectionBindings, SliceDatum};
use crate::params::DonutInnerRadius;

/// Pie chart visualization.
pub struct PieChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    colors: Vec<Srgb>,
    inner_radius: f32,
    selection: SelectionBindings<SliceDatum>,
    composition: ChartComposition<SliceDatum>,
}

crate::charts::impl_chart_debug!(PieChart, S, Vec<DataPoint>);

impl<S: Signal<Output = Vec<DataPoint>>> PieChart<S> {
    /// Creates a pie chart from reactive scalar data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            colors: Vec::new(),
            inner_radius: 0.0,
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(SliceDatum);

    /// Sets the slice color palette.
    #[must_use]
    pub fn colors(mut self, colors: Vec<Srgb>) -> Self {
        self.colors = colors;
        self
    }

    /// Converts the pie into a donut chart with the given inner-radius ratio.
    ///
    /// Accepts any value convertible into [`DonutInnerRadius`]. Passing a
    /// raw `f32` panics on `NaN`, infinity, or values outside `[0.0, 0.95]`.
    #[must_use]
    pub fn donut(mut self, inner_radius: impl Into<DonutInnerRadius>) -> Self {
        self.inner_radius = inner_radius.into().get();
        self
    }

    /// Renders the chart as a full pie without a donut hole.
    #[must_use]
    pub const fn full_pie(mut self) -> Self {
        self.inner_radius = 0.0;
        self
    }

    /// Tracks the currently focused slice in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected slice in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for PieChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let colors = self.colors;
        let inner_radius = self.inner_radius;
        interactive_signal_canvas(
            env,
            self.data,
            move |ctx, data| pie_geometry(ctx, data, inner_radius),
            move |ctx, data, _geometry| {
                let colors = colors.clone();
                draw_pie(ctx, data, &colors, inner_radius);
            },
            self.selection,
            self.composition,
        )
    }
}

/// Convenience constructor for [`PieChart`]. Equivalent to [`PieChart::new`].
#[must_use]
pub fn pie_chart<S: Signal<Output = Vec<DataPoint>>>(data: S) -> PieChart<S> {
    PieChart::new(data)
}
