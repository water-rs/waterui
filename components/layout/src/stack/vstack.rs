//! Vertical stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, id::Identifable, view::TupleViews, views::ForEach};

use crate::{
    Container, Layout, Point, ProposalSize, Rect, Size,
    container::FixedContainer, stack::HorizontalAlignment, SubView,
};

/// Layout engine shared by the public [`VStack`] view.
#[derive(Debug, Default, Clone)]
pub struct VStackLayout {
    /// The horizontal alignment of children within the stack.
    pub alignment: HorizontalAlignment,
    /// The spacing between children in the stack.
    pub spacing: f32,
}

/// Cached measurement for a child during layout
struct ChildMeasurement {
    size: Size,
    is_stretch: bool,
}

#[allow(clippy::cast_precision_loss)]
impl Layout for VStackLayout {
    fn size_that_fits(
        &self,
        proposal: ProposalSize,
        children: &mut [&mut dyn SubView],
    ) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        // Measure each child with parent's width (for text wrapping) and unspecified height
        let child_proposal = ProposalSize::new(proposal.width, None);

        let measurements: Vec<ChildMeasurement> = children
            .iter_mut()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
                is_stretch: child.is_stretch(),
            })
            .collect();

        let has_stretch = measurements.iter().any(|m| m.is_stretch);

        // Height: sum of non-stretch children + spacing
        let non_stretch_height: f32 = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.height)
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let intrinsic_height = non_stretch_height + total_spacing;
        let final_height = if has_stretch {
            proposal.height.unwrap_or(intrinsic_height)
        } else {
            intrinsic_height
        };

        // Width: max of non-stretch children
        let max_width = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.width)
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);

        let final_width = match proposal.width {
            Some(proposed) => max_width.min(proposed),
            None => max_width,
        };

        Size::new(final_width, final_height)
    }

    fn place(
        &self,
        bounds: Rect,
        children: &mut [&mut dyn SubView],
    ) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Measure children again (will be cached by SubView implementation)
        let child_proposal = ProposalSize::new(Some(bounds.width()), None);

        let measurements: Vec<ChildMeasurement> = children
            .iter_mut()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
                is_stretch: child.is_stretch(),
            })
            .collect();

        // Calculate stretch child height
        let stretch_count = measurements.iter().filter(|m| m.is_stretch).count();
        let non_stretch_height: f32 = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.height)
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let remaining_height = bounds.height() - non_stretch_height - total_spacing;
        let stretch_height = if stretch_count > 0 {
            (remaining_height / stretch_count as f32).max(0.0)
        } else {
            0.0
        };

        // Place children
        let mut rects = Vec::with_capacity(children.len());
        let mut current_y = bounds.y();

        for (i, measurement) in measurements.iter().enumerate() {
            if i > 0 {
                current_y += self.spacing;
            }

            // Handle infinite width (axis-expanding views) and clamp to bounds
            let child_width = if measurement.size.width.is_infinite() {
                bounds.width()
            } else {
                // Clamp child width to bounds - child can't be wider than container
                measurement.size.width.min(bounds.width())
            };

            let child_height = if measurement.is_stretch {
                stretch_height
            } else {
                measurement.size.height
            };

            let x = match self.alignment {
                HorizontalAlignment::Leading => bounds.x(),
                HorizontalAlignment::Center => bounds.x() + (bounds.width() - child_width) / 2.0,
                HorizontalAlignment::Trailing => bounds.x() + bounds.width() - child_width,
            };

            rects.push(Rect::new(
                Point::new(x, current_y),
                Size::new(child_width, child_height),
            ));

            current_y += child_height;
        }

        rects
    }
}

/// A vertical stack view that arranges its children in a column.
#[derive(Debug, Clone)]
pub struct VStack<C> {
    layout: VStackLayout,
    contents: C,
}

impl<C: TupleViews> VStack<(C,)> {
    /// Creates a vertical stack with the provided alignment, spacing, and
    /// children.
    pub const fn new(alignment: HorizontalAlignment, spacing: f32, contents: C) -> Self {
        Self {
            layout: VStackLayout { alignment, spacing },
            contents: (contents,),
        }
    }
}

impl<C, F, V> VStack<ForEach<C, F, V>>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    /// Creates a vertical stack by iterating over a collection and generating views.
    pub fn for_each(collection: C, generator: F) -> Self {
        Self {
            layout: VStackLayout::default(),
            contents: ForEach::new(collection, generator),
        }
    }
}

impl<C> VStack<C> {
    /// Sets the horizontal alignment for children in the stack.
    #[must_use]
    pub const fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

    /// Sets the spacing between children in the stack.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.layout.spacing = spacing;
        self
    }
}

impl<V> FromIterator<V> for VStack<(Vec<AnyView>,)>
where
    V: View,
{
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect::<Vec<_>>();
        Self::new(HorizontalAlignment::default(), 10.0, contents)
    }
}

/// Convenience constructor that centres children and uses the default spacing.
pub const fn vstack<C: TupleViews>(contents: C) -> VStack<(C,)> {
    VStack::new(HorizontalAlignment::Center, 10.0, contents)
}

impl<C, F, V> View for VStack<ForEach<C, F, V>>
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

impl<C: TupleViews + 'static> View for VStack<(C,)> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSubView {
        size: Size,
        is_stretch: bool,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&mut self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn is_stretch(&self) -> bool {
            self.is_stretch
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_vstack_size_two_children() {
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: 10.0,
        };

        let mut child1 = MockSubView {
            size: Size::new(100.0, 30.0),
            is_stretch: false,
        };
        let mut child2 = MockSubView {
            size: Size::new(80.0, 40.0),
            is_stretch: false,
        };

        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &mut children);

        assert_eq!(size.width, 100.0); // max width
        assert_eq!(size.height, 80.0); // 30 + 10 + 40
    }

    #[test]
    fn test_vstack_with_spacer() {
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: 0.0,
        };

        let mut child1 = MockSubView {
            size: Size::new(100.0, 30.0),
            is_stretch: false,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            is_stretch: true,
        };
        let mut child2 = MockSubView {
            size: Size::new(100.0, 30.0),
            is_stretch: false,
        };

        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        // With specified height, spacer should expand
        let size = layout.size_that_fits(
            ProposalSize::new(None, Some(200.0)),
            &mut children,
        );

        assert_eq!(size.height, 200.0);

        // Place should distribute remaining space to spacer
        let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));

        // Need fresh references
        let mut child1 = MockSubView {
            size: Size::new(100.0, 30.0),
            is_stretch: false,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            is_stretch: true,
        };
        let mut child2 = MockSubView {
            size: Size::new(100.0, 30.0),
            is_stretch: false,
        };
        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        let rects = layout.place(bounds, &mut children);

        assert_eq!(rects[0].height(), 30.0);
        assert_eq!(rects[1].height(), 140.0); // 200 - 30 - 30
        assert_eq!(rects[2].height(), 30.0);
        assert_eq!(rects[2].y(), 170.0); // 30 + 140
    }
}
