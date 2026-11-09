//! Divider layout primitive shared across `WaterUI` surfaces.

use waterui_core::View;
use waterui_core::layout::StretchAxis;
use waterui_graphics::color::{BorderColor, Color};

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

impl View for Divider {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let vertical_divider = matches!(env.get::<stack::Axis>(), Some(stack::Axis::Horizontal));

        if vertical_divider {
            Frame::new(Color::new(BorderColor)).width(1.0)
        } else {
            Frame::new(Color::new(BorderColor)).height(1.0)
        }
    }

    /// A divider always spans its parent stack's cross axis and stays one
    /// point thick on the main axis — which orientation that is resolves in
    /// `body` from `stack::Axis`, but the cross-axis answer is the same either
    /// way.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::CrossAxis
    }
}
