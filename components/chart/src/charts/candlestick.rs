//! Candlestick chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    candlestick_bounds, candlestick_geometry, draw_candlestick, interactive_signal_canvas,
};
use crate::data::Candle;
use crate::interaction::{HitResult, SelectionBindings};

/// Candlestick chart for financial OHLC data.
pub struct CandlestickChart<S: Signal<Output = Vec<Candle>>> {
    data: S,
    bullish_color: Srgb,
    bearish_color: Srgb,
    selection: SelectionBindings<Candle>,
}

impl<S: Signal<Output = Vec<Candle>>> CandlestickChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            bullish_color: Srgb::from_hex("#22C55E"),
            bearish_color: Srgb::from_hex("#EF4444"),
            selection: SelectionBindings::default(),
        }
    }

    #[must_use]
    pub fn bullish_color(mut self, color: Srgb) -> Self {
        self.bullish_color = color;
        self
    }

    #[must_use]
    pub fn bearish_color(mut self, color: Srgb) -> Self {
        self.bearish_color = color;
        self
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<Candle>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<Candle>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<Candle>> + Clone + 'static> View for CandlestickChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let bullish_color = self.bullish_color;
        let bearish_color = self.bearish_color;
        interactive_signal_canvas(
            self.data,
            move |ctx, data| {
                let bounds = candlestick_bounds(data);
                candlestick_geometry(ctx, data, bounds)
            },
            move |ctx, data, geometry| {
                draw_candlestick(ctx, data, geometry.bounds, bullish_color, bearish_color);
            },
            self.selection,
        )
    }
}
