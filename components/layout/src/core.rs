//! Core layout primitives used by layout components.
//!
//! The layout system follows a simple two-pass protocol: first ask children how
//! large they would like to be given a [`ProposalSize`], then place them within
//! the final [`Size`]. This module defines the traits and helper types that are
//! shared by layout implementations across backends.

use core::fmt::Debug;

use alloc::vec::Vec;

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
    pub const fn proposal_height(&self) -> Option<f64> {
        self.proposal.height
    }

    /// Shortcut for the proposed width.
    #[must_use]
    pub const fn proposal_width(&self) -> Option<f64> {
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
pub trait Layout {
    /// Proposes sizes for each child based on the parent's proposal and the
    /// metadata collected during the previous frame.
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize>;

    /// Computes the layout's own size after its children have answered the
    /// proposals created in [`propose`](Self::propose).
    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size;

    /// Places children within the final bounds chosen by the parent and
    /// returns the rectangles they should occupy.
    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect>;
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
    pub const fn x(&self) -> f64 {
        self.origin.x
    }
    /// Returns the rectangle's y-coordinate.
    #[must_use]
    pub const fn y(&self) -> f64 {
        self.origin.y
    }
    
    /// Returns the rectangle's width.
    #[must_use]
    pub const fn width(&self) -> f64 {
        self.size.width
    }
    /// Returns the rectangle's height.
    #[must_use]
    pub const fn height(&self) -> f64 {
        self.size.height
    }
    /// Returns the rectangle's maximum x-coordinate.
    #[must_use]
    pub const fn max_x(&self) -> f64 {
        self.origin.x + self.size.width
    }
    /// Returns the rectangle's maximum y-coordinate.
    #[must_use]
    pub const fn max_y(&self) -> f64 {
        self.origin.y + self.size.height
    }
    /// Returns the rectangle's midpoint x-coordinate.
    #[must_use]
    pub const fn mid_x(&self) -> f64 {
        self.origin.x + self.size.width / 2.0
    }
    /// Returns the rectangle's midpoint y-coordinate.
    #[must_use]
    pub const fn mid_y(&self) -> f64 {
        self.origin.y + self.size.height / 2.0
    }
    /// Returns the rectangle's minimum x-coordinate.
    #[must_use]
    pub const fn min_x(&self) -> f64 {
        self.origin.x - self.size.width
    }
    /// Returns the rectangle's minimum y-coordinate.
    #[must_use]
    pub const fn min_y(&self) -> f64 {
        self.origin.y - self.size.height
    }
}

/// Two-dimensional size expressed in absolute pixels.
#[derive(Clone, Debug, PartialEq,PartialOrd)]
pub struct Size {
    /// The width in pixels.
    pub width: f64,
    /// The height in pixels.
    pub height: f64,
}

impl Default for Size {
    fn default() -> Self {
        Self::zero()
    }
}

impl Size {
    /// Constructs a [`Size`] with the given `width` and `height`.
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    /// Creates a [`Size`] with zero width and height.
    #[must_use] 
    pub const fn zero() -> Self {
        Self { width: 0.0, height: 0.0 }
    }
}

/// Absolute coordinate relative to a parent layout's origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// The x-coordinate in pixels.
    pub x: f64,
    /// The y-coordinate in pixels.
    pub y: f64,
}

impl Point {
    /// Constructs a [`Point`] at the given `x` and `y`.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
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
    /// Width constraint: `Some(f64)` for exact value, None for unconstrained, [`f64::INFINITY`] for infinite
    pub width: Option<f64>,
    /// Height constraint: `Some(f64)` for exact value, None for unconstrained, [`f64::INFINITY`] for infinite
    pub height: Option<f64>,
}

impl ProposalSize {
    /// Creates a [`ProposalSize`] from optional width and height hints.
    pub fn new(width: impl Into<Option<f64>>, height: impl Into<Option<f64>>) -> Self {
        Self {
            width: width.into(),
            height: height.into(),
        }
    }
}
