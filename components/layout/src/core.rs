//! Core layout primitives used by layout components.
//!
//! The layout system follows a simple two-pass protocol: first ask children how
//! large they would like to be given a [`ProposalSize`], then place them within
//! the final [`Size`]. This module defines the traits and helper types that are
//! shared by layout implementations across backends.

use core::fmt::Debug;

use alloc::{boxed::Box, vec::Vec};
use waterui_core::{AnyView, raw_view};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ChildMetadata {
    proposal: ProposalSize,
    priority: u8,
    stretch: bool,
}

impl ChildMetadata {
    pub fn new(proposal: ProposalSize, priority: u8, stretch: bool) -> Self {
        Self {
            proposal,
            priority,
            stretch,
        }
    }

    pub fn proposal(&self) -> ProposalSize {
        self.proposal
    }

    pub fn proposal_height(&self) -> Option<f64> {
        self.proposal.height
    }

    pub fn proposal_width(&self) -> Option<f64> {
        self.proposal.width
    }

    pub fn priority(&self) -> u8 {
        self.priority
    }

    pub fn stretch(&self) -> bool {
        self.stretch
    }
}

pub trait Layout {
    // Propose sizes for each child based on the parent's proposal and children's metadata.
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize>;

    // These children will receive proposal by underlying renderer, then report their current proposal.
    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size;

    // Place children within the given bounds and return their final rectangles.
    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect>;
}

pub struct Rect {
    origin: Point,
    size: Size,
}

impl Rect {
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    pub const fn origin(&self) -> Point {
        self.origin
    }

    pub const fn size(&self) -> Size {
        self.size
    }

    pub const fn x(&self) -> f64 {
        self.origin.x
    }

    pub const fn y(&self) -> f64 {
        self.origin.y
    }

    pub const fn width(&self) -> f64 {
        self.size.width
    }

    pub const fn height(&self) -> f64 {
        self.size.height
    }

    pub const fn max_x(&self) -> f64 {
        self.origin.x + self.size.width
    }

    pub const fn max_y(&self) -> f64 {
        self.origin.y + self.size.height
    }

    pub const fn mid_x(&self) -> f64 {
        self.origin.x + self.size.width / 2.0
    }

    pub const fn mid_y(&self) -> f64 {
        self.origin.y + self.size.height / 2.0
    }

    pub const fn min_x(&self) -> f64 {
        self.origin.x - self.size.width
    }

    pub const fn min_y(&self) -> f64 {
        self.origin.y - self.size.height
    }
}

/// Two-dimensional size expressed in absolute pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// Absolute coordinate relative to a parent layout's origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Soft constraint describing the desired size for a layout or subview.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ProposalSize {
    // exact value: Some(f64), unconstrained: None, infinite: f64::INFINITY
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl ProposalSize {
    pub fn new(width: impl Into<Option<f64>>, height: impl Into<Option<f64>>) -> Self {
        Self {
            width: width.into(),
            height: height.into(),
        }
    }
}

pub struct Container {
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
}

impl Container {
    pub fn new(layout: impl Layout + 'static, contents: Vec<AnyView>) -> Self {
        Self {
            layout: Box::new(layout),
            contents,
        }
    }
    #[must_use]
    pub fn into_inner(self) -> (Box<dyn Layout>, Vec<AnyView>) {
        (self.layout, self.contents)
    }
}

raw_view!(Container);
