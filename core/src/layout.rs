//! Layout primitives and geometry types for the `WaterUI` layout system.
//!
//! # Logical Pixels (Points)
//!
//! All layout values in `WaterUI` use **logical pixels** (also called "points" or "dp").
//! This is the same unit system used by design tools like Figma, Sketch, and Adobe XD,
//! allowing seamless translation from design to implementation.
//!
//! - **1 logical pixel** = 1 point in design tools
//! - Native backends handle conversion to physical pixels based on screen density
//! - iOS: `UIKit` uses points natively (1pt = 1-3 physical pixels depending on device)
//! - Android: Backend converts dp to physical pixels using `displayMetrics.density`
//! - macOS: `AppKit` uses points (1pt = 1-2 physical pixels on Retina displays)
//!
//! This means `spacing: 8.0` or `width: 100.0` will appear the same physical size
//! across all platforms and screen densities.
//!
//! # Example
//!
//! ```ignore
//! // In Figma: Button with 16pt horizontal padding, 8pt vertical padding
//! // In WaterUI: Same values work directly
//! vstack((
//!     text("Hello").padding(16.0),  // 16 logical pixels = 16pt in Figma
//!     Divider,                       // 1pt thick line
//! )).spacing(8.0)                    // 8 logical pixels between items
//! ```

use core::fmt::Debug;

use alloc::vec::Vec;

// ============================================================================
// StretchAxis - Specifies which axis a view stretches on
// ============================================================================

/// Specifies which axis (or axes) a view wants to stretch to fill available space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StretchAxis {
    /// No stretching - view uses its intrinsic size
    #[default]
    None,
    /// Stretch horizontally only (expand width, use intrinsic height)
    Horizontal,
    /// Stretch vertically only (expand height, use intrinsic width)
    Vertical,
    /// Stretch in both directions (expand width and height)
    Both,
    /// Stretch along the parent container's main axis.
    /// In `VStack`: expands vertically. In `HStack`: expands horizontally.
    /// Used by Spacer.
    MainAxis,
    /// Stretch along the parent container's cross axis.
    /// In `VStack`: expands horizontally. In `HStack`: expands vertically.
    /// Used by Divider.
    CrossAxis,
}

impl StretchAxis {
    /// Returns true if this stretches horizontally.
    #[must_use]
    pub const fn stretches_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    /// Returns true if this stretches vertically.
    #[must_use]
    pub const fn stretches_vertical(&self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    /// Returns true if this stretches in any direction.
    #[must_use]
    pub const fn stretches_any(&self) -> bool {
        !matches!(self, Self::None)
    }
}

// ============================================================================
// SubView Trait - Child View Proxy
// ============================================================================

/// A proxy for querying child view sizes during layout.
///
/// This trait allows layout containers to negotiate with children by asking
/// "if I propose this size, how big would you be?" multiple times with
/// different proposals.
///
/// # Pure Functions
///
/// All methods are pure (take `&self`) with no side effects. Caching of
/// measurement results is handled by the native backend, not in Rust.
pub trait SubView {
    /// Query the child's size for a given proposal.
    ///
    /// This method may be called multiple times with different proposals
    /// to probe the child's flexibility:
    ///
    /// - `ProposalSize::new(None, None)` - ideal/intrinsic size
    /// - `ProposalSize::new(Some(0.0), None)` - minimum width
    /// - `ProposalSize::new(Some(f32::INFINITY), None)` - maximum width
    /// - `ProposalSize::new(Some(200.0), None)` - constrained width
    fn size_that_fits(&self, proposal: ProposalSize) -> Size;

    /// Which axis (or axes) this view stretches to fill available space.
    ///
    /// - `StretchAxis::None`: Content-sized, uses intrinsic size
    /// - `StretchAxis::Horizontal`: Expands width only (e.g., `TextField`, Slider)
    /// - `StretchAxis::Vertical`: Expands height only
    /// - `StretchAxis::Both`: Greedy, fills all space (e.g., Spacer, Color)
    ///
    /// Layout containers use this to distribute remaining space appropriately:
    /// - `VStack` checks `stretches_vertical()` for height distribution
    /// - `HStack` checks `stretches_horizontal()` for width distribution
    fn stretch_axis(&self) -> StretchAxis;

    /// Layout priority for space distribution.
    ///
    /// Higher priority views are measured first and get space preference.
    fn priority(&self) -> i32;
}

// ============================================================================
// Layout Trait - Container Layout
// ============================================================================

/// A layout algorithm for arranging child views.
///
/// Layouts receive a size proposal from their parent, query their children
/// to determine sizes, and then place children within the final bounds.
///
/// # Two-Phase Layout
///
/// 1. **Sizing** ([`size_that_fits`](Self::size_that_fits)): Determine how big
///    this container should be given a proposal
/// 2. **Placement** ([`place`](Self::place)): Position children within the
///    final bounds
///
/// # Note on Safe Area
///
/// Safe area handling is intentionally **not** part of the Layout trait.
/// Safe area is a platform-specific concept handled by backends. Views can
/// use the `IgnoresSafeArea` metadata to opt out of safe area insets.
pub trait Layout: Debug {
    /// Calculate the size this layout wants given a proposal.
    ///
    /// The layout can query children multiple times with different proposals
    /// to determine optimal sizing.
    ///
    /// # Arguments
    ///
    /// * `proposal` - The size proposed by the parent
    /// * `children` - References to child proxies for size queries
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size;

    /// Place children within the given bounds.
    ///
    /// Called after sizing is complete. Returns a rect for each child
    /// specifying its position and size within `bounds`.
    ///
    /// # Arguments
    ///
    /// * `bounds` - The rectangle this layout should fill
    /// * `children` - References to child proxies (may query sizes again)
    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect>;

    /// Which axis this container stretches to fill available space.
    ///
    /// - `VStack`: `.horizontal` (fills available width, intrinsic height)
    /// - `HStack`: `.vertical` (fills available height, intrinsic width)
    /// - `ZStack`: `.both` (fills all available space)
    /// - Other layouts: `.none` by default
    ///
    /// This allows parent containers to know whether to expand this container
    /// to fill available space on the cross axis.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}

// ============================================================================
// Geometry Types
// ============================================================================

/// Axis-aligned rectangle relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    origin: Point,
    size: Size,
}

impl Rect {
    /// Creates a new [`Rect`] with the provided `origin` and `size`.
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Creates a rectangle from origin (0, 0) with the given size.
    #[must_use]
    pub const fn from_size(size: Size) -> Self {
        Self {
            origin: Point::zero(),
            size,
        }
    }

    /// Returns the rectangle's origin (top-left corner).
    #[must_use]
    pub const fn origin(&self) -> Point {
        self.origin
    }

    /// Returns the rectangle's size.
    #[must_use]
    pub const fn size(&self) -> &Size {
        &self.size
    }

    /// Returns the rectangle's x-coordinate (left edge).
    #[must_use]
    pub const fn x(&self) -> f32 {
        self.origin.x
    }

    /// Returns the rectangle's y-coordinate (top edge).
    #[must_use]
    pub const fn y(&self) -> f32 {
        self.origin.y
    }

    /// Returns the rectangle's width.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.size.width
    }

    /// Returns the rectangle's height.
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.size.height
    }

    /// Returns the minimum x-coordinate (left edge).
    #[must_use]
    pub const fn min_x(&self) -> f32 {
        self.origin.x
    }

    /// Returns the minimum y-coordinate (top edge).
    #[must_use]
    pub const fn min_y(&self) -> f32 {
        self.origin.y
    }

    /// Returns the maximum x-coordinate (right edge).
    #[must_use]
    pub const fn max_x(&self) -> f32 {
        self.origin.x + self.size.width
    }

    /// Returns the maximum y-coordinate (bottom edge).
    #[must_use]
    pub const fn max_y(&self) -> f32 {
        self.origin.y + self.size.height
    }

    /// Returns the midpoint x-coordinate.
    #[must_use]
    pub const fn mid_x(&self) -> f32 {
        self.origin.x + self.size.width / 2.0
    }

    /// Returns the midpoint y-coordinate.
    #[must_use]
    pub const fn mid_y(&self) -> f32 {
        self.origin.y + self.size.height / 2.0
    }

    /// Returns the center point of the rectangle.
    #[must_use]
    pub const fn center(&self) -> Point {
        Point::new(self.mid_x(), self.mid_y())
    }

    /// Inset the rectangle by the given amounts on each edge.
    #[must_use]
    pub fn inset(&self, top: f32, bottom: f32, leading: f32, trailing: f32) -> Self {
        Self::new(
            Point::new(self.origin.x + leading, self.origin.y + top),
            Size::new(
                (self.size.width - leading - trailing).max(0.0),
                (self.size.height - top - bottom).max(0.0),
            ),
        )
    }
}

// ============================================================================
// Size
// ============================================================================

/// Two-dimensional size expressed in points.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
pub struct Size {
    /// The width in points.
    pub width: f32,
    /// The height in points.
    pub height: f32,
}

impl Size {
    /// Constructs a [`Size`] with the given `width` and `height`.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Creates a [`Size`] with zero width and height.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }

    /// Returns true if both dimensions are zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.width == 0.0 && self.height == 0.0
    }
}

// ============================================================================
// Point
// ============================================================================

/// Absolute coordinate relative to a parent layout's origin.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point {
    /// The x-coordinate in points.
    pub x: f32,
    /// The y-coordinate in points.
    pub y: f32,
}

impl Point {
    /// Constructs a [`Point`] at the given `x` and `y`.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Creates a [`Point`] at the origin (0, 0).
    #[must_use]
    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

// ============================================================================
// ProposalSize
// ============================================================================

/// A size proposal from parent to child during layout negotiation.
///
/// Each dimension can be:
/// - `None` - "Tell me your ideal size" (unspecified)
/// - `Some(0.0)` - "Tell me your minimum size"
/// - `Some(f32::INFINITY)` - "Tell me your maximum size"
/// - `Some(value)` - "I suggest you use this size"
///
/// Children are free to return any size; the proposal is just a suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ProposalSize {
    /// Width proposal: `None` = unspecified, `Some(f32)` = suggested width
    pub width: Option<f32>,
    /// Height proposal: `None` = unspecified, `Some(f32)` = suggested height
    pub height: Option<f32>,
}

impl ProposalSize {
    /// Creates a [`ProposalSize`] from optional width and height.
    #[must_use]
    pub fn new(width: impl Into<Option<f32>>, height: impl Into<Option<f32>>) -> Self {
        Self {
            width: width.into(),
            height: height.into(),
        }
    }

    /// Unspecified proposal - asks for ideal/intrinsic size.
    pub const UNSPECIFIED: Self = Self {
        width: None,
        height: None,
    };

    /// Zero proposal - asks for minimum size.
    pub const ZERO: Self = Self {
        width: Some(0.0),
        height: Some(0.0),
    };

    /// Infinite proposal - asks for maximum size.
    pub const INFINITY: Self = Self {
        width: Some(f32::INFINITY),
        height: Some(f32::INFINITY),
    };

    /// Returns the width or a default value if unspecified.
    #[must_use]
    pub fn width_or(&self, default: f32) -> f32 {
        self.width.unwrap_or(default)
    }

    /// Returns the height or a default value if unspecified.
    #[must_use]
    pub fn height_or(&self, default: f32) -> f32 {
        self.height.unwrap_or(default)
    }

    /// Replace only the width, keeping the height.
    #[must_use]
    pub const fn with_width(self, width: Option<f32>) -> Self {
        Self {
            width,
            height: self.height,
        }
    }

    /// Replace only the height, keeping the width.
    #[must_use]
    pub const fn with_height(self, height: Option<f32>) -> Self {
        Self {
            width: self.width,
            height,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_geometry() {
        let rect = Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 50.0));

        assert_eq!(rect.min_x(), 10.0);
        assert_eq!(rect.min_y(), 20.0);
        assert_eq!(rect.max_x(), 110.0);
        assert_eq!(rect.max_y(), 70.0);
        assert_eq!(rect.mid_x(), 60.0);
        assert_eq!(rect.mid_y(), 45.0);
        assert_eq!(rect.width(), 100.0);
        assert_eq!(rect.height(), 50.0);
    }

    #[test]
    fn test_rect_inset() {
        let rect = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let inset = rect.inset(10.0, 10.0, 20.0, 20.0);

        assert_eq!(inset.x(), 20.0);
        assert_eq!(inset.y(), 10.0);
        assert_eq!(inset.width(), 60.0);
        assert_eq!(inset.height(), 80.0);
    }

    #[test]
    fn test_proposal_size() {
        let proposal = ProposalSize::new(Some(100.0), None);

        assert_eq!(proposal.width_or(0.0), 100.0);
        assert_eq!(proposal.height_or(50.0), 50.0);

        let with_height = proposal.with_height(Some(200.0));
        assert_eq!(with_height.width, Some(100.0));
        assert_eq!(with_height.height, Some(200.0));
    }
}
