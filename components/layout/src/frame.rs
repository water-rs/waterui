//! Placeholder for fixed-size frame layouts.
//!
//! A future iteration will add a public `Frame` view capable of overriding a
//! child's incoming proposal. The struct below documents the intent so that
//! renderers and component authors have a reference point.

use alloc::{vec, vec::Vec};
use waterui_core::{AnyView, View};

use crate::{
    Layout, Point, ProposalSize, Rect, Size, SubView,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Planned layout that clamps a single child's proposal.
#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
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
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        // A Frame proposes a modified size to its single child.
        // It uses its own ideal dimensions if they exist, otherwise parent's proposal.
        // This is then clamped by the frame's min/max constraints.

        let proposed_width = self.ideal_width.or(proposal.width);
        let proposed_height = self.ideal_height.or(proposal.height);

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

        // Measure the child with our constrained proposal
        let child_size = children
            .first()
            .map_or(Size::zero(), |c| c.size_that_fits(child_proposal));

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
            proposal.width.unwrap_or(target_width),
            proposal.height.unwrap_or(target_height),
        )
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Create constrained proposal for child
        let proposed_width = self.ideal_width.unwrap_or(bounds.width());
        let proposed_height = self.ideal_height.unwrap_or(bounds.height());

        let child_proposal = ProposalSize {
            width: Some(
                proposed_width
                    .max(self.min_width.unwrap_or(0.0))
                    .min(self.max_width.unwrap_or(f32::INFINITY))
                    .min(bounds.width()),
            ),
            height: Some(
                proposed_height
                    .max(self.min_height.unwrap_or(0.0))
                    .min(self.max_height.unwrap_or(f32::INFINITY))
                    .min(bounds.height()),
            ),
        };

        let child_size = children
            .first()
            .map_or(Size::zero(), |c| c.size_that_fits(child_proposal));

        // Handle infinite dimensions (axis-expanding views)
        let child_width = if child_size.width.is_infinite() {
            bounds.width()
        } else {
            child_size.width
        };

        let child_height = if child_size.height.is_infinite() {
            bounds.height()
        } else {
            child_size.height
        };

        let final_child_size = Size::new(child_width, child_height);

        // Calculate the child's origin point (top-left) based on alignment.
        let child_x = match self.alignment.horizontal() {
            HorizontalAlignment::Leading => bounds.x(),
            HorizontalAlignment::Center => {
                bounds.x() + (bounds.width() - final_child_size.width) / 2.0
            }
            HorizontalAlignment::Trailing => bounds.max_x() - final_child_size.width,
        };

        let child_y = match self.alignment.vertical() {
            VerticalAlignment::Top => bounds.y(),
            VerticalAlignment::Center => {
                bounds.y() + (bounds.height() - final_child_size.height) / 2.0
            }
            VerticalAlignment::Bottom => bounds.max_y() - final_child_size.height,
        };

        vec![Rect::new(Point::new(child_x, child_y), final_child_size)]
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
        FixedContainer::new(self.layout, vec![self.content])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StretchAxis;

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_frame_with_ideal_size() {
        let layout = FrameLayout {
            ideal_width: Some(100.0),
            ideal_height: Some(50.0),
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Frame uses ideal dimensions
        assert_eq!(size.width, 100.0);
        assert_eq!(size.height, 50.0);
    }

    #[test]
    fn test_frame_alignment() {
        let layout = FrameLayout {
            alignment: Alignment::BottomTrailing,
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &children);

        // Child should be at bottom-trailing corner
        assert_eq!(rects[0].x(), 70.0); // 100 - 30
        assert_eq!(rects[0].y(), 80.0); // 100 - 20
    }
}
