//! Overlay stack layout.

use alloc::{vec, vec::Vec};
use waterui_core::{AnyView, View, view::TupleViews};

use crate::{Layout, Point, ProposalSize, Rect, Size, container, stack::Alignment};

/// A layout implementation for stacking views on top of each other with specified alignment.
///
/// `ZStackLayout` positions all child views within the same bounds, overlaying them
/// according to the specified alignment. Each child is sized independently and
/// positioned based on the alignment setting.
#[derive(Debug, Clone)]
pub struct ZStackLayout {
    /// The alignment used to position children within the `ZStack`
    pub alignment: Alignment,
}

impl Layout for ZStackLayout {
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<ProposalSize> {
        // For ZStack, we propose the parent's size to each child
        // Each child gets to size itself based on the full available space
        vec![parent; children.len()]
    }

    #[allow(clippy::cast_precision_loss)]
    fn size(&mut self, parent: ProposalSize, children: &[crate::ChildMetadata]) -> Size {
        // ZStack's size is determined by the largest child
        // We find the maximum width and height among all children
        let mut max_width: f32 = 0.0;
        let mut max_height: f32 = 0.0;

        for child in children {
            if let Some(width) = child.proposal_width() {
                max_width = max_width.max(width);
            }
            if let Some(height) = child.proposal_height() {
                max_height = max_height.max(height);
            }
        }

        // If no children have explicit sizes, use parent constraints
        if max_width == 0.0 {
            max_width = parent.width.unwrap_or(0.0);
        }
        if max_height == 0.0 {
            max_height = parent.height.unwrap_or(0.0);
        }

        // Respect parent constraints - don't exceed them
        let final_width = parent
            .width
            .map_or(max_width, |parent_width| max_width.min(parent_width));

        let final_height = parent
            .height
            .map_or(max_height, |parent_height| max_height.min(parent_height));

        Size::new(final_width, final_height)
    }

    fn place(
        &mut self,
        bound: Rect,
        _proposal: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<Rect> {
        // ZStack places all children within the same bounds, positioned according to alignment
        if children.is_empty() {
            return Vec::new();
        }

        let mut rects = Vec::with_capacity(children.len());

        for child in children {
            // Each child gets sized to its ideal size, but constrained by the ZStack bounds
            let child_width = child
                .proposal_width()
                .unwrap_or_else(|| bound.width())
                .min(bound.width())
                .max(0.0);

            let child_height = child
                .proposal_height()
                .unwrap_or_else(|| bound.height())
                .min(bound.height())
                .max(0.0);

            // Position the child within the ZStack bounds according to alignment
            let (x, y) = self.calculate_position(&bound, &Size::new(child_width, child_height));

            rects.push(Rect::new(
                Point::new(x, y),
                Size::new(child_width, child_height),
            ));
        }

        rects
    }
}

impl ZStackLayout {
    /// Calculate the position of a child within the `ZStack` bounds based on alignment
    fn calculate_position(&self, bound: &Rect, child_size: &Size) -> (f32, f32) {
        let available_width = bound.width();
        let available_height = bound.height();

        match self.alignment {
            Alignment::TopLeading => (bound.x(), bound.y()),
            Alignment::Top => (
                bound.x() + (available_width - child_size.width) / 2.0,
                bound.y(),
            ),
            Alignment::TopTrailing => (bound.max_x() - child_size.width, bound.y()),
            Alignment::Leading => (
                bound.x(),
                bound.y() + (available_height - child_size.height) / 2.0,
            ),
            Alignment::Center => (
                bound.x() + (available_width - child_size.width) / 2.0,
                bound.y() + (available_height - child_size.height) / 2.0,
            ),
            Alignment::Trailing => (
                bound.max_x() - child_size.width,
                bound.y() + (available_height - child_size.height) / 2.0,
            ),
            Alignment::BottomLeading => (bound.x(), bound.max_y() - child_size.height),
            Alignment::Bottom => (
                bound.x() + (available_width - child_size.width) / 2.0,
                bound.max_y() - child_size.height,
            ),
            Alignment::BottomTrailing => (
                bound.max_x() - child_size.width,
                bound.max_y() - child_size.height,
            ),
        }
    }
}

container!(
    ZStack,
    ZStackLayout,
    "A stack layout that overlays its children on top of each other, aligning them based on the specified alignment."
);

impl ZStack {
    /// Creates a new `ZStack` with the specified alignment and contents.
    pub fn new(alignment: Alignment, contents: impl TupleViews) -> Self {
        Self {
            layout: ZStackLayout { alignment },
            contents: contents.into_views(),
        }
    }

    /// Sets the alignment for the `ZStack`.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }
}

impl<V> FromIterator<V> for ZStack
where
    V: View,
{
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect::<Vec<_>>();
        Self::new(Alignment::default(), contents)
    }
}

/// Creates a new `ZStack` with center alignment and the specified contents.
///
/// This is a convenience function that creates a `ZStack` with `Alignment::Center`.
pub fn zstack(contents: impl TupleViews) -> ZStack {
    ZStack::new(Alignment::Center, contents)
}
