//! Vertical stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, env::with, id::Identifable, view::TupleViews, views::ForEach};

use crate::{
    Container, Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    container::FixedContainer,
    stack::{Axis, HorizontalAlignment},
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
    stretch_axis: StretchAxis,
}

impl ChildMeasurement {
    /// Returns true if this child stretches vertically (for `VStack` height distribution).
    /// In `VStack` context:
    /// - `MainAxis` means vertical (`VStack`'s main axis)
    /// - `CrossAxis` means horizontal (`VStack`'s cross axis)
    const fn stretches_main_axis(&self) -> bool {
        matches!(
            self.stretch_axis,
            StretchAxis::Vertical | StretchAxis::Both | StretchAxis::MainAxis
        )
    }

    /// Returns true if this child stretches horizontally (for `VStack` width expansion).
    /// In `VStack` context:
    /// - `CrossAxis` means horizontal (`VStack`'s cross axis)
    const fn stretches_cross_axis(&self) -> bool {
        matches!(
            self.stretch_axis,
            StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::CrossAxis
        )
    }
}

#[allow(clippy::cast_precision_loss)]
impl Layout for VStackLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        // Measure each child with parent's width (for text wrapping) and unspecified height
        let child_proposal = ProposalSize::new(proposal.width, None);

        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
                stretch_axis: child.stretch_axis(),
            })
            .collect();

        // VStack checks for main-axis (vertical) stretching
        let has_main_axis_stretch = measurements
            .iter()
            .any(ChildMeasurement::stretches_main_axis);

        // Height: sum of children that don't stretch on main axis (vertically) + spacing
        // (axis-expanding components like TextField report their intrinsic height here)
        let non_stretch_height: f32 = measurements
            .iter()
            .filter(|m| !m.stretches_main_axis())
            .map(|m| m.size.height)
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let intrinsic_height = non_stretch_height + total_spacing;
        let final_height = if has_main_axis_stretch {
            proposal.height.unwrap_or(intrinsic_height)
        } else {
            intrinsic_height
        };

        // Width: max of children that don't stretch on cross axis (horizontally)
        // (cross-axis stretching children don't contribute to intrinsic width)
        let max_width = measurements
            .iter()
            .filter(|m| !m.stretches_cross_axis())
            .map(|m| m.size.width)
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);

        let final_width = match proposal.width {
            Some(proposed) => max_width.min(proposed),
            None => max_width,
        };

        Size::new(final_width, final_height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Measure children again (will be cached by SubView implementation)
        let child_proposal = ProposalSize::new(Some(bounds.width()), None);

        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
                stretch_axis: child.stretch_axis(),
            })
            .collect();

        // Calculate stretch child height - only for main-axis (vertically) stretching children
        let main_axis_stretch_count = measurements
            .iter()
            .filter(|m| m.stretches_main_axis())
            .count();
        let non_stretch_height: f32 = measurements
            .iter()
            .filter(|m| !m.stretches_main_axis())
            .map(|m| m.size.height)
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let remaining_height = bounds.height() - non_stretch_height - total_spacing;
        let stretch_height = if main_axis_stretch_count > 0 {
            (remaining_height / main_axis_stretch_count as f32).max(0.0)
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

            // Handle cross-axis (horizontal) stretching and infinite width
            let child_width = if measurement.stretches_cross_axis() {
                // CrossAxis in VStack means expand horizontally to full bounds width
                bounds.width()
            } else if measurement.size.width.is_infinite() {
                bounds.width()
            } else {
                // Clamp child width to bounds - child can't be wider than container
                measurement.size.width.min(bounds.width())
            };

            let child_height = if measurement.stretches_main_axis() {
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

    /// `VStack` stretches horizontally to fill available width (cross-axis).
    /// It uses intrinsic height based on children (main-axis).
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

/// A view that arranges its children in a vertical line.
///
/// Use a `VStack` to arrange views top-to-bottom. The stack sizes itself to fit
/// its contents, distributing available space among its children.
///
/// ```ignore
/// vstack((
///     text("Title"),
///     text("Subtitle"),
/// ))
/// ```
///
/// You can customize the spacing between children and their horizontal alignment:
///
/// ```ignore
/// VStack::new(HorizontalAlignment::Leading, 8.0, (
///     text("First"),
///     text("Second"),
/// ))
/// ```
///
/// Use [`spacer()`] to push content to the top and bottom:
///
/// ```ignore
/// vstack((
///     text("Header"),
///     spacer(),
///     text("Footer"),
/// ))
/// ```
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
        // Inject the vertical axis into the container
        with(Container::new(self.layout, self.contents), Axis::Vertical)
    }
}

impl<C: TupleViews + 'static> View for VStack<(C,)> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Inject the vertical axis into the container
        with(
            FixedContainer::new(self.layout, self.contents.0),
            Axis::Vertical,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSubView {
        size: Size,
        stretch_axis: StretchAxis,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn stretch_axis(&self) -> StretchAxis {
            self.stretch_axis
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
            stretch_axis: StretchAxis::None,
        };
        let mut child2 = MockSubView {
            size: Size::new(80.0, 40.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

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
            stretch_axis: StretchAxis::None,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            stretch_axis: StretchAxis::Both, // Spacer stretches in both directions
        };
        let mut child2 = MockSubView {
            size: Size::new(100.0, 30.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        // With specified height, spacer should expand
        let size = layout.size_that_fits(ProposalSize::new(None, Some(200.0)), &children);

        assert_eq!(size.height, 200.0);

        // Place should distribute remaining space to spacer
        let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));

        // Need fresh references
        let mut child1 = MockSubView {
            size: Size::new(100.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            stretch_axis: StretchAxis::Both,
        };
        let mut child2 = MockSubView {
            size: Size::new(100.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        let rects = layout.place(bounds, &children);

        assert_eq!(rects[0].height(), 30.0);
        assert_eq!(rects[1].height(), 140.0); // 200 - 30 - 30
        assert_eq!(rects[2].height(), 30.0);
        assert_eq!(rects[2].y(), 170.0); // 30 + 140
    }

    #[test]
    fn test_vstack_with_horizontal_stretch() {
        // TextField-like component: stretches horizontally but has fixed height
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: 10.0,
        };

        let mut label = MockSubView {
            size: Size::new(50.0, 20.0),
            stretch_axis: StretchAxis::None,
        };
        let mut text_field = MockSubView {
            size: Size::new(100.0, 40.0), // reports minimum width, intrinsic height
            stretch_axis: StretchAxis::Horizontal, // stretches width only
        };
        let mut button = MockSubView {
            size: Size::new(80.0, 44.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut label, &mut text_field, &mut button];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Width: max of non-horizontal-stretching children = max(50, 80) = 80
        // Note: text_field stretches horizontally so its width doesn't contribute
        assert_eq!(size.width, 80.0);
        // Height: all children contribute (text_field doesn't stretch vertically)
        // = 20 + 10 + 40 + 10 + 44 = 124
        assert_eq!(size.height, 124.0);
    }
}
