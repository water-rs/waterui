//! Vertical stack layout.

use alloc::{vec, vec::Vec};
use nami::{Computed, Signal, SignalExt, collection::Collection};
use waterui_core::{
    AnyView, IntoSignalF32, View, env::with, id::Identifiable, layout::LayoutInvalidationCallback,
    view::TupleViews, views::ForEach,
};

use crate::{
    HorizontalAlignment, Layout, LazyContainer, PlacedSubview, Point, ProposalSize, Rect, Size,
    StretchAxis, SubView, SubviewPlacement,
    container::FixedContainer,
    stack::{
        Axis,
        distribute::{
            ChildMeasurement, LineAnchor, container_line, cross_envelope, measure_stack,
            place_cross_extent, stack_spacing,
        },
        stack_stretch_axis,
    },
};

/// Layout engine shared by the public [`VStack`] view.
#[derive(Debug, Clone)]
pub struct VStackLayout {
    /// The horizontal alignment of children within the stack.
    pub alignment: HorizontalAlignment,
    /// The spacing between children in the stack.
    pub spacing: Computed<f32>,
}

impl Default for VStackLayout {
    fn default() -> Self {
        Self {
            alignment: HorizontalAlignment::default(),
            spacing: Computed::constant(10.0),
        }
    }
}

/// The widest child either side of the alignment guide.
///
/// Every child that reports a width counts, the ones that fill the cross axis
/// included. Filling means "at least what I measure, and more if you have it",
/// not "nothing": leaving a filler out of this makes a column of nothing-but-
/// fillers report zero width, and its parent then hands it zero width to fill.
/// A child that answers an unbounded width is answering "as much as you have"
/// rather than naming a size, so it sets no floor here — the fill pass in
/// [`Layout::place`] is what gives it the column's width.
/// The horizontal envelope of the column's children lined up on `alignment`.
fn vstack_intrinsic_cross_metrics(
    measurements: &[ChildMeasurement],
    alignment: HorizontalAlignment,
) -> (f32, f32) {
    cross_envelope(measurements.iter().map(|measurement| {
        (
            measurement.size().width,
            measurement.horizontal_guide(alignment),
        )
    }))
}

impl Layout for VStackLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let spacing = self.spacing.get();
        let measurements = measure_stack(Axis::Vertical, proposal, spacing, children);
        let final_height = measurements.iter().map(|m| m.size().height).sum::<f32>()
            + stack_spacing(spacing, children.len());

        let (leading, trailing) = vstack_intrinsic_cross_metrics(&measurements, self.alignment);
        let width = if measurements.iter().any(|m| m.size().width.is_infinite()) {
            f32::INFINITY
        } else {
            leading + trailing
        };
        Size::new(width, final_height)
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

        let spacing = self.spacing.get();
        // Placement is a fresh negotiation against the bounds the column was
        // placed in: every child is allocated from the column's resolved
        // height and proposed the column's resolved width, so what a child
        // lays out under is the extent it actually has, never the offer the
        // column was measured with. A column widened past its proposal by an
        // unshrinkable row hands that width to every child, so a title lays
        // out across the column it actually has.
        let placement = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let measurements = measure_stack(Axis::Vertical, placement, spacing, children);

        let (above, below) = vstack_intrinsic_cross_metrics(&measurements, self.alignment);
        let guide_line = bounds.x()
            + container_line(
                bounds.width(),
                LineAnchor::horizontal(self.alignment),
                above,
                below,
            );

        // Place children
        let mut placements = Vec::with_capacity(children.len());
        let mut current_y = bounds.y();

        for (i, measurement) in measurements.iter().enumerate() {
            if i > 0 {
                current_y += spacing;
            }

            let child_width = place_cross_extent(
                measurement.size().width,
                bounds.width(),
                measurement.stretches_cross_axis(),
            );

            // Main-axis extent is whatever the last measurement recorded:
            // intrinsic when the column was unspecified, at least the stretch
            // allocation when it was specified.
            let child_height = measurement.size().height;

            let mut adjusted_dimensions = measurement.dimensions.clone();
            adjusted_dimensions.size = Size::new(child_width, child_height);

            // A child that fills the cross axis spans the bounds; every other
            // child sits with its guide on the column's line, wherever that
            // guide lies.
            let x = if measurement.stretches_cross_axis() {
                bounds.x()
            } else {
                guide_line - adjusted_dimensions.horizontal(self.alignment)
            };

            placements.push(SubviewPlacement::new(
                Rect::new(
                    Point::new(x, current_y),
                    Size::new(child_width, child_height),
                ),
                measurement.proposal,
            ));

            current_y += child_height;
        }

        placements
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment == self.alignment {
            return children
                .iter()
                .filter_map(|child| child.explicit_horizontal(alignment))
                .min_by(f32::total_cmp);
        }

        None
    }

    /// A `VStack` claims nothing of its own — a column of labels is
    /// content-sized, like `SwiftUI`'s, and a parent placing an undersized
    /// column centers it per its alignment. What it does claim is whatever its
    /// children claim: filling comes from children that ask for it (`Spacer`,
    /// greedy frames, `Color`), and the ask has to survive the trip up through
    /// every container between that child and whoever owns the space.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        stack_stretch_axis(Axis::Vertical, children)
    }

    fn watch_invalidation(
        &self,
        invalidate: LayoutInvalidationCallback,
    ) -> Vec<nami::watcher::BoxWatcherGuard> {
        vec![self.spacing.watch(move |_| invalidate())]
    }
}

/// A view that arranges its children in a vertical line.
///
/// Use a `VStack` to arrange views top-to-bottom. The stack sizes itself to fit
/// its contents, distributing available space among its children.
///
/// ```rust
/// # use waterui::prelude::*;
/// # fn heading() -> impl View {
/// vstack((
///     text("Title"),
///     text("Subtitle"),
/// ))
/// # }
/// ```
///
/// You can customize the spacing between children and their horizontal alignment:
///
/// ```rust
/// # use waterui::prelude::*;
/// # fn spaced() -> impl View {
/// VStack::new(HorizontalAlignment::Leading, 8.0, (
///     text("First"),
///     text("Second"),
/// ))
/// # }
/// ```
///
/// Use [`spacer()`](crate::spacer()) to push content to the top and bottom:
///
/// ```rust
/// # use waterui::prelude::*;
/// # fn page() -> impl View {
/// vstack((
///     text("Header"),
///     spacer(),
///     text("Footer"),
/// ))
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct VStack<C> {
    layout: VStackLayout,
    contents: C,
}

impl<C: TupleViews> VStack<(C,)> {
    /// Creates a vertical stack with the provided alignment, spacing, and
    /// children.
    pub fn new(alignment: HorizontalAlignment, spacing: f32, contents: C) -> Self {
        Self {
            layout: VStackLayout {
                alignment,
                spacing: Computed::constant(spacing),
            },
            contents: (contents,),
        }
    }
}

crate::stack::impl_stack_for_each!(VStack, VStackLayout);

impl<C> VStack<C> {
    /// Sets the horizontal alignment for children in the stack — a
    /// [`HorizontalAlignment`] or one of the tokens [`Leading`],
    /// [`Center`], [`Trailing`].
    ///
    /// [`Leading`]: crate::Leading
    /// [`Center`]: crate::Center
    /// [`Trailing`]: crate::Trailing
    #[must_use]
    pub fn alignment(mut self, alignment: impl Into<HorizontalAlignment>) -> Self {
        self.layout.alignment = alignment.into();
        self
    }

    crate::alignment::horizontal_alignment_methods!();

    /// Sets the spacing between children in the stack.
    ///
    /// Accepts any numeric literal or signal of `f32`. Signal changes invalidate
    /// only this stack's layout.
    #[must_use]
    pub fn spacing(mut self, spacing: impl IntoSignalF32 + 'static) -> Self {
        self.layout.spacing = spacing.into_signal_f32().computed();
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
pub fn vstack<C: TupleViews>(contents: C) -> VStack<(C,)> {
    VStack::new(HorizontalAlignment::Center, 10.0, contents)
}

impl<C, F, V> View for VStack<ForEach<C, F, V>>
where
    C: Collection + Clone,
    C::Item: Identifiable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Inject the vertical axis into the container
        with(
            LazyContainer::new(self.layout, self.contents),
            Axis::Vertical,
        )
    }

    /// Resolves to `LazyContainer`, which cannot enumerate children without
    /// materializing them and answers its layout's axis over an empty child
    /// set — matching `LazyContainer::stretch_axis`.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&[])
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

    /// Resolves to `FixedContainer` over the same layout and children;
    /// reports what that container would — matching `FixedContainer`'s
    /// `View::stretch_axis`.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&self.contents.0.stretch_axes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewDimensions;
    use crate::tests::CompressibleHeightView;

    struct MockSubView {
        size: Size,
        stretch_axis: StretchAxis,
    }

    impl SubView for MockSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
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
            spacing: Computed::constant(10.0),
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

        assert!((size.width - 100.0).abs() < f32::EPSILON); // max width
        assert!((size.height - 80.0).abs() < f32::EPSILON); // 30 + 10 + 40
    }

    /// A column holding a header above something that grows — a list, a scroll
    /// view — must report the height of both. Reporting only the header's makes
    /// an unconstrained parent hand the column that much and the list draws
    /// nothing at all.
    #[test]
    fn a_column_counts_a_growing_child_in_its_intrinsic_height() {
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Leading,
            spacing: Computed::constant(0.0),
        };

        let mut header = MockSubView {
            size: Size::new(100.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        // A list measures its content and also accepts more room if offered.
        let mut list = MockSubView {
            size: Size::new(100.0, 200.0),
            stretch_axis: StretchAxis::Both,
        };
        let children: Vec<&dyn SubView> = vec![&mut header, &mut list];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        assert!(
            (size.height - 230.0).abs() < f32::EPSILON,
            "the column dropped its growing child from its own height: got {}, want 230",
            size.height
        );
    }

    /// A spacer contributes nothing, so counting growing children must not
    /// inflate a column that merely holds one.
    #[test]
    fn a_spacer_adds_nothing_to_the_intrinsic_height() {
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Leading,
            spacing: Computed::constant(0.0),
        };

        let mut row = MockSubView {
            size: Size::new(100.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            stretch_axis: StretchAxis::Both,
        };
        let children: Vec<&dyn SubView> = vec![&mut row, &mut spacer];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        assert!(
            (size.height - 30.0).abs() < f32::EPSILON,
            "a spacer changed the column's intrinsic height: got {}, want 30",
            size.height
        );
    }

    #[test]
    fn test_vstack_with_spacer() {
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(0.0),
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

        assert!((size.height - 200.0).abs() < f32::EPSILON);

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

        let placements = layout.place(bounds, ProposalSize::new(None, Some(200.0)), &children);

        assert!((placements[0].frame.height() - 30.0).abs() < f32::EPSILON);
        assert!((placements[1].frame.height() - 140.0).abs() < f32::EPSILON); // 200 - 30 - 30
        assert!((placements[2].frame.height() - 30.0).abs() < f32::EPSILON);
        assert!((placements[2].frame.y() - 170.0).abs() < f32::EPSILON); // 30 + 140
    }

    /// A child that fills the cross axis still contributes the width it
    /// measures. Filling means "at least this, and more if you have it": a
    /// column that reported less than its widest child would be handed less,
    /// and the child would fill a box too small for it.
    #[test]
    fn a_cross_filling_child_still_sets_the_column_width() {
        // TextField-like component: stretches horizontally but has fixed height
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(10.0),
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

        // Width: max of every child = max(50, 100, 80) = 100
        assert!(
            (size.width - 100.0).abs() < f32::EPSILON,
            "the text field's own width has to reach the column, got {}",
            size.width
        );
        // Height: all children contribute (text_field doesn't stretch vertically)
        // = 20 + 10 + 40 + 10 + 44 = 124
        assert!((size.height - 124.0).abs() < f32::EPSILON);
    }

    /// A column never reports less than its widest child, whichever way it is
    /// asked. The minimum a window may shrink to and the size the column wants
    /// are the same question about the same children.
    #[test]
    fn a_column_reports_its_widest_child_however_it_is_asked() {
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(10.0),
        };

        let mut label = MockSubView {
            size: Size::new(50.0, 20.0),
            stretch_axis: StretchAxis::None,
        };
        let mut toggle = MockSubView {
            size: Size::new(200.0, 40.0), // Toggle with label has larger min width
            stretch_axis: StretchAxis::Horizontal, // but it stretches horizontally
        };
        let mut button = MockSubView {
            size: Size::new(80.0, 44.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut label, &mut toggle, &mut button];

        // Width: max of every child = max(50, 200, 80) = 200
        let min_size = layout.size_that_fits(ProposalSize::ZERO, &children);
        assert!(
            (min_size.width - 200.0).abs() < f32::EPSILON,
            "a window cannot shrink below the toggle, got {}",
            min_size.width
        );

        let children2: Vec<&dyn SubView> = vec![&mut label, &mut toggle, &mut button];
        let intrinsic_size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children2);
        assert!(
            (intrinsic_size.width - 200.0).abs() < f32::EPSILON,
            "and it wants the same width when nothing constrains it, got {}",
            intrinsic_size.width
        );
    }

    #[test]
    fn compressible_children_shrink_to_fit_the_column_bounded_proposal() {
        // Three 60pt rows in 120pt: a column used to just overflow, because
        // `VStack` summed heights and never asked anyone to give way. Content
        // that can shrink now does, evenly.
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(0.0),
        };

        let mut rows: Vec<CompressibleHeightView> = (0..3)
            .map(|_| CompressibleHeightView {
                ideal: Size::new(50.0, 60.0),
                floor: 0.0,
            })
            .collect();
        let children: Vec<&dyn SubView> = rows.iter_mut().map(|row| row as &dyn SubView).collect();

        let bounds = Rect::new(Point::zero(), Size::new(50.0, 120.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout.place(bounds, proposal, &children);

        for placement in &placements {
            assert!(
                (placement.frame.height() - 40.0).abs() < 0.01,
                "expected each row to give up 20pt, got {}",
                placement.frame.height()
            );
        }
    }

    #[test]
    fn a_row_never_shrinks_below_the_height_it_reports_bounded_proposal() {
        // The middle row will not go below 50pt, so the others absorb what they
        // can and the column overflows by the remainder rather than crushing it.
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(0.0),
        };

        let mut top = CompressibleHeightView {
            ideal: Size::new(50.0, 60.0),
            floor: 0.0,
        };
        let mut middle = CompressibleHeightView {
            ideal: Size::new(50.0, 60.0),
            floor: 50.0,
        };
        let mut bottom = CompressibleHeightView {
            ideal: Size::new(50.0, 60.0),
            floor: 0.0,
        };
        let children: Vec<&dyn SubView> = vec![&mut top, &mut middle, &mut bottom];

        let bounds = Rect::new(Point::zero(), Size::new(50.0, 90.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout.place(bounds, proposal, &children);

        assert!(
            placements[1].frame.height() >= 50.0 - 0.01,
            "the middle row reported a 50pt floor, got {}",
            placements[1].frame.height()
        );
    }
}
