//! Candlestick (K-line) chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_canvas::Canvas;
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_candlestick, reactive_canvas};
use crate::data::Candle;

/// Candlestick (K-line) chart for financial data.
///
/// Renders OHLCV data as traditional candlestick charts with optional volume bars.
/// Uses GPU-accelerated rendering for smooth 120fps performance.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{CandlestickChart, Candle};
///
/// let candles = binding(vec![
///     Candle::new(1609459200.0, 29000.0, 29500.0, 28800.0, 29300.0, 1000.0),
///     Candle::new(1609545600.0, 29300.0, 30000.0, 29100.0, 29800.0, 1200.0),
/// ]);
///
/// CandlestickChart::new(&candles)
///     .bullish_color(Srgb::from_hex("#22C55E"))
///     .bearish_color(Srgb::from_hex("#EF4444"))
/// ```
pub struct CandlestickChart<S: Signal<Output = Vec<Candle>>> {
    data: S,
    bullish_color: Srgb,
    bearish_color: Srgb,
}

impl<S: Signal<Output = Vec<Candle>>> CandlestickChart<S> {
    /// Creates a new candlestick chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            bullish_color: Srgb::from_hex("#22C55E"), // Green
            bearish_color: Srgb::from_hex("#EF4444"), // Red
        }
    }

    /// Sets the bullish (up) candle color.
    #[must_use]
    pub fn bullish_color(mut self, color: Srgb) -> Self {
        self.bullish_color = color;
        self
    }

    /// Sets the bearish (down) candle color.
    #[must_use]
    pub fn bearish_color(mut self, color: Srgb) -> Self {
        self.bearish_color = color;
        self
    }

    /// Sets both bullish and bearish colors.
    #[must_use]
    pub fn colors(mut self, bullish: Srgb, bearish: Srgb) -> Self {
        self.bullish_color = bullish;
        self.bearish_color = bearish;
        self
    }
}

impl<S: Signal<Output = Vec<Candle>> + Clone + 'static> View for CandlestickChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let bullish_color = self.bullish_color;
        let bearish_color = self.bearish_color;
        reactive_canvas(self.data, move |data| {
            Canvas::new(move |ctx| {
                draw_candlestick(ctx, &data, bullish_color, bearish_color);
            })
        })
    }
}
