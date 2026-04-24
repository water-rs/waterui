//! Placeholder for fixed-size frame layouts.
//!
//! A future iteration will add a public `Frame` view capable of overriding a
//! child's incoming proposal. The struct below documents the intent so that
//! renderers and component authors have a reference point.

use alloc::{vec, vec::Vec};
use waterui_core::{AnyView, View};

use crate::{
    Layout, PlacedSubview, Point, ProposalSize, Rect, Size, SubView, ViewDimensions,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Planned layout that clamps a single child's proposal.
#[derive(Debug, Clone, PartialEq, Default)]
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
        // It uses its own ideal dimensions if they exist, otherwise parent's proposal,
        // then clamps that proposal by the frame's min/max constraints.
        let child_proposal = ProposalSize {
            width: frame_child_proposal_axis(
                proposal.width,
                self.min_width,
                self.ideal_width,
                self.max_width,
            ),
            height: frame_child_proposal_axis(
                proposal.height,
                self.min_height,
                self.ideal_height,
                self.max_height,
            ),
        };

        // Measure the child with our constrained proposal
        let child_dimensions = children
            .first()
            .map_or(ViewDimensions::new(Size::zero()), |c| {
                c.measure(child_proposal)
            });
        let child_size = child_dimensions.size;

        // Resolve the frame size on each axis.
        //
        // If parent proposes a concrete size, we respect it but clamp through frame constraints.
        // If parent leaves axis unspecified, use frame ideal or measured child size, then clamp.
        let final_width = frame_resolved_axis(
            proposal.width,
            child_size.width,
            self.min_width,
            self.ideal_width,
            self.max_width,
        );
        let final_height = frame_resolved_axis(
            proposal.height,
            child_size.height,
            self.min_height,
            self.ideal_height,
            self.max_height,
        );

        Size::new(final_width, final_height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Create constrained proposal for child
        let proposed_width = self.ideal_width.unwrap_or_else(|| bounds.width());
        let proposed_height = self.ideal_height.unwrap_or_else(|| bounds.height());

        let child_proposal = ProposalSize {
            width: Some(clamp_frame_axis(
                proposed_width.min(bounds.width()),
                self.min_width,
                self.max_width,
            )),
            height: Some(clamp_frame_axis(
                proposed_height.min(bounds.height()),
                self.min_height,
                self.max_height,
            )),
        };

        let child_dimensions = children
            .first()
            .map_or(ViewDimensions::new(Size::zero()), |c| {
                c.measure(child_proposal)
            });
        let child_size = child_dimensions.size;

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
        let mut adjusted_dimensions = child_dimensions;
        adjusted_dimensions.size = final_child_size;

        // Calculate the child's origin point (top-left) based on alignment.
        let horizontal = self.alignment.horizontal();
        let horizontal_target = if horizontal == HorizontalAlignment::Leading {
            0.0
        } else if horizontal == HorizontalAlignment::Trailing {
            bounds.width()
        } else if horizontal == HorizontalAlignment::Center {
            bounds.width() * 0.5
        } else {
            adjusted_dimensions
                .horizontal(horizontal)
                .clamp(0.0, final_child_size.width)
        };
        let child_x = bounds.x() + horizontal_target
            - adjusted_dimensions
                .horizontal(horizontal)
                .clamp(0.0, final_child_size.width);

        let vertical = self.alignment.vertical();
        let vertical_target = if vertical == VerticalAlignment::Top {
            0.0
        } else if vertical == VerticalAlignment::Bottom {
            bounds.height()
        } else if vertical == VerticalAlignment::Center {
            bounds.height() * 0.5
        } else {
            adjusted_dimensions
                .vertical(vertical)
                .clamp(0.0, final_child_size.height)
        };
        let child_y = bounds.y() + vertical_target
            - adjusted_dimensions
                .vertical(vertical)
                .clamp(0.0, final_child_size.height);

        vec![Rect::new(Point::new(child_x, child_y), final_child_size)]
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .first()
            .and_then(|child| child.explicit_horizontal(alignment))
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .first()
            .and_then(|child| child.explicit_vertical(alignment))
    }
}

#[inline]
fn clamp_frame_axis(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    value
        .max(min.unwrap_or(f32::NEG_INFINITY))
        .min(max.unwrap_or(f32::INFINITY))
}

#[inline]
fn frame_child_proposal_axis(
    parent_proposal: Option<f32>,
    min: Option<f32>,
    ideal: Option<f32>,
    max: Option<f32>,
) -> Option<f32> {
    ideal
        .or(parent_proposal)
        .map(|value| clamp_frame_axis(value, min, max))
}

#[inline]
fn frame_resolved_axis(
    parent_proposal: Option<f32>,
    child_size: f32,
    min: Option<f32>,
    ideal: Option<f32>,
    max: Option<f32>,
) -> f32 {
    parent_proposal.map_or_else(
        || clamp_frame_axis(ideal.unwrap_or(child_size), min, max),
        |value| clamp_frame_axis(value, min, max),
    )
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
        self.layout.min_width = Some(width);
        self.layout.ideal_width = Some(width);
        self.layout.max_width = Some(width);
        self
    }

    /// Sets the ideal height of the frame.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.layout.min_height = Some(height);
        self.layout.ideal_height = Some(height);
        self.layout.max_height = Some(height);
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
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
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
        assert!((size.width - 100.0).abs() < f32::EPSILON);
        assert!((size.height - 50.0).abs() < f32::EPSILON);
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
        assert!((rects[0].x() - 70.0).abs() < f32::EPSILON); // 100 - 30
        assert!((rects[0].y() - 80.0).abs() < f32::EPSILON); // 100 - 20
    }

    #[test]
    fn test_fixed_width_resists_zero_min_query() {
        let layout = FrameLayout {
            min_width: Some(120.0),
            ideal_width: Some(120.0),
            max_width: Some(120.0),
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::ZERO, &children);
        assert!((size.width - 120.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_min_width_clamps_zero_min_query() {
        let layout = FrameLayout {
            min_width: Some(64.0),
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(10.0, 10.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::ZERO, &children);
        assert!((size.width - 64.0).abs() < f32::EPSILON);
    }
}
