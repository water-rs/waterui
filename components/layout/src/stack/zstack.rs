//! Overlay stack layout for multiple layers.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, id::Identifiable, view::TupleViews, views::ForEach};

use crate::{
    Layout, LazyContainer, PlacedSubview, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    ViewDimensions,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Cached measurement for a child during layout
struct ChildMeasurement {
    dimensions: ViewDimensions,
}

impl ChildMeasurement {
    const fn size(&self) -> Size {
        self.dimensions.size
    }
}

fn zstack_horizontal_metrics(
    measurements: &[ChildMeasurement],
    alignment: HorizontalAlignment,
) -> (f32, f32) {
    let mut max_leading = 0.0_f32;
    let mut max_trailing = 0.0_f32;
    for measurement in measurements {
        let size = measurement.size();
        let guide = measurement
            .dimensions
            .horizontal(alignment)
            .clamp(0.0, size.width);
        max_leading = max_leading.max(guide);
        max_trailing = max_trailing.max((size.width - guide).max(0.0));
    }
    (max_leading, max_trailing)
}

fn zstack_vertical_metrics(
    measurements: &[ChildMeasurement],
    alignment: VerticalAlignment,
) -> (f32, f32) {
    let mut max_above = 0.0_f32;
    let mut max_below = 0.0_f32;
    for measurement in measurements {
        let size = measurement.size();
        let guide = measurement
            .dimensions
            .vertical(alignment)
            .clamp(0.0, size.height);
        max_above = max_above.max(guide);
        max_below = max_below.max((size.height - guide).max(0.0));
    }
    (max_above, max_below)
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
    /// `ZStack` is content-sized by default (it does not stretch automatically).
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        // Measure each child with the parent's proposal
        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                dimensions: child.measure(proposal),
            })
            .collect();

        let (max_leading, max_trailing) =
            zstack_horizontal_metrics(&measurements, self.alignment.horizontal());
        let (max_above, max_below) =
            zstack_vertical_metrics(&measurements, self.alignment.vertical());
        let max_width = max_leading + max_trailing;
        let max_height = max_above + max_below;

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
                dimensions: child.measure(child_proposal),
            })
            .collect();

        let horizontal = self.alignment.horizontal();
        let vertical = self.alignment.vertical();
        let target_x = if horizontal == HorizontalAlignment::Leading {
            0.0
        } else if horizontal == HorizontalAlignment::Trailing {
            bounds.width()
        } else if horizontal == HorizontalAlignment::Center {
            bounds.width() * 0.5
        } else {
            let (max_leading, _) = zstack_horizontal_metrics(&measurements, horizontal);
            max_leading
        };
        let target_y = if vertical == VerticalAlignment::Top {
            0.0
        } else if vertical == VerticalAlignment::Bottom {
            bounds.height()
        } else if vertical == VerticalAlignment::Center {
            bounds.height() * 0.5
        } else {
            let (max_above, _) = zstack_vertical_metrics(&measurements, vertical);
            max_above
        };

        // Place each child according to alignment
        let mut rects = Vec::with_capacity(children.len());

        for measurement in &measurements {
            // Handle infinite dimensions (axis-expanding views)
            let child_width = if measurement.dimensions.size.width.is_infinite() {
                bounds.width()
            } else {
                measurement.dimensions.size.width.min(bounds.width())
            };

            let child_height = if measurement.dimensions.size.height.is_infinite() {
                bounds.height()
            } else {
                measurement.dimensions.size.height.min(bounds.height())
            };

            let child_size = Size::new(child_width, child_height);
            let mut adjusted_dimensions = measurement.dimensions.clone();
            adjusted_dimensions.size = child_size;
            let x = bounds.x() + target_x
                - adjusted_dimensions
                    .horizontal(horizontal)
                    .clamp(0.0, child_size.width);
            let y = bounds.y() + target_y
                - adjusted_dimensions
                    .vertical(vertical)
                    .clamp(0.0, child_size.height);

            rects.push(Rect::new(Point::new(x, y), child_size));
        }

        rects
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment == self.alignment.horizontal() {
            return children
                .iter()
                .filter_map(|child| child.explicit_horizontal(alignment))
                .min_by(f32::total_cmp);
        }
        None
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment == VerticalAlignment::LastBaseline {
            return children
                .iter()
                .filter_map(|child| child.explicit_vertical(alignment))
                .max_by(f32::total_cmp);
        }
        if alignment == VerticalAlignment::FirstBaseline || alignment == self.alignment.vertical() {
            return children
                .iter()
                .filter_map(|child| child.explicit_vertical(alignment))
                .min_by(f32::total_cmp);
        }
        None
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
    C::Item: Identifiable,
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
    C: Collection + Clone,
    C::Item: Identifiable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        LazyContainer::new(self.layout, self.contents)
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
        assert!((size.width - 80.0).abs() < f32::EPSILON);
        assert!((size.height - 60.0).abs() < f32::EPSILON);
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
        assert!((rects[0].x() - 30.0).abs() < f32::EPSILON); // (100 - 40) / 2
        assert!((rects[0].y() - 40.0).abs() < f32::EPSILON); // (100 - 20) / 2

        // Child 2: centered in 100x100
        assert!((rects[1].x() - 20.0).abs() < f32::EPSILON); // (100 - 60) / 2
        assert!((rects[1].y() - 30.0).abs() < f32::EPSILON); // (100 - 40) / 2
    }
}
