//! Overlay stack layout for multiple layers.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, id::Identifable, view::TupleViews, views::ForEach};

use crate::{
    Container, Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    container::FixedContainer, stack::Alignment,
};

/// Cached measurement for a child during layout
struct ChildMeasurement {
    size: Size,
}

/// Stacks an arbitrary number of children with a shared alignment.
///
/// `ZStackLayout` positions every child within the same bounds, overlaying them
/// according to the specified alignment. Each child is sized independently,
/// and the container's final width/height are the maxima of the children's
/// reported sizes. If you instead need the base child to dictate the container
/// size while layering secondary content, see [`crate::overlay::OverlayLayout`].
#[derive(Debug, Clone, Default)]
pub struct ZStackLayout {
    /// The alignment used to position children within the `ZStack`
    pub alignment: Alignment,
}

impl Layout for ZStackLayout {
    /// `ZStack` stretches in both directions to fill available space.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        // Measure each child with the parent's proposal
        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(proposal),
            })
            .collect();

        // ZStack's size is determined by the largest child
        let max_width = measurements
            .iter()
            .map(|m| m.size.width)
            .filter(|w| w.is_finite())
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);

        let max_height = measurements
            .iter()
            .map(|m| m.size.height)
            .filter(|h| h.is_finite())
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);

        // Respect parent constraints - don't exceed them
        let final_width = proposal
            .width
            .map_or(max_width, |parent_width| max_width.min(parent_width));

        let final_height = proposal
            .height
            .map_or(max_height, |parent_height| max_height.min(parent_height));

        Size::new(final_width, final_height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Re-measure children with the bounds as proposal
        let child_proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));

        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
            })
            .collect();

        // Place each child according to alignment
        let mut rects = Vec::with_capacity(children.len());

        for measurement in &measurements {
            // Handle infinite dimensions (axis-expanding views)
            let child_width = if measurement.size.width.is_infinite() {
                bounds.width()
            } else {
                measurement.size.width.min(bounds.width())
            };

            let child_height = if measurement.size.height.is_infinite() {
                bounds.height()
            } else {
                measurement.size.height.min(bounds.height())
            };

            let child_size = Size::new(child_width, child_height);
            let (x, y) = self.calculate_position(&bounds, child_size);

            rects.push(Rect::new(Point::new(x, y), child_size));
        }

        rects
    }
}

impl ZStackLayout {
    /// Calculate the position of a child within the `ZStack` bounds based on alignment
    fn calculate_position(&self, bound: &Rect, child_size: Size) -> (f32, f32) {
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

/// A view that overlays its children, aligning them in front of each other.
///
/// Use a `ZStack` when you want to layer views on top of each other. The stack
/// sizes itself to fit its largest child.
///
/// ```ignore
/// zstack((
///     Color::blue(),
///     text("Overlay Text"),
/// ))
/// ```
///
/// You can control how children align within the stack:
///
/// ```ignore
/// ZStack::new(Alignment::TopLeading, (
///     background_view,
///     content_view,
/// ))
/// ```
///
/// **Note:** If you only need a decorative background without affecting layout size,
/// use `.background()` instead.
#[derive(Debug, Clone)]
pub struct ZStack<C> {
    layout: ZStackLayout,
    contents: C,
}

impl<C> ZStack<C> {
    /// Sets the alignment for the `ZStack`.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }
}

impl<C, F, V> ZStack<ForEach<C, F, V>>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    /// Creates a new `ZStack` with views generated from a collection using `ForEach`.
    ///
    /// # Arguments
    /// * `collection` - The collection of items to iterate over
    /// * `generator` - A function that generates a view for each item in the collection
    pub fn for_each(collection: C, generator: F) -> Self {
        Self {
            layout: ZStackLayout::default(),
            contents: ForEach::new(collection, generator),
        }
    }
}

impl<C: TupleViews> ZStack<(C,)> {
    /// Creates a new `ZStack` with the specified alignment and contents.
    ///
    /// # Arguments
    /// * `alignment` - The alignment to use for positioning children within the stack
    /// * `contents` - A collection of views to be stacked
    pub const fn new(alignment: Alignment, contents: C) -> Self {
        Self {
            layout: ZStackLayout { alignment },
            contents: (contents,),
        }
    }
}

impl<V> FromIterator<V> for ZStack<(Vec<AnyView>,)>
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
pub const fn zstack<C: TupleViews>(contents: C) -> ZStack<(C,)> {
    ZStack::new(Alignment::Center, contents)
}

impl<C> View for ZStack<(C,)>
where
    C: TupleViews + 'static,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}

impl<C, F, V> View for ZStack<ForEach<C, F, V>>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self.layout, self.contents)
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
    fn test_zstack_size_multiple_children() {
        let layout = ZStackLayout {
            alignment: Alignment::Center,
        };

        let mut child1 = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(80.0, 40.0),
        };
        let mut child3 = MockSubView {
            size: Size::new(60.0, 60.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2, &mut child3];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // ZStack takes the max width and max height
        assert_eq!(size.width, 80.0);
        assert_eq!(size.height, 60.0);
    }

    #[test]
    fn test_zstack_placement_center() {
        let layout = ZStackLayout {
            alignment: Alignment::Center,
        };

        let mut child1 = MockSubView {
            size: Size::new(40.0, 20.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(60.0, 40.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &children);

        // Child 1: centered in 100x100
        assert_eq!(rects[0].x(), 30.0); // (100 - 40) / 2
        assert_eq!(rects[0].y(), 40.0); // (100 - 20) / 2

        // Child 2: centered in 100x100
        assert_eq!(rects[1].x(), 20.0); // (100 - 60) / 2
        assert_eq!(rects[1].y(), 30.0); // (100 - 40) / 2
    }
}
