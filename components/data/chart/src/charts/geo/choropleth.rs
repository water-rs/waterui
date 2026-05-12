//! Choropleth chart view component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{choropleth_geometry, draw_choropleth, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::ChoroplethData;
use crate::interaction::{HitResult, RegionDatum, SelectionBindings};
use crate::params::PositiveF32;

/// Choropleth map chart for geographic data visualization.
pub struct ChoroplethChart<S: Signal<Output = ChoroplethData>> {
    data: S,
    stroke_width: f32,
    stroke_color: Srgb,
    show_stroke: bool,
    selection: SelectionBindings<RegionDatum>,
    composition: ChartComposition<RegionDatum>,
}

crate::charts::impl_chart_debug!(ChoroplethChart, S, ChoroplethData);

impl<S: Signal<Output = ChoroplethData>> ChoroplethChart<S> {
    /// Creates a choropleth chart from reactive geographic data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            stroke_width: 1.0,
            stroke_color: Srgb::new(1.0, 1.0, 1.0),
            show_stroke: true,
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(RegionDatum);

    /// Sets the polygon stroke width.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values.
    #[must_use]
    pub fn stroke_width(mut self, width: impl Into<PositiveF32>) -> Self {
        self.stroke_width = width.into().get();
        self
    }

    /// Sets the polygon stroke color.
    #[must_use]
    pub const fn stroke_color(mut self, color: Srgb) -> Self {
        self.stroke_color = color;
        self
    }

    /// Enables or disables polygon stroke rendering.
    #[must_use]
    pub const fn show_stroke(mut self, show: bool) -> Self {
        self.show_stroke = show;
        self
    }

    /// Tracks the currently focused region in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<RegionDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected region in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<RegionDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = ChoroplethData> + Clone + 'static> View for ChoroplethChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let stroke_width = self.stroke_width;
        let stroke_color = self.stroke_color;
        let show_stroke = self.show_stroke;
        interactive_signal_canvas(
            env,
            self.data,
            choropleth_geometry,
            move |ctx, data, _geometry| {
                draw_choropleth(ctx, data, stroke_width, stroke_color, show_stroke);
            },
            self.selection,
            self.composition,
        )
    }
}

/// Convenience constructor for [`ChoroplethChart`]. Equivalent to [`ChoroplethChart::new`].
#[must_use]
pub fn choropleth_chart<S: Signal<Output = ChoroplethData>>(data: S) -> ChoroplethChart<S> {
    ChoroplethChart::new(data)
}
