//! Core layout primitives used by layout components.
//!
//! The layout system follows a simple two-pass protocol: first ask children how
//! large they would like to be given a [`ProposalSize`], then place them within
//! the final [`Size`]. This module defines the traits and helper types that are
//! shared by layout implementations across backends.

use core::fmt::Debug;

use alloc::vec::Vec;

// ============================================================================
// Safe Area Types
// ============================================================================

bitflags::bitflags! {
    /// Edges that can be selectively ignored for safe area
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct SafeAreaEdges: u8 {
        /// Top edge
        const TOP = 0b0001;
        /// Bottom edge
        const BOTTOM = 0b0010;
        /// Leading edge (left in LTR, right in RTL)
        const LEADING = 0b0100;
        /// Trailing edge (right in LTR, left in RTL)
        const TRAILING = 0b1000;
        /// Both horizontal edges
        const HORIZONTAL = Self::LEADING.bits() | Self::TRAILING.bits();
        /// Both vertical edges
        const VERTICAL = Self::TOP.bits() | Self::BOTTOM.bits();
        /// All edges
        const ALL = Self::HORIZONTAL.bits() | Self::VERTICAL.bits();
    }
}

/// Safe area insets in points, relative to the container bounds
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SafeAreaInsets {
    /// Top inset in points
    pub top: f32,
    /// Bottom inset in points
    pub bottom: f32,
    /// Leading inset in points (left in LTR)
    pub leading: f32,
    /// Trailing inset in points (right in LTR)
    pub trailing: f32,
}

impl SafeAreaInsets {
    /// Zero insets
    pub const ZERO: Self = Self {
        top: 0.0,
        bottom: 0.0,
        leading: 0.0,
        trailing: 0.0,
    };

    /// Creates new safe area insets
    #[must_use]
    pub const fn new(top: f32, bottom: f32, leading: f32, trailing: f32) -> Self {
        Self {
            top,
            bottom,
            leading,
            trailing,
        }
    }

    /// Inset a rect by the safe area
    #[must_use]
    pub fn inset(&self, rect: &Rect) -> Rect {
        Rect::new(
            Point::new(rect.x() + self.leading, rect.y() + self.top),
            Size::new(
                (rect.width() - self.leading - self.trailing).max(0.0),
                (rect.height() - self.top - self.bottom).max(0.0),
            ),
        )
    }

    /// Combine with another safe area (takes max of each edge)
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            top: self.top.max(other.top),
            bottom: self.bottom.max(other.bottom),
            leading: self.leading.max(other.leading),
            trailing: self.trailing.max(other.trailing),
        }
    }

    /// Zero out specific edges
    #[must_use]
    pub fn without(&self, edges: SafeAreaEdges) -> Self {
        Self {
            top: if edges.contains(SafeAreaEdges::TOP) { 0.0 } else { self.top },
            bottom: if edges.contains(SafeAreaEdges::BOTTOM) { 0.0 } else { self.bottom },
            leading: if edges.contains(SafeAreaEdges::LEADING) { 0.0 } else { self.leading },
            trailing: if edges.contains(SafeAreaEdges::TRAILING) { 0.0 } else { self.trailing },
        }
    }

    /// Check if all insets are zero
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.top == 0.0 && self.bottom == 0.0 && self.leading == 0.0 && self.trailing == 0.0
    }
}

/// Context passed to layout operations containing safe area and other layout state
#[derive(Debug, Clone, Default)]
pub struct LayoutContext {
    /// Safe area insets relative to this container's bounds
    pub safe_area: SafeAreaInsets,
    /// Which safe area edges this container ignores
    pub ignores_safe_area: SafeAreaEdges,
}

impl LayoutContext {
    /// Creates a new layout context with the given safe area
    #[must_use]
    pub const fn new(safe_area: SafeAreaInsets) -> Self {
        Self {
            safe_area,
            ignores_safe_area: SafeAreaEdges::empty(),
        }
    }

    /// Creates a layout context with zero safe area
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            safe_area: SafeAreaInsets::ZERO,
            ignores_safe_area: SafeAreaEdges::empty(),
        }
    }

    /// Returns the effective safe area (considering ignored edges)
    #[must_use]
    pub fn effective_safe_area(&self) -> SafeAreaInsets {
        self.safe_area.without(self.ignores_safe_area)
    }
}

// ============================================================================
// Child Metadata
// ============================================================================

/// Backend-supplied metrics that describe how a child responded to a layout
/// proposal.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChildMetadata {
    proposal: ProposalSize,
    priority: u8,
    stretch: bool,
}

impl ChildMetadata {
    /// Creates a metadata instance describing a single child.
    #[must_use]
    pub const fn new(proposal: ProposalSize, priority: u8, stretch: bool) -> Self {
        Self {
            proposal,
            priority,
            stretch,
        }
    }

    /// Returns the proposal that originated this metadata.
    #[must_use]
    pub const fn proposal(&self) -> &ProposalSize {
        &self.proposal
    }

    /// Shortcut for the proposed height.
    #[must_use]
    pub const fn proposal_height(&self) -> Option<f32> {
        self.proposal.height
    }

    /// Shortcut for the proposed width.
    #[must_use]
    pub const fn proposal_width(&self) -> Option<f32> {
        self.proposal.width
    }

    /// Priority hints future layout scheduling (unused for now).
    #[must_use]
    pub const fn priority(&self) -> u8 {
        self.priority
    }

    /// Whether the child is willing to expand beyond its intrinsic size.
    #[must_use]
    pub const fn stretch(&self) -> bool {
        self.stretch
    }
}

/// Behaviour shared by all layout containers.
pub trait Layout: Debug {
    /// Proposes sizes for each child based on the parent's proposal and the
    /// metadata collected during the previous frame.
    /// 
    /// The `context` contains safe area information for this container.
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<ProposalSize>;

    /// Computes the layout's own size after its children have answered the
    /// proposals created in [`propose`](Self::propose).
    /// 
    /// The `context` contains safe area information for this container.
    fn size(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Size;

    /// Places children within the final bounds chosen by the parent and
    /// returns the rectangles they should occupy along with their layout contexts.
    /// 
    /// Each child receives its own `LayoutContext` which may have different
    /// safe area values (e.g., first child in VStack gets top safe area,
    /// last child gets bottom safe area).
    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<ChildPlacement>;
}

/// Result of placing a child view
#[derive(Debug, Clone)]
pub struct ChildPlacement {
    /// The rectangle where the child should be placed
    pub rect: Rect,
    /// The layout context for the child (with updated safe area)
    pub context: LayoutContext,
}

impl ChildPlacement {
    /// Creates a new child placement
    #[must_use]
    pub const fn new(rect: Rect, context: LayoutContext) -> Self {
        Self { rect, context }
    }
    
    /// Creates a child placement with empty context (no safe area)
    #[must_use]
    pub fn with_empty_context(rect: Rect) -> Self {
        Self {
            rect,
            context: LayoutContext::empty(),
        }
    }
}

/// Axis-aligned rectangle relative to its parent.
#[derive(Clone, Debug, PartialEq)]
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

    /// Returns the rectangle's origin.
    #[must_use]
    pub const fn origin(&self) -> Point {
        self.origin
    }

    /// Returns the rectangle's size.
    #[must_use]
    pub const fn size(&self) -> &Size {
        &self.size
    }

    /// Returns the rectangle's x-coordinate.
    #[must_use]
    pub const fn x(&self) -> f32 {
        self.origin.x
    }
    /// Returns the rectangle's y-coordinate.
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
    /// Returns the rectangle's maximum x-coordinate.
    #[must_use]
    pub const fn max_x(&self) -> f32 {
        self.origin.x + self.size.width
    }
    /// Returns the rectangle's maximum y-coordinate.
    #[must_use]
    pub const fn max_y(&self) -> f32 {
        self.origin.y + self.size.height
    }
    /// Returns the rectangle's midpoint x-coordinate.
    #[must_use]
    pub const fn mid_x(&self) -> f32 {
        self.origin.x + self.size.width / 2.0
    }
    /// Returns the rectangle's midpoint y-coordinate.
    #[must_use]
    pub const fn mid_y(&self) -> f32 {
        self.origin.y + self.size.height / 2.0
    }
    /// Returns the rectangle's minimum x-coordinate.
    #[must_use]
    pub const fn min_x(&self) -> f32 {
        self.origin.x - self.size.width
    }
    /// Returns the rectangle's minimum y-coordinate.
    #[must_use]
    pub const fn min_y(&self) -> f32 {
        self.origin.y - self.size.height
    }
}

/// Two-dimensional size expressed in absolute pixels.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct Size {
    /// The width in pixels.
    pub width: f32,
    /// The height in pixels.
    pub height: f32,
}

impl Default for Size {
    fn default() -> Self {
        Self::zero()
    }
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
}

/// Absolute coordinate relative to a parent layout's origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// The x-coordinate in pixels.
    pub x: f32,
    /// The y-coordinate in pixels.
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

/// Soft constraint describing the desired size for a layout or subview.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProposalSize {
    /// Width constraint: `Some(f32)` for exact value, None for unconstrained, [`f32::INFINITY`] for infinite
    pub width: Option<f32>,
    /// Height constraint: `Some(f32)` for exact value, None for unconstrained, [`f32::INFINITY`] for infinite
    pub height: Option<f32>,
}

impl ProposalSize {
    /// Creates a [`ProposalSize`] from optional width and height hints.
    pub fn new(width: impl Into<Option<f32>>, height: impl Into<Option<f32>>) -> Self {
        Self {
            width: width.into(),
            height: height.into(),
        }
    }
}
