//! Divider layout primitive shared across `WaterUI` surfaces.

use waterui_color::{BorderColor, Color};
use waterui_core::layout::StretchAxis;
use waterui_core::{Environment, View, raw_view};

use crate::{frame::Frame, stack};

/// A thin line that separates content.
///
/// Divider adapts to its parent container: in `VStack` it spans horizontally,
/// in `HStack` it spans vertically. The line is drawn in the theme's separator
/// colour — the [`BorderColor`] slot — so it follows the active theme and
/// colour scheme on every backend.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Divider;

// A divider always spans its parent stack's cross axis and stays one point
// thick on the main axis — which orientation that is resolves from
// `stack::Axis`, but the cross-axis answer is the same either way.
raw_view!(Divider, StretchAxis::CrossAxis);

impl Divider {
    /// The divider composed from `WaterUI` views: a one-point line in the
    /// separator colour, oriented across the enclosing stack. How a backend
    /// without a separator of its own realizes it.
    pub fn composed(self, env: &Environment) -> impl View {
        let vertical_divider = matches!(env.get::<stack::Axis>(), Some(stack::Axis::Horizontal));

        if vertical_divider {
            Frame::new(Color::new(BorderColor)).width(1.0)
        } else {
            Frame::new(Color::new(BorderColor)).height(1.0)
        }
    }
}
