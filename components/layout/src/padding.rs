//! Padding layouts that inset a child by fixed edge distances.

use alloc::{vec, vec::Vec};
use waterui_core::{AnyView, View};

use crate::{
    HorizontalAlignment, Layout, PlacedSubview, Point, ProposalSize, Rect, Size, SubView,
    VerticalAlignment, container::FixedContainer,
};

/// Layout that insets its single child by the configured edge values.
#[derive(Debug, Clone)]
pub struct PaddingLayout {
    edges: EdgeInsets,
}

impl Layout for PaddingLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        // The horizontal and vertical space consumed by padding.
        let horizontal_padding = self.edges.leading + self.edges.trailing;
        let vertical_padding = self.edges.top + self.edges.bottom;

        // Reduce the proposed size for the child by the padding amount.
        let child_proposal = ProposalSize {
            width: proposal.width.map(|w| (w - horizontal_padding).max(0.0)),
            height: proposal.height.map(|h| (h - vertical_padding).max(0.0)),
        };

        // Measure the child
        let child_size = children
            .first()
            .map_or(Size::zero(), |c| c.measure(child_proposal).size);

        // Handle infinite dimensions
        let child_width = if child_size.width.is_infinite() {
            proposal.width.unwrap_or(0.0) - horizontal_padding
        } else {
            child_size.width
        };

        let child_height = if child_size.height.is_infinite() {
            proposal.height.unwrap_or(0.0) - vertical_padding
        } else {
            child_size.height
        };

        // The final size is the child's size plus the padding.
        Size::new(
            child_width + horizontal_padding,
            child_height + vertical_padding,
        )
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Create the child's frame by insetting the parent's bound by the padding amount.
        let child_origin = Point::new(bounds.x() + self.edges.leading, bounds.y() + self.edges.top);

        let horizontal_padding = self.edges.leading + self.edges.trailing;
        let vertical_padding = self.edges.top + self.edges.bottom;

        let child_size = Size::new(
            (bounds.width() - horizontal_padding).max(0.0),
            (bounds.height() - vertical_padding).max(0.0),
        );

        vec![Rect::new(child_origin, child_size)]
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

/// Insets applied to the four edges of a rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeInsets {
    top: f32,
    bottom: f32,
    leading: f32,
    trailing: f32,
}

#[allow(clippy::cast_possible_truncation)]
impl<T: Into<f64>> From<T> for EdgeInsets {
    fn from(value: T) -> Self {
        let v = value.into() as f32;
        Self::all(v)
    }
}

impl Default for EdgeInsets {
    fn default() -> Self {
        Self::all(0.0)
    }
}

impl EdgeInsets {
    /// Creates an [`EdgeInsets`] value with explicit edges.
    #[must_use]
    pub const fn new(top: f32, bottom: f32, leading: f32, trailing: f32) -> Self {
        Self {
            top,
            bottom,
            leading,
            trailing,
        }
    }

    /// Returns equal insets on every edge.
    #[must_use]
    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            bottom: value,
            leading: value,
            trailing: value,
        }
    }

    /// Returns symmetric vertical and horizontal insets.
    #[must_use]
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            bottom: vertical,
            leading: horizontal,
            trailing: horizontal,
        }
    }

    /// Returns the top inset.
    #[must_use]
    pub const fn top(&self) -> f32 {
        self.top
    }

    /// Returns the bottom inset.
    #[must_use]
    pub const fn bottom(&self) -> f32 {
        self.bottom
    }

    /// Returns the leading (left in LTR) inset.
    #[must_use]
    pub const fn leading(&self) -> f32 {
        self.leading
    }

    /// Returns the trailing (right in LTR) inset.
    #[must_use]
    pub const fn trailing(&self) -> f32 {
        self.trailing
    }
}

/// View wrapper that applies [`PaddingLayout`] to a single child.
#[derive(Debug)]
pub struct Padding {
    layout: PaddingLayout,
    content: AnyView,
}

impl Padding {
    /// Wraps a view with custom `edges`.
    pub fn new(edges: EdgeInsets, content: impl View + 'static) -> Self {
        Self {
            layout: PaddingLayout { edges },
            content: AnyView::new(content),
        }
    }

    /// Consumes the padding and returns the edge insets and content.
    #[must_use]
    pub fn into_inner(self) -> (EdgeInsets, AnyView) {
        (self.layout.edges, self.content)
    }
}

impl View for Padding {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, vec![self.content])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewDimensions;
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
    fn test_padding_size() {
        let layout = PaddingLayout {
            edges: EdgeInsets::all(10.0),
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Size = child size + padding on all sides
        assert!((size.width - 70.0).abs() < f32::EPSILON); // 50 + 10 + 10
        assert!((size.height - 50.0).abs() < f32::EPSILON); // 30 + 10 + 10
    }

    #[test]
    fn test_padding_placement() {
        let layout = PaddingLayout {
            edges: EdgeInsets::new(10.0, 20.0, 15.0, 25.0),
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &children);

        // Child origin is offset by leading and top
        assert!((rects[0].x() - 15.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 10.0).abs() < f32::EPSILON);

        // Child size is bounds minus padding
        assert!((rects[0].width() - 60.0).abs() < f32::EPSILON); // 100 - 15 - 25
        assert!((rects[0].height() - 70.0).abs() < f32::EPSILON); // 100 - 10 - 20
    }
}
