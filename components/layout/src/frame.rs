//! Placeholder for fixed-size frame layouts.
//!
//! A future iteration will add a public `Frame` view capable of overriding a
//! child's incoming proposal. The struct below documents the intent so that
//! renderers and component authors have a reference point.

use alloc::{vec, vec::Vec};
use waterui_core::{AnyView, View};

use crate::{
    ChildMetadata, Container, Layout, Point, ProposalSize, Rect, Size,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Planned layout that clamps a single child's proposal.
#[derive(Debug, Clone, PartialEq, PartialOrd,Default)]
pub struct FrameLayout {
    min_width: Option<f32>,
    ideal_width: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    ideal_height: Option<f32>,
    max_height: Option<f32>,
    alignment: Alignment,
}

impl Layout for FrameLayout {
    /// Proposes a size to the child, taking the frame's constraints into account.
    fn propose(&mut self, parent: ProposalSize, _children: &[ChildMetadata]) -> Vec<ProposalSize> {
        // A Frame passes a modified proposal to its single child.
        // It uses its own ideal dimensions if they exist, otherwise it passes the parent's proposal.
        // This is then clamped by the frame's min/max constraints.

        let proposed_width = self.ideal_width.or(parent.width);
        let proposed_height = self.ideal_height.or(parent.height);

        let child_proposal = ProposalSize {
            width: proposed_width.map(|w| {
                w.max(self.min_width.unwrap_or(f32::NEG_INFINITY))
                    .min(self.max_width.unwrap_or(f32::INFINITY))
            }),
            height: proposed_height.map(|h| {
                h.max(self.min_height.unwrap_or(f32::NEG_INFINITY))
                    .min(self.max_height.unwrap_or(f32::INFINITY))
            }),
        };

        vec![child_proposal]
    }

    /// Determines the size of the frame itself.
    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        // The frame's size is determined by its child's measured size,
        // but overridden by its own ideal dimensions and clamped by its min/max.

        let child_size = children.first().map_or(Size::zero(), |c| {
            Size::new(
                c.proposal_width().unwrap_or(0.0),
                c.proposal_height().unwrap_or(0.0),
            )
        });

        // 1. Determine the frame's ideal width based on its own properties and its child.
        let mut target_width = self.ideal_width.unwrap_or(child_size.width);
        target_width = target_width
            .max(self.min_width.unwrap_or(f32::NEG_INFINITY))
            .min(self.max_width.unwrap_or(f32::INFINITY));

        // 2. Determine the frame's ideal height.
        let mut target_height = self.ideal_height.unwrap_or(child_size.height);
        target_height = target_height
            .max(self.min_height.unwrap_or(f32::NEG_INFINITY))
            .min(self.max_height.unwrap_or(f32::INFINITY));

        // 3. The final size is the target size, but it must also respect the parent's proposal.
        // If the parent proposed a fixed size, we must take it.
        Size::new(
            parent.width.unwrap_or(target_width),
            parent.height.unwrap_or(target_height),
        )
    }

    /// Places the child within the frame's final bounds according to the alignment.
    fn place(
        &mut self,
        bound: Rect,
        _proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect> {
        let child = children
            .first()
            .expect("FrameLayout expects exactly one child");

        let child_size = Size::new(
            child.proposal_width().unwrap_or(0.0),
            child.proposal_height().unwrap_or(0.0),
        );

        // Calculate the child's origin point (top-left) based on alignment.
        let child_x = match self.alignment.horizontal() {
            HorizontalAlignment::Leading => bound.x(),
            HorizontalAlignment::Center => bound.x() + (bound.width() - child_size.width) / 2.0,
            HorizontalAlignment::Trailing => bound.max_x() - child_size.width,
        };

        let child_y = match self.alignment.vertical() {
            VerticalAlignment::Top => bound.y(),
            VerticalAlignment::Center => bound.y() + (bound.height() - child_size.height) / 2.0,
            VerticalAlignment::Bottom => bound.max_y() - child_size.height,
        };

        let child_origin = Point::new(child_x, child_y);

        vec![Rect::new(child_origin, child_size)]
    }
}

/// A view that provides a frame with optional size constraints and alignment for its child.
///
/// The Frame view allows you to specify minimum, ideal, and maximum dimensions
/// for width and height, and controls how the child is aligned within the frame.
#[derive(Debug)]
pub struct Frame {
    layout: FrameLayout,
    content: AnyView,
}

impl Frame {
    /// Creates a new Frame with the specified content and alignment.
    ///
    /// # Arguments
    /// * `content` - The child view to be contained within the frame
    /// * `alignment` - How the child should be aligned within the frame
    #[must_use]
    pub fn new(content: impl View) -> Self {
        Self {
            layout: FrameLayout::default(),
            content: AnyView::new(content),
        }
    }

    /// Sets the alignment of the child within the frame.
    ///
    /// # Arguments
    /// * `alignment` - The alignment to apply to the child view
    #[must_use] 
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

    /// Sets the ideal width of the frame.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.layout.ideal_width = Some(width);
        self
    }

    /// Sets the ideal height of the frame.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.layout.ideal_height = Some(height);
        self
    }

    /// Sets the minimum width of the frame.
    #[must_use]
    pub const fn min_width(mut self, width: f32) -> Self {
        self.layout.min_width = Some(width);
        self
    }

    /// Sets the maximum width of the frame.
    #[must_use]
    pub const fn max_width(mut self, width: f32) -> Self {
        self.layout.max_width = Some(width);
        self
    }

    /// Sets the minimum height of the frame.
    #[must_use]
    pub const fn min_height(mut self, height: f32) -> Self {
        self.layout.min_height = Some(height);
        self
    }

    /// Sets the maximum height of the frame.
    #[must_use]
    pub const fn max_height(mut self, height: f32) -> Self {
        self.layout.max_height = Some(height);
        self
    }
}

impl View for Frame {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // The Frame view's body is just a Container with our custom layout and the child content.
        Container::new(self.layout, vec![self.content])
    }
}
