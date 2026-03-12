//! Depth (order book) chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{depth_bounds, depth_geometry, draw_depth, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::DepthData;
use crate::interaction::{DepthDatum, HitResult, SelectionBindings};

/// Depth chart for order book visualization.
pub struct DepthChart<S: Signal<Output = DepthData>> {
    data: S,
    bid_color: Srgb,
    ask_color: Srgb,
    selection: SelectionBindings<DepthDatum>,
    composition: ChartComposition<DepthDatum>,
}

impl<S: Signal<Output = DepthData>> DepthChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            bid_color: Srgb::from_hex("#22C55E"),
            ask_color: Srgb::from_hex("#EF4444"),
            selection: SelectionBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(DepthDatum);

    #[must_use]
    pub fn bid_color(mut self, color: Srgb) -> Self {
        self.bid_color = color;
        self
    }

    #[must_use]
    pub fn ask_color(mut self, color: Srgb) -> Self {
        self.ask_color = color;
        self
    }

    #[must_use]
    pub fn colors(mut self, bid: Srgb, ask: Srgb) -> Self {
        self.bid_color = bid;
        self.ask_color = ask;
        self
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DepthDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DepthDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = DepthData> + Clone + 'static> View for DepthChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let bid_color = self.bid_color;
        let ask_color = self.ask_color;
        interactive_signal_canvas(
            _env,
            self.data,
            move |ctx, data| {
                let bounds = depth_bounds(data);
                depth_geometry(ctx, data, bounds)
            },
            move |ctx, data, geometry| {
                draw_depth(ctx, data, geometry.bounds, bid_color, ask_color);
            },
            self.selection,
            self.composition,
        )
    }
}
