//! Layout context shared by render nodes.

use waterui_core::Environment;

/// Context provided to nodes during the layout pass.
#[derive(Debug)]
pub struct LayoutCtx<'a> {
    env: &'a Environment,
}

impl<'a> LayoutCtx<'a> {
    /// Creates a new layout context.
    #[must_use]
    pub const fn new(env: &'a Environment) -> Self {
        Self { env }
    }

    /// Returns the environment associated with this layout pass.
    #[must_use]
    pub const fn env(&self) -> &'a Environment {
        self.env
    }
}

/// Two-dimensional size expressed in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    /// Width in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
}

impl Size {
    /// Creates a new [`Size`] using the provided dimensions.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }
}

/// Absolute coordinate relative to the parent node.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal position in logical pixels.
    pub x: f32,
    /// Vertical position in logical pixels.
    pub y: f32,
}

impl Point {
    /// Creates a new [`Point`].
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Rectangle origin.
    pub origin: Point,
    /// Rectangle size.
    pub size: Size,
}

impl Rect {
    /// Creates a new [`Rect`].
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }
}

impl Rect {
    /// Returns the maximum X coordinate of the rectangle.
    #[must_use]
    pub fn max_x(&self) -> f32 {
        self.origin.x + self.size.width
    }

    /// Returns the maximum Y coordinate of the rectangle.
    #[must_use]
    pub fn max_y(&self) -> f32 {
        self.origin.y + self.size.height
    }
}

/// Result returned by [`RenderNode::layout`](super::render::RenderNode::layout).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutResult {
    /// The measured size for this node.
    pub size: Size,
}
