//! Border modifier for applying borders to views.
//!
//! # Examples
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::border::Border;
//!
//! fn border_example() -> impl View {
//!     // Simple border
//!     text!("Hello")
//!         .border(Color::red(), 2.0)
//! }
//!
//! fn rounded_border_example() -> impl View {
//!     // Rounded border
//!     text!("Rounded")
//!         .border_with(Border::new(Color::blue(), 1.0).corner_radius(8.0))
//! }
//! ```

use waterui_core::metadata::MetadataKey;
use waterui_graphics::color::Color;
use waterui_layout::EdgeSet;

/// Represents a border effect that can be applied to UI elements.
///
/// A border is defined by its color, width, corner radius, and which edges
/// to draw the border on.
///
/// # Examples
///
/// ```rust
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
#[derive(Debug)]
pub struct Border {
    /// The color of the border.
    pub color: Color,
    /// The width of the border in points.
    pub width: f32,
    /// The corner radius for rounded borders (0 = square corners).
    pub corner_radius: f32,
    /// Which edges to draw the border on (default: all edges).
    pub edges: EdgeSet,
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
    pub fn new(color: impl Into<Color>, width: f32) -> Self {
        Self {
            color: color.into(),
            width,
            corner_radius: 0.0,
            edges: EdgeSet::ALL,
        }
    }

    /// Sets the corner radius for this border.
    ///
    /// # Arguments
    ///
    /// * `radius` - The corner radius in points (0 = square corners)
    #[must_use]
    pub const fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
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
    /// ```rust
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
        Self {
            color: Color::srgb(0, 0, 0),
            width: 1.0,
            corner_radius: 0.0,
            edges: EdgeSet::ALL,
        }
    }
}
