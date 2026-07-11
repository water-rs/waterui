//! Depth (order book) chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    depth_bounds, depth_geometry, draw_depth, interactive_cartesian_signal_canvas,
};
use crate::composition::ChartComposition;
use crate::data::DepthData;
use crate::interaction::{
    CartesianSelectionBindings, CartesianViewportBindings, DepthDatum, HitResult, SelectionBindings,
};

/// Depth chart for order book visualization.
pub struct DepthChart<S: Signal<Output = DepthData>> {
    data: S,
    bid_color: Srgb,
    ask_color: Srgb,
    selection: SelectionBindings<DepthDatum>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<DepthDatum>,
}

crate::charts::impl_chart_debug!(DepthChart, S, DepthData);

impl<S: Signal<Output = DepthData>> DepthChart<S> {
    /// Creates a depth chart from reactive order-book data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            bid_color: Srgb::from_hex("#22C55E"),
            ask_color: Srgb::from_hex("#EF4444"),
            selection: SelectionBindings::default(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(DepthDatum);

    /// Sets the bid-side fill color.
    #[must_use]
    pub const fn bid_color(mut self, color: Srgb) -> Self {
        self.bid_color = color;
        self
    }

    /// Sets the ask-side fill color.
    #[must_use]
    pub const fn ask_color(mut self, color: Srgb) -> Self {
        self.ask_color = color;
        self
    }

    /// Sets both bid and ask colors.
    #[must_use]
    pub const fn colors(mut self, bid: Srgb, ask: Srgb) -> Self {
        self.bid_color = bid;
        self.ask_color = ask;
        self
    }

    /// Tracks the currently focused depth level in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DepthDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected depth level in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DepthDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = DepthData> + Clone + 'static> View for DepthChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let bid_color = self.bid_color;
        let ask_color = self.ask_color;
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            depth_bounds,
            depth_geometry,
            move |ctx, data, geometry, transition_alpha| {
                draw_depth(
                    ctx,
                    data,
                    geometry.bounds,
                    bid_color,
                    ask_color,
                    transition_alpha,
                );
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`DepthChart`]. Equivalent to [`DepthChart::new`].
#[must_use]
pub fn depth_chart<S: Signal<Output = DepthData>>(data: S) -> DepthChart<S> {
    DepthChart::new(data)
}
