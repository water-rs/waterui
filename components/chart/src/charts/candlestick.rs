//! Candlestick chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    candlestick_bounds, candlestick_geometry, draw_candlestick, interactive_cartesian_signal_canvas,
};
use crate::composition::ChartComposition;
use crate::data::Candle;
use crate::interaction::{
    CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings,
};

/// Candlestick chart for financial OHLC data.
pub struct CandlestickChart<S: Signal<Output = Vec<Candle>>> {
    data: S,
    bullish_color: Srgb,
    bearish_color: Srgb,
    selection: SelectionBindings<Candle>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<Candle>,
}

crate::charts::impl_chart_debug!(CandlestickChart, S, Vec<Candle>);

impl<S: Signal<Output = Vec<Candle>>> CandlestickChart<S> {
    /// Creates a candlestick chart from reactive OHLC candle data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            bullish_color: Srgb::from_hex("#22C55E"),
            bearish_color: Srgb::from_hex("#EF4444"),
            selection: SelectionBindings::default(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(Candle);

    /// Sets the color used for bullish candles.
    #[must_use]
    pub const fn bullish_color(mut self, color: Srgb) -> Self {
        self.bullish_color = color;
        self
    }

    /// Sets the color used for bearish candles.
    #[must_use]
    pub const fn bearish_color(mut self, color: Srgb) -> Self {
        self.bearish_color = color;
        self
    }

    /// Tracks the currently focused candle in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<Candle>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected candle in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<Candle>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<Candle>> + Clone + 'static> View for CandlestickChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let bullish_color = self.bullish_color;
        let bearish_color = self.bearish_color;
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            |data: &Vec<Candle>| candlestick_bounds(data),
            move |ctx, data, bounds| candlestick_geometry(ctx, data, bounds),
            move |ctx, data, geometry| {
                draw_candlestick(ctx, data, geometry.bounds, bullish_color, bearish_color);
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`CandlestickChart`]. Equivalent to [`CandlestickChart::new`].
#[must_use]
pub fn candlestick_chart<S: Signal<Output = Vec<Candle>>>(data: S) -> CandlestickChart<S> {
    CandlestickChart::new(data)
}
