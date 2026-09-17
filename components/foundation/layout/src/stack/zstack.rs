//! Overlay stack layout for multiple layers.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, id::Identifiable, view::TupleViews, views::ForEach};

use crate::{
    Layout, LazyContainer, PlacedSubview, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement, ViewDimensions,
    container::FixedContainer,
    stack::{
        Alignment, HorizontalAlignment, VerticalAlignment,
        distribute::{LineAnchor, container_line, cross_envelope},
    },
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

/// The horizontal envelope of the layers lined up on `alignment`.
fn zstack_horizontal_metrics(
    measurements: &[ChildMeasurement],
    alignment: HorizontalAlignment,
) -> (f32, f32) {
    cross_envelope(measurements.iter().map(|measurement| {
        (
            measurement.size().width,
            measurement.dimensions.horizontal(alignment),
        )
    }))
}

/// The vertical envelope of the layers lined up on `alignment`.
fn zstack_vertical_metrics(
    measurements: &[ChildMeasurement],
    alignment: VerticalAlignment,
) -> (f32, f32) {
    cross_envelope(measurements.iter().map(|measurement| {
        (
            measurement.size().height,
            measurement.dimensions.vertical(alignment),
        )
    }))
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
    /// A `ZStack` is content-sized only while every child is — a child that
    /// stretches (a `Color`, a `GpuSurface`) makes the stack stretch on the
    /// same axes, since `place` hands such children the full bounds.
    ///
    /// Axis-relative answers have no direction to resolve against here:
    /// `MainAxis` children (`Spacer`) want whatever space is offered, which in
    /// a zstack is both axes; `CrossAxis` children (`Divider`) resolve to
    /// their default orientation — a horizontal rule — and fill horizontally.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        let mut fills_h = false;
        let mut fills_v = false;
        for child in children {
            match child {
                StretchAxis::None => {}
                StretchAxis::Both | StretchAxis::MainAxis => {
                    fills_h = true;
                    fills_v = true;
                }
                StretchAxis::Horizontal | StretchAxis::CrossAxis => fills_h = true,
                StretchAxis::Vertical => fills_v = true,
            }
        }
        match (fills_h, fills_v) {
            (true, true) => StretchAxis::Both,
            (true, false) => StretchAxis::Horizontal,
            (false, true) => StretchAxis::Vertical,
            (false, false) => StretchAxis::None,
        }
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

        // The stack is the envelope of its layers, like every container: a
        // layer wider than the offer widens the stack rather than being
        // shrunk, and a layer that answers an unbounded extent makes the
        // stack answer it too so the fill pass above resolves it.
        let (max_leading, max_trailing) =
            zstack_horizontal_metrics(&measurements, self.alignment.horizontal());
        let (max_above, max_below) =
            zstack_vertical_metrics(&measurements, self.alignment.vertical());
        let width = if measurements.iter().any(|m| m.size().width.is_infinite()) {
            f32::INFINITY
        } else {
            max_leading + max_trailing
        };
        let height = if measurements.iter().any(|m| m.size().height.is_infinite()) {
            f32::INFINITY
        } else {
            max_above + max_below
        };
        Size::new(width, height)
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        if children.is_empty() {
            return vec![];
        }

        // Placement is a fresh negotiation against the bounds the stack was
        // placed in: every layer is proposed the resolved extent, so what it
        // lays out under is the space it actually has.
        let placement = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                dimensions: child.measure(placement),
            })
            .collect();

        let horizontal = self.alignment.horizontal();
        let vertical = self.alignment.vertical();
        let (max_leading, max_trailing) = zstack_horizontal_metrics(&measurements, horizontal);
        let (max_above, max_below) = zstack_vertical_metrics(&measurements, vertical);
        let line_x = bounds.x()
            + container_line(
                bounds.width(),
                LineAnchor::horizontal(horizontal),
                max_leading,
                max_trailing,
            );
        let line_y = bounds.y()
            + container_line(
                bounds.height(),
                LineAnchor::vertical(vertical),
                max_above,
                max_below,
            );

        let mut placements = Vec::with_capacity(children.len());

        for measurement in &measurements {
            // A layer that answers an unbounded extent fills the bounds; every
            // other layer keeps its answer, overflowing the bounds when it is
            // larger, and sits with its guide on the stack's line.
            let child_width = if measurement.dimensions.size.width.is_infinite() {
                bounds.width()
            } else {
                measurement.dimensions.size.width
            };
            let child_height = if measurement.dimensions.size.height.is_infinite() {
                bounds.height()
            } else {
                measurement.dimensions.size.height
            };

            let child_size = Size::new(child_width, child_height);
            let mut adjusted_dimensions = measurement.dimensions.clone();
            adjusted_dimensions.size = child_size;
            let x = line_x - adjusted_dimensions.horizontal(horizontal);
            let y = line_y - adjusted_dimensions.vertical(vertical);

            placements.push(SubviewPlacement::new(
                Rect::new(Point::new(x, y), child_size),
                placement,
            ));
        }

        placements
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
/// ```rust
/// # use waterui::prelude::*;
/// # fn banner() -> impl View {
/// zstack((
///     Color::blue(),
///     text("Overlay Text"),
/// ))
/// # }
/// ```
///
/// You can control how children align within the stack:
///
/// ```rust
/// # use waterui::prelude::*;
/// # fn aligned(background_view: impl View, content_view: impl View) -> impl View {
/// ZStack::new(Alignment::TopLeading, (
///     background_view,
///     content_view,
/// ))
/// # }
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
    /// Sets the alignment for the `ZStack`. An [`Alignment`] or one of the
    /// tokens (`Leading`, `TopTrailing`, `Center`, …).
    #[must_use]
    pub fn alignment(mut self, alignment: impl Into<Alignment>) -> Self {
        self.layout.alignment = alignment.into();
        self
    }

    crate::alignment::two_dimensional_alignment_methods!();
}

crate::stack::impl_stack_for_each!(ZStack, ZStackLayout);

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

    /// Resolves to `FixedContainer` over the same layout and children;
    /// reports what that container would — matching `FixedContainer`'s
    /// `View::stretch_axis`.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&self.contents.0.stretch_axes())
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

    /// Resolves to `LazyContainer`, which cannot enumerate children without
    /// materializing them and answers its layout's axis over an empty child
    /// set — matching `LazyContainer::stretch_axis`.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&[])
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
    fn test_zstack_placement_center_bounded_proposal() {
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
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout.place(bounds, proposal, &children);

        // Child 1: centered in 100x100
        assert!((placements[0].frame.x() - 30.0).abs() < f32::EPSILON); // (100 - 40) / 2
        assert!((placements[0].frame.y() - 40.0).abs() < f32::EPSILON); // (100 - 20) / 2

        // Child 2: centered in 100x100
        assert!((placements[1].frame.x() - 20.0).abs() < f32::EPSILON); // (100 - 60) / 2
        assert!((placements[1].frame.y() - 30.0).abs() < f32::EPSILON); // (100 - 40) / 2
    }
}
