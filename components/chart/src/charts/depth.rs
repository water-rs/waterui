//! Depth (order book) chart component.

use nami::Signal;
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{depth_bounds, draw_depth, interactive_cartesian_canvas};
use crate::data::DepthData;

/// Depth chart for order book visualization.
///
/// Renders bid and ask levels as filled step-area charts meeting at the mid price.
/// Bids are shown in green (left), asks in red (right).
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{DepthChart, DepthData, DepthLevel};
///
/// let depth = binding(DepthData::new(
///     vec![
///         DepthLevel::new(100.0, 1000.0),  // Best bid
///         DepthLevel::new(99.0, 2500.0),   // Cumulative volume
///         DepthLevel::new(98.0, 4000.0),
///     ],
///     vec![
///         DepthLevel::new(101.0, 800.0),   // Best ask
///         DepthLevel::new(102.0, 2000.0),
///         DepthLevel::new(103.0, 3500.0),
///     ],
/// ));
///
/// DepthChart::new(depth)
///     .bid_color(Srgb::from_hex("#22C55E"))
///     .ask_color(Srgb::from_hex("#EF4444"))
/// ```
pub struct DepthChart<S: Signal<Output = DepthData>> {
    data: S,
    bid_color: Srgb,
    ask_color: Srgb,
}

impl<S: Signal<Output = DepthData>> DepthChart<S> {
    /// Creates a new depth chart with the given data source.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            bid_color: Srgb::from_hex("#22C55E"), // Green
            ask_color: Srgb::from_hex("#EF4444"), // Red
        }
    }

    /// Sets the bid (buy) color.
    #[must_use]
    pub fn bid_color(mut self, color: Srgb) -> Self {
        self.bid_color = color;
        self
    }

    /// Sets the ask (sell) color.
    #[must_use]
    pub fn ask_color(mut self, color: Srgb) -> Self {
        self.ask_color = color;
        self
    }

    /// Sets both bid and ask colors.
    #[must_use]
    pub fn colors(mut self, bid: Srgb, ask: Srgb) -> Self {
        self.bid_color = bid;
        self.ask_color = ask;
        self
    }
}

impl<S: Signal<Output = DepthData> + Clone + 'static> View for DepthChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let bid_color = self.bid_color;
        let ask_color = self.ask_color;
        interactive_cartesian_canvas(self.data, depth_bounds, move |ctx, data, bounds| {
            draw_depth(ctx, data, bounds, bid_color, ask_color);
        })
    }
}
