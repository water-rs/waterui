//! Padding layouts that inset a child by fixed edge distances.

use alloc::{vec, vec::Vec};
use waterui_core::{AnyView, View};

use crate::{
    Layout, Point, ProposalSize, Rect, Size, SubView,
    container::FixedContainer,
};

/// Layout that insets its single child by the configured edge values.
#[derive(Debug, Clone)]
pub struct PaddingLayout {
    edges: EdgeInsets,
}

impl Layout for PaddingLayout {
    fn size_that_fits(
        &self,
        proposal: ProposalSize,
        children: &mut [&mut dyn SubView],
    ) -> Size {
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
            .first_mut()
            .map(|c| c.size_that_fits(child_proposal))
            .unwrap_or(Size::zero());

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

    fn place(
        &self,
        bounds: Rect,
        children: &mut [&mut dyn SubView],
    ) -> Vec<Rect> {
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
}

impl View for Padding {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, vec![self.content])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&mut self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn is_stretch(&self) -> bool {
            false
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
        let mut children: Vec<&mut dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &mut children);

        // Size = child size + padding on all sides
        assert_eq!(size.width, 70.0);  // 50 + 10 + 10
        assert_eq!(size.height, 50.0); // 30 + 10 + 10
    }

    #[test]
    fn test_padding_placement() {
        let layout = PaddingLayout {
            edges: EdgeInsets::new(10.0, 20.0, 15.0, 25.0),
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let mut children: Vec<&mut dyn SubView> = vec![&mut child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &mut children);

        // Child origin is offset by leading and top
        assert_eq!(rects[0].x(), 15.0);
        assert_eq!(rects[0].y(), 10.0);

        // Child size is bounds minus padding
        assert_eq!(rects[0].width(), 60.0);  // 100 - 15 - 25
        assert_eq!(rects[0].height(), 70.0); // 100 - 10 - 20
    }
}
