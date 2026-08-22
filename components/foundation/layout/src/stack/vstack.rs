//! Vertical stack layout.

use alloc::{vec, vec::Vec};
use nami::{Computed, Signal, SignalExt, collection::Collection};
use waterui_core::{
    AnyView, IntoSignalF32, View, env::with, id::Identifiable, layout::LayoutInvalidationCallback,
    view::TupleViews, views::ForEach,
};

use crate::{
    HorizontalAlignment, Layout, LazyContainer, PlacedSubview, Point, ProposalSize, Rect, Size,
    StretchAxis, SubView, ViewDimensions,
    container::FixedContainer,
    stack::{
        Axis,
        distribute::{Extent, compress_to_fit, exceeds},
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

/// Cached measurement for a child during layout
struct ChildMeasurement {
    dimensions: ViewDimensions,
    stretch_axis: StretchAxis,
}

impl ChildMeasurement {
    const fn size(&self) -> Size {
        self.dimensions.size
    }

    fn horizontal_guide(&self, alignment: HorizontalAlignment) -> f32 {
        self.dimensions.horizontal(alignment)
    }

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

fn vstack_intrinsic_cross_metrics(
    measurements: &[ChildMeasurement],
    alignment: HorizontalAlignment,
    include_cross_axis_stretch: bool,
) -> (f32, f32) {
    let mut max_leading = 0.0_f32;
    let mut max_trailing = 0.0_f32;

    for measurement in measurements
        .iter()
        .filter(|m| include_cross_axis_stretch || !m.stretches_cross_axis())
    {
        let size = measurement.size();
        let guide = measurement
            .horizontal_guide(alignment)
            .clamp(0.0, size.width);
        max_leading = max_leading.max(guide);
        max_trailing = max_trailing.max((size.width - guide).max(0.0));
    }

    (max_leading, max_trailing)
}

/// Compresses the children that do not stretch vertically into `available`,
/// taking height from the lowest layout priorities first and never pushing a
/// child below the height it reports when proposed zero.
///
/// `HStack` has always done this on its axis; `VStack` used to just sum heights,
/// so a column taller than its bounds overflowed even when its content could
/// have wrapped or truncated into the space.
fn compress_children(
    measurements: &mut [ChildMeasurement],
    children: &[&dyn SubView],
    compress_indices: &[usize],
    available: f32,
    width_proposal: Option<f32>,
) {
    if compress_indices.is_empty() {
        return;
    }

    // Probing minimums only pays once the column is known not to fit.
    let ideal_total: f32 = compress_indices
        .iter()
        .map(|&index| measurements[index].size().height)
        .sum();
    if !exceeds(ideal_total, available, compress_indices.len()) {
        return;
    }

    let extents: Vec<Extent> = compress_indices
        .iter()
        .map(|&index| Extent {
            ideal: measurements[index].size().height,
            min: children[index]
                .measure(ProposalSize::new(width_proposal, Some(0.0)))
                .size
                .height,
            priority: children[index].priority(),
        })
        .collect();

    for (&index, resolved) in compress_indices
        .iter()
        .zip(compress_to_fit(&extents, available))
    {
        if resolved >= measurements[index].size().height {
            continue;
        }
        let constrained = ProposalSize::new(width_proposal, Some(resolved));
        measurements[index].dimensions = children[index].measure(constrained);
        measurements[index].dimensions.size.height =
            measurements[index].size().height.min(resolved);
    }
}

fn usize_to_f32(value: usize) -> f32 {
    use num_traits::ToPrimitive;
    value
        .to_f32()
        .expect("VStackLayout: child count must be representable as f32")
}

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
                dimensions: child.measure(child_proposal),
                stretch_axis: child.stretch_axis(),
            })
            .collect();

        // VStack checks for main-axis (vertical) stretching
        let has_main_axis_stretch = measurements
            .iter()
            .any(ChildMeasurement::stretches_main_axis);

        // Height: every child's measured height plus spacing. Children that
        // stretch are included: measured under this same proposal a stretcher
        // reports what it actually needs — nothing for a spacer, its content
        // for a list or a scroll view. Leaving them out makes a column that
        // holds, say, a header above a list report only the header's height,
        // and the list is then handed nothing to draw in.
        let content_height: f32 = measurements.iter().map(|m| m.size().height).sum();

        let spacing = self.spacing.get();
        let total_spacing = if children.len() > 1 {
            usize_to_f32(children.len() - 1) * spacing
        } else {
            0.0
        };

        let intrinsic_height = content_height + total_spacing;
        // Only a column that has something able to grow accepts an offer larger
        // than its content; a content-sized column keeps its own height.
        let final_height = if has_main_axis_stretch {
            proposal.height.unwrap_or(intrinsic_height)
        } else {
            intrinsic_height
        };

        // Width: when proposal.width is zero (min size query), include ALL children's widths
        // to ensure container can't shrink below any child's minimum.
        // Otherwise, exclude cross-axis stretching children from intrinsic width calculation.
        let is_min_size_query = proposal.width == Some(0.0);
        let (max_leading, max_trailing) =
            vstack_intrinsic_cross_metrics(&measurements, self.alignment, is_min_size_query);
        let max_width = max_leading + max_trailing;

        // VStack stretches horizontally (cross-axis), so use proposed width when available
        // (unless it's a min-size query where we want the minimum required width)
        let final_width = if is_min_size_query {
            max_width
        } else {
            proposal.width.unwrap_or(max_width)
        };

        Size::new(final_width, final_height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Measure children again (will be cached by SubView implementation)
        let child_proposal = ProposalSize::new(Some(bounds.width()), None);

        let mut measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                dimensions: child.measure(child_proposal),
                stretch_axis: child.stretch_axis(),
            })
            .collect();

        let spacing = self.spacing.get();
        let total_spacing = if children.len() > 1 {
            usize_to_f32(children.len() - 1) * spacing
        } else {
            0.0
        };

        // Fit the non-stretching children into the height on offer before any of
        // it is handed to the stretchers.
        let compress_indices: Vec<usize> = measurements
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.stretches_main_axis())
            .map(|(index, _)| index)
            .collect();
        compress_children(
            &mut measurements,
            children,
            &compress_indices,
            (bounds.height() - total_spacing).max(0.0),
            Some(bounds.width()),
        );

        // Calculate stretch child height - only for main-axis (vertically) stretching children
        let main_axis_stretch_count = measurements
            .iter()
            .filter(|m| m.stretches_main_axis())
            .count();
        let non_stretch_height: f32 = measurements
            .iter()
            .filter(|m| !m.stretches_main_axis())
            .map(|m| m.size().height)
            .sum();

        let remaining_height = bounds.height() - non_stretch_height - total_spacing;
        let stretch_height = if main_axis_stretch_count > 0 {
            (remaining_height / usize_to_f32(main_axis_stretch_count)).max(0.0)
        } else {
            0.0
        };

        let has_explicit_alignment_guides = measurements.iter().any(|measurement| {
            measurement
                .dimensions
                .explicit_horizontal(self.alignment)
                .is_some()
        });
        let guide_line = has_explicit_alignment_guides.then(|| {
            let (intrinsic_leading, _intrinsic_trailing) =
                vstack_intrinsic_cross_metrics(&measurements, self.alignment, false);
            bounds.x() + intrinsic_leading
        });

        // Place children
        let mut rects = Vec::with_capacity(children.len());
        let mut current_y = bounds.y();

        for (i, measurement) in measurements.iter().enumerate() {
            if i > 0 {
                current_y += spacing;
            }

            // Handle cross-axis (horizontal) stretching and infinite width
            let child_width = if measurement.stretches_cross_axis() {
                // CrossAxis in VStack means expand horizontally to full bounds width
                bounds.width()
            } else if measurement.size().width.is_infinite() {
                bounds.width()
            } else {
                // Clamp child width to bounds - child can't be wider than container
                measurement.size().width.min(bounds.width())
            };

            let child_height = if measurement.stretches_main_axis() {
                stretch_height
            } else {
                measurement.size().height
            };

            let mut adjusted_dimensions = measurement.dimensions.clone();
            adjusted_dimensions.size = Size::new(child_width, child_height);

            let x = if measurement.stretches_cross_axis() {
                bounds.x()
            } else if let Some(guide_line) = guide_line {
                let guide = adjusted_dimensions
                    .horizontal(self.alignment)
                    .clamp(0.0, child_width);
                guide_line - guide
            } else if self.alignment == HorizontalAlignment::Leading {
                bounds.x()
            } else if self.alignment == HorizontalAlignment::Trailing {
                bounds.x() + bounds.width() - child_width
            } else {
                let guide = adjusted_dimensions
                    .horizontal(self.alignment)
                    .clamp(0.0, child_width);
                bounds.x() + bounds.width() * 0.5 - guide
            };

            rects.push(Rect::new(
                Point::new(x, current_y),
                Size::new(child_width, child_height),
            ));

            current_y += child_height;
        }

        rects
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

    /// `VStack` is content-sized on both axes, like `SwiftUI`'s: it never
    /// claims the cross axis for itself. Filling comes from children that ask
    /// for it (`Spacer`, greedy frames, `Color`), and a parent placing an
    /// undersized stack centers it per its alignment.
    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::None
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
    /// Sets the horizontal alignment for children in the stack.
    #[must_use]
    pub const fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

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

        let rects = layout.place(bounds, &children);

        assert!((rects[0].height() - 30.0).abs() < f32::EPSILON);
        assert!((rects[1].height() - 140.0).abs() < f32::EPSILON); // 200 - 30 - 30
        assert!((rects[2].height() - 30.0).abs() < f32::EPSILON);
        assert!((rects[2].y() - 170.0).abs() < f32::EPSILON); // 30 + 140
    }

    #[test]
    fn test_vstack_with_horizontal_stretch() {
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

        // Width: max of non-horizontal-stretching children = max(50, 80) = 80
        // Note: text_field stretches horizontally so its width doesn't contribute
        assert!((size.width - 80.0).abs() < f32::EPSILON);
        // Height: all children contribute (text_field doesn't stretch vertically)
        // = 20 + 10 + 40 + 10 + 44 = 124
        assert!((size.height - 124.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_vstack_min_size_query_includes_all_children() {
        // When ProposalSize::ZERO is used (min size query), ALL children's widths
        // should be included, even stretching ones. This is essential for window min size.
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

        // With ZERO proposal (min size query), toggle's width SHOULD be included
        let min_size = layout.size_that_fits(ProposalSize::ZERO, &children);
        // Width: max of ALL children = max(50, 200, 80) = 200
        assert!(
            (min_size.width - 200.0).abs() < f32::EPSILON,
            "Min size query should include stretching children's widths, got {}",
            min_size.width
        );

        // Verify existing behavior: UNSPECIFIED excludes stretching children
        let children2: Vec<&dyn SubView> = vec![&mut label, &mut toggle, &mut button];
        let intrinsic_size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children2);
        // Width: max of non-stretching children = max(50, 80) = 80
        assert!(
            (intrinsic_size.width - 80.0).abs() < f32::EPSILON,
            "Unspecified proposal should exclude stretching children's widths, got {}",
            intrinsic_size.width
        );
    }

    #[test]
    fn compressible_children_shrink_to_fit_the_column() {
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
        let rects = layout.place(bounds, &children);

        for rect in &rects {
            assert!(
                (rect.height() - 40.0).abs() < 0.01,
                "expected each row to give up 20pt, got {}",
                rect.height()
            );
        }
    }

    #[test]
    fn a_row_never_shrinks_below_the_height_it_reports() {
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
        let rects = layout.place(bounds, &children);

        assert!(
            rects[1].height() >= 50.0 - 0.01,
            "the middle row reported a 50pt floor, got {}",
            rects[1].height()
        );
    }

    /// A row that shrinks to whatever height it is proposed, down to `floor`.
    struct CompressibleHeightView {
        ideal: Size,
        floor: f32,
    }

    impl SubView for CompressibleHeightView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            let height = proposal.height.map_or(self.ideal.height, |proposed| {
                proposed.clamp(self.floor, self.ideal.height)
            });
            ViewDimensions::new(Size::new(self.ideal.width, height))
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }
}
