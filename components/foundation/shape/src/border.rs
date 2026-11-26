//! Border modifier metadata for applying borders to views.

use nami::{Computed, SignalExt, signal::IntoSignal};
use waterui_core::{IntoSignalF32, metadata::MetadataKey};
use waterui_graphics::color::Color;
use waterui_layout::EdgeSet;

/// Represents a border effect that can be applied to UI elements.
///
/// A border is defined by its color, width, corner radius, and which edges
/// to draw the border on.
///
/// A border only strokes: it never clips the view it wraps, whatever the
/// corner radius. To cut content to a rounded shape, apply `.clip(shape)`
/// to the view before the border.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::prelude::*;
/// use waterui::border::Border;
///
/// // Create a simple border
/// let border = Border::new(Color::red(), 2.0);
///
/// // Create a rounded border on specific edges
/// let border = Border::new(Color::grey(), 1.0)
///     .corner_radius(8.0)
///     .edges(EdgeSet::BOTTOM);
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Border {
    /// The color of the border.
    color: Color,
    /// The width of the border in points.
    width: Computed<f32>,
    /// The corner radius for rounded borders (0 = square corners).
    corner_radius: Computed<f32>,
    /// Which edges to draw the border on (default: all edges).
    edges: EdgeSet,
}

impl Border {
    /// The color of the border.
    #[must_use]
    pub const fn color(&self) -> &Color {
        &self.color
    }

    /// The width of the border in points.
    #[must_use]
    pub const fn width(&self) -> &Computed<f32> {
        &self.width
    }

    /// The corner radius for rounded borders (0 = square corners).
    #[must_use]
    pub const fn get_corner_radius(&self) -> &Computed<f32> {
        &self.corner_radius
    }

    /// Which edges to draw the border on (default: all edges).
    #[must_use]
    pub const fn get_edges(&self) -> EdgeSet {
        self.edges
    }
}

impl MetadataKey for Border {}

impl Border {
    /// Creates a new border with the specified color and width.
    ///
    /// Corner radius defaults to 0 (square corners).
    /// All edges are included by default.
    ///
    /// # Arguments
    ///
    /// * `color` - The color of the border
    /// * `width` - The width of the border in points
    #[must_use]
    pub fn new(color: impl IntoSignal<Color> + 'static, width: impl IntoSignalF32) -> Self {
        Self {
            color: Color::from_signal(color),
            width: width.into_signal_f32().computed(),
            corner_radius: 0.0_f32.into_signal_f32().computed(),
            edges: EdgeSet::ALL,
        }
    }

    /// Sets the corner radius for this border.
    ///
    /// # Arguments
    ///
    /// * `radius` - The corner radius in points (0 = square corners)
    #[must_use]
    pub fn corner_radius(mut self, radius: impl IntoSignalF32) -> Self {
        self.corner_radius = radius.into_signal_f32().computed();
        self
    }

    /// Sets which edges to display the border on.
    ///
    /// # Arguments
    ///
    /// * `edges` - The edges to draw the border on
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::border::Border;
    ///
    /// // Border only on bottom edge
    /// let underline = Border::new(Color::grey(), 1.0).edges(EdgeSet::BOTTOM);
    ///
    /// // Border on horizontal edges (leading and trailing)
    /// let sides = Border::new(Color::grey(), 1.0).edges(EdgeSet::HORIZONTAL);
    /// ```
    #[must_use]
    pub const fn edges(mut self, edges: EdgeSet) -> Self {
        self.edges = edges;
        self
    }
}

impl Default for Border {
    fn default() -> Self {
        Self::new(Color::srgb(0, 0, 0), 1.0)
    }
}

#[cfg(test)]
mod tests {
    use nami::{Computed, Signal};
    use waterui_core::Environment;
    use waterui_graphics::color::Srgb;
    use waterui_graphics::color::install_color_signal;
    use waterui_graphics::color::token::Foreground;
    use waterui_layout::EdgeSet;

    use super::Border;

    #[test]
    fn border_clones_and_accepts_color_tokens() {
        let mut env = Environment::new();
        let expected = Srgb::new(0.1, 0.2, 0.3).resolve();
        install_color_signal::<Foreground>(&mut env, Computed::constant(expected));

        let border = Border::new(Foreground, 2.0)
            .corner_radius(8.0)
            .edges(EdgeSet::BOTTOM);
        let cloned = border.clone();
        assert_eq!(border.color().resolve(&env).snapshot(), expected);
        assert_eq!(cloned.color().resolve(&env).snapshot(), expected);
        assert!((cloned.width().snapshot() - 2.0).abs() < f32::EPSILON);
        assert!((cloned.get_corner_radius().snapshot() - 8.0).abs() < f32::EPSILON);
        assert_eq!(cloned.get_edges(), EdgeSet::BOTTOM);
    }
}
