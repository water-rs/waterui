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
use crate::params::{ChartParamError, DonutInnerRadius};

/// Pie chart visualization.
pub struct PieChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    colors: Vec<Srgb>,
    inner_radius: f32,
    selection: SelectionBindings<SliceDatum>,
    composition: ChartComposition<SliceDatum>,
}

impl<S: Signal<Output = Vec<DataPoint>>> PieChart<S> {
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

    #[must_use]
    pub fn colors(mut self, colors: Vec<Srgb>) -> Self {
        self.colors = colors;
        self
    }

    #[must_use]
    pub fn donut(self, inner_radius: f32) -> Self {
        self.try_donut(inner_radius)
            .expect("PieChart::donut(inner_radius) requires finite 0.0 <= inner_radius <= 0.95")
    }

    #[must_use]
    pub fn with_donut(mut self, inner_radius: DonutInnerRadius) -> Self {
        self.inner_radius = inner_radius.get();
        self
    }

    pub fn try_donut(self, inner_radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_donut(DonutInnerRadius::try_new(inner_radius)?))
    }

    #[must_use]
    pub fn full_pie(mut self) -> Self {
        self.inner_radius = 0.0;
        self
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<SliceDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for PieChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let colors = self.colors;
        let inner_radius = self.inner_radius;
        interactive_signal_canvas(
            _env,
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
