use alloc::{boxed::Box, vec, vec::Vec};

use nami::Computed;

use crate::stack::{Axis, HStackLayout, VStackLayout};
use crate::{
    HorizontalAlignment, Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    VerticalAlignment, ViewDimensions, measure_layout,
};

struct RangeLeaf {
    axis: Axis,
    minimum: f32,
    ideal: f32,
    maximum: f32,
    cross: f32,
}

impl SubView for RangeLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal)
            .map_or(self.ideal, |value| value.clamp(self.minimum, self.maximum));
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        if self.maximum.is_infinite() {
            StretchAxis::MainAxis
        } else {
            StretchAxis::None
        }
    }

    fn priority(&self) -> i32 {
        0
    }
}

struct LayoutNode {
    layout: Box<dyn Layout>,
    children: Vec<Box<dyn SubView>>,
}

impl LayoutNode {
    fn child_refs(&self) -> Vec<&dyn SubView> {
        self.children.iter().map(AsRef::as_ref).collect()
    }
}

impl SubView for LayoutNode {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_layout(self.layout.as_ref(), proposal, &self.child_refs())
    }

    fn stretch_axis(&self) -> StretchAxis {
        let axes: Vec<_> = self
            .children
            .iter()
            .map(|child| child.stretch_axis())
            .collect();
        self.layout.stretch_axis(&axes)
    }

    fn priority(&self) -> i32 {
        0
    }
}

fn stack(axis: Axis) -> Box<dyn Layout> {
    if axis.is_horizontal() {
        Box::new(HStackLayout {
            alignment: VerticalAlignment::Top,
            spacing: Computed::constant(0.0),
        })
    } else {
        Box::new(VStackLayout {
            alignment: HorizontalAlignment::Leading,
            spacing: Computed::constant(0.0),
        })
    }
}

const fn axis_size(axis: Axis, main: f32, cross: f32) -> Size {
    if axis.is_horizontal() {
        Size::new(main, cross)
    } else {
        Size::new(cross, main)
    }
}

fn axis_proposal(axis: Axis, main: Option<f32>, cross: Option<f32>) -> ProposalSize {
    if axis.is_horizontal() {
        ProposalSize::new(main, cross)
    } else {
        ProposalSize::new(cross, main)
    }
}

const fn main_proposal(axis: Axis, proposal: ProposalSize) -> Option<f32> {
    if axis.is_horizontal() {
        proposal.width
    } else {
        proposal.height
    }
}

const fn main_extent(axis: Axis, size: Size) -> f32 {
    if axis.is_horizontal() {
        size.width
    } else {
        size.height
    }
}

const fn main_origin(axis: Axis, frame: Rect) -> f32 {
    if axis.is_horizontal() {
        frame.x()
    } else {
        frame.y()
    }
}

fn flexible_children(axis: Axis) -> Vec<Box<dyn SubView>> {
    [(20.0, 40.0), (60.0, 120.0)]
        .into_iter()
        .map(|(minimum, ideal)| {
            Box::new(RangeLeaf {
                axis,
                minimum,
                ideal,
                maximum: f32::INFINITY,
                cross: 20.0,
            }) as Box<dyn SubView>
        })
        .collect()
}

fn sections(axis: Axis) -> Vec<Box<dyn SubView>> {
    [40.0, 120.0]
        .into_iter()
        .map(|extent| {
            Box::new(LayoutNode {
                layout: stack(axis),
                children: vec![
                    Box::new(RangeLeaf {
                        axis,
                        minimum: extent,
                        ideal: extent,
                        maximum: extent,
                        cross: 20.0,
                    }),
                    Box::new(RangeLeaf {
                        axis,
                        minimum: 0.0,
                        ideal: 0.0,
                        maximum: f32::INFINITY,
                        cross: 0.0,
                    }),
                ],
            }) as Box<dyn SubView>
        })
        .collect()
}

fn assert_extent(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() < 0.001,
        "{context}: expected {expected}, got {actual}"
    );
}

/// Measures the stack under `main`, then places it at its own answer and
/// checks the allocation. `expected` is what placement hands each child from
/// the resolved bounds; it equals the measured allocation under a finite
/// proposal and, under an unspecified one, the distribution of the sum of
/// ideals — placement negotiates against the bounds, never a replay of the
/// ideal probe.
fn assert_allocation(axis: Axis, children: &[&dyn SubView], main: Option<f32>, expected: [f32; 2]) {
    let layout = stack(axis);
    let proposal = axis_proposal(axis, main, Some(100.0));
    let size = layout.size_that_fits(proposal, children);
    let total = expected.iter().sum();
    assert_extent(main_extent(axis, size), total, "container main extent");
    let bounds = Rect::new(Point::new(11.0, -7.0), size);
    let placements = layout.place(bounds, proposal, children);
    assert_eq!(placements.len(), expected.len());
    let mut cursor = main_origin(axis, bounds);
    for ((placement, child), expected) in placements.iter().zip(children).zip(expected) {
        assert_extent(
            main_extent(axis, *placement.frame.size()),
            expected,
            "child main extent",
        );
        assert_extent(
            main_origin(axis, placement.frame),
            cursor,
            "child main origin",
        );
        // Placement proposes each child its allocation, whatever the stack
        // itself was measured with.
        assert_extent(
            main_proposal(axis, placement.proposal).expect("placement proposes a finite extent"),
            expected,
            "selected main proposal",
        );
        let response = child.measure(placement.proposal);
        assert_extent(
            main_extent(axis, response.size),
            expected,
            "selected proposal response",
        );
        cursor += expected;
    }
}

#[test]
fn flexible_children_follow_reference_allocations_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children = flexible_children(axis);
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        // Unspecified: the stack answers the sum of ideals (160) and, placed
        // at that extent, splits it between two equally flexible children.
        for (main, expected) in [
            (None, [80.0, 80.0]),
            (Some(80.0), [20.0, 60.0]),
            (Some(160.0), [80.0, 80.0]),
            (Some(240.0), [120.0, 120.0]),
            (Some(320.0), [160.0, 160.0]),
        ] {
            assert_allocation(axis, &refs, main, expected);
        }
    }
}

#[test]
fn nested_sections_follow_reference_allocations_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children = sections(axis);
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        for (main, expected) in [
            (None, [40.0, 120.0]),
            (Some(80.0), [40.0, 120.0]),
            (Some(160.0), [40.0, 120.0]),
            (Some(240.0), [120.0, 120.0]),
            (Some(320.0), [160.0, 160.0]),
        ] {
            assert_allocation(axis, &refs, main, expected);
        }
    }
}

/// Placement is a function of the bounds alone: the same bounds allocate the
/// same way whether the stack was measured unspecified or bounded, and
/// whatever other probes ran in between.
#[test]
fn equal_bounds_allocate_identically_whatever_the_proposal_or_probe_order() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let layout = stack(axis);
        let children = flexible_children(axis);
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let unspecified = axis_proposal(axis, None, Some(100.0));
        let bounded = axis_proposal(axis, Some(160.0), Some(100.0));
        let ideal = layout.size_that_fits(unspecified, &refs);
        assert_eq!(ideal, layout.size_that_fits(bounded, &refs));
        let bounds = Rect::from_size(ideal);
        for other in [Some(320.0), Some(0.0), Some(80.0), None] {
            layout.size_that_fits(axis_proposal(axis, other, Some(100.0)), &refs);
            for proposal in [unspecified, bounded] {
                let placements = layout.place(bounds, proposal, &refs);
                assert_eq!(placements.len(), 2);
                for placement in &placements {
                    assert_extent(
                        main_extent(axis, *placement.frame.size()),
                        80.0,
                        "probe order",
                    );
                    assert_eq!(main_proposal(axis, placement.proposal), Some(80.0));
                }
            }
        }
    }
}

#[test]
fn rigid_cross_axis_response_is_preserved_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let layout = stack(axis);
        let child = RangeLeaf {
            axis,
            minimum: 40.0,
            ideal: 40.0,
            maximum: 40.0,
            cross: 20.0,
        };
        for cross in [Some(10.0), Some(200.0), None] {
            let proposal = axis_proposal(axis, None, cross);
            assert_eq!(
                layout.size_that_fits(proposal, &[&child]),
                axis_size(axis, 40.0, 20.0)
            );
        }
    }
}

#[test]
fn rigid_cross_axis_placement_overflows_a_smaller_host_region() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let layout = stack(axis);
        let child = RangeLeaf {
            axis,
            minimum: 40.0,
            ideal: 40.0,
            maximum: 40.0,
            cross: 20.0,
        };
        let proposal = axis_proposal(axis, None, Some(10.0));
        let bounds = Rect::new(Point::new(11.0, -7.0), axis_size(axis, 40.0, 10.0));
        let placements = layout.place(bounds, proposal, &[&child]);
        assert_eq!(*placements[0].frame.size(), axis_size(axis, 40.0, 20.0));
        // The bounds, not the measurement probe, are what placement proposes.
        assert_eq!(
            placements[0].proposal,
            axis_proposal(axis, Some(40.0), Some(10.0))
        );
    }
}

#[test]
fn nested_rigid_stacks_preserve_mixed_axis_probe_responses() {
    let mut node: Box<dyn SubView> = Box::new(RangeLeaf {
        axis: Axis::Horizontal,
        minimum: 40.0,
        ideal: 40.0,
        maximum: 40.0,
        cross: 20.0,
    });
    for depth in 0..8 {
        node = Box::new(LayoutNode {
            layout: stack(if depth % 2 == 0 {
                Axis::Horizontal
            } else {
                Axis::Vertical
            }),
            children: vec![node],
        });
    }
    for width in [
        None,
        Some(0.0),
        Some(10.0),
        Some(200.0),
        Some(f32::INFINITY),
    ] {
        for height in [
            None,
            Some(0.0),
            Some(10.0),
            Some(200.0),
            Some(f32::INFINITY),
        ] {
            assert_eq!(
                node.measure(ProposalSize::new(width, height)).size,
                Size::new(40.0, 20.0)
            );
        }
    }
}

#[test]
fn spacing_invalidation_and_membership_changes_return_to_original_geometry() {
    use alloc::rc::Rc;
    use core::cell::Cell;
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let spacing = nami::binding(10.0_f32);
        let layout: Box<dyn Layout> = if axis.is_horizontal() {
            Box::new(HStackLayout {
                alignment: VerticalAlignment::Top,
                spacing: spacing.clone().into(),
            })
        } else {
            Box::new(VStackLayout {
                alignment: HorizontalAlignment::Leading,
                spacing: spacing.clone().into(),
            })
        };
        let invalidations = Rc::new(Cell::new(0));
        let observer = Rc::clone(&invalidations);
        let _guards = layout.watch_invalidation(Rc::new(move || observer.set(observer.get() + 1)));
        let leaf = RangeLeaf {
            axis,
            minimum: 40.0,
            ideal: 40.0,
            maximum: 40.0,
            cross: 20.0,
        };
        let all: [&dyn SubView; 3] = [&leaf, &leaf, &leaf];
        for (gap, count, expected) in [(10.0, 2, 90.0), (20.0, 3, 160.0), (10.0, 2, 90.0)] {
            spacing.set(gap);
            let children = &all[..count];
            for offer in [Some(300.0), Some(50.0), None] {
                let proposal = axis_proposal(axis, offer, Some(100.0));
                let size = layout.size_that_fits(proposal, children);
                assert_eq!(size, axis_size(axis, expected, 20.0));
                let bounds = Rect::new(Point::new(11.0, -7.0), size);
                let placements = layout.place(bounds, proposal, children);
                assert_eq!(placements.len(), count);
                let mut origin = main_origin(axis, bounds);
                for placement in placements {
                    assert_extent(
                        main_origin(axis, placement.frame),
                        origin,
                        "membership order",
                    );
                    assert_eq!(*placement.frame.size(), axis_size(axis, 40.0, 20.0));
                    origin += 40.0 + gap;
                }
            }
        }
        assert!(
            invalidations.get() >= 2,
            "spacing changes must invalidate the existing layout"
        );
    }
}

struct CrossFillLeaf(Axis);

impl SubView for CrossFillLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let cross = if self.0.is_horizontal() {
            proposal.height
        } else {
            proposal.width
        };
        ViewDimensions::new(axis_size(
            self.0,
            40.0,
            cross.map_or(20.0, |value| value.max(10.0)),
        ))
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::CrossAxis
    }
    fn priority(&self) -> i32 {
        0
    }
}

#[test]
fn cross_axis_maximum_query_preserves_unbounded_response() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let child = CrossFillLeaf(axis);
        let layout = stack(axis);
        let proposal = axis_proposal(axis, None, Some(f32::INFINITY));
        assert_eq!(
            layout.size_that_fits(proposal, &[&child]),
            axis_size(axis, 40.0, f32::INFINITY)
        );
    }
}

#[test]
fn cross_axis_fill_preserves_its_minimum_in_small_bounds() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let child = CrossFillLeaf(axis);
        let proposal = axis_proposal(axis, None, Some(5.0));
        let placements = stack(axis).place(
            Rect::from_size(axis_size(axis, 40.0, 5.0)),
            proposal,
            &[&child],
        );
        assert_eq!(*placements[0].frame.size(), axis_size(axis, 40.0, 10.0));
    }
}

/// A leaf that wraps: it takes the width it is offered (never more than its
/// unwrapped width) and grows taller the narrower it gets.
struct WrappingLeaf {
    axis: Axis,
    unwrapped: f32,
    area: f32,
}

impl SubView for WrappingLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let cross = cross_proposal(self.axis, proposal)
            .unwrap_or(self.unwrapped)
            .clamp(1.0, self.unwrapped);
        ViewDimensions::new(axis_size(self.axis, self.area / cross, cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

fn cross_proposal(axis: Axis, proposal: ProposalSize) -> Option<f32> {
    if axis.is_horizontal() {
        proposal.height
    } else {
        proposal.width
    }
}

/// A stack widened past its proposal by a rigid child proposes its resolved
/// cross extent at placement: the wrapping sibling lays out across the whole
/// stack, as `SwiftUI`'s does, instead of staying wrapped at the proposal.
#[test]
fn placement_proposes_the_resolved_cross_extent() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let layout = stack(axis);
        let rigid = RangeLeaf {
            axis,
            minimum: 40.0,
            ideal: 40.0,
            maximum: 40.0,
            cross: 414.0,
        };
        let wrapping = WrappingLeaf {
            axis,
            unwrapped: 414.0,
            area: 414.0 * 10.0,
        };
        let proposal = axis_proposal(axis, None, Some(370.0));
        let measured = layout.size_that_fits(proposal, &[&rigid, &wrapping]);
        // Measured at 370 the wrapping child needs two lines' worth of main
        // extent; the rigid child widens the stack to 414 regardless.
        assert_extent(cross_extent(axis, measured), 414.0, "stack cross extent");

        let bounds = Rect::from_size(axis_size(axis, main_extent(axis, measured), 414.0));
        let placements = layout.place(bounds, proposal, &[&rigid, &wrapping]);
        assert_eq!(
            cross_proposal(axis, placements[1].proposal),
            Some(414.0),
            "the wrapping child is proposed the stack's resolved cross extent"
        );
        assert_extent(
            cross_extent(axis, *placements[1].frame.size()),
            414.0,
            "wrapping child cross extent",
        );
        assert_extent(
            main_extent(axis, *placements[1].frame.size()),
            10.0,
            "wrapping child main extent (one line)",
        );
        assert_eq!(
            main_proposal(axis, placements[0].proposal),
            Some(40.0),
            "the rigid child is proposed its allocation"
        );
    }
}

/// A stack measured narrower than its offer proposes its own extent at
/// placement, not the offer: a content-sized card in a wide column hands its
/// children the card's width.
#[test]
fn placement_proposes_the_resolved_extent_on_underfill() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let layout = stack(axis);
        let child = RangeLeaf {
            axis,
            minimum: 30.0,
            ideal: 30.0,
            maximum: 30.0,
            cross: 140.0,
        };
        let proposal = axis_proposal(axis, Some(500.0), Some(362.0));
        let measured = layout.size_that_fits(proposal, &[&child]);
        assert_extent(cross_extent(axis, measured), 140.0, "stack cross extent");
        assert_extent(main_extent(axis, measured), 30.0, "stack main extent");
        let placements = layout.place(Rect::from_size(measured), proposal, &[&child]);
        assert_eq!(cross_proposal(axis, placements[0].proposal), Some(140.0));
        assert_eq!(main_proposal(axis, placements[0].proposal), Some(30.0));
    }
}

/// A leaf whose answer on one axis follows the other, like an image kept in
/// a square: it answers the ideal to an unspecified offer and fits inside a
/// finite one.
struct SquareLeaf(f32);

impl SubView for SquareLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let side = match (proposal.width, proposal.height) {
            (Some(width), Some(height)) if width.is_finite() && height.is_finite() => {
                width.min(height)
            }
            (Some(width), _) if width.is_finite() => width,
            (_, Some(height)) if height.is_finite() => height,
            _ => self.0,
        };
        ViewDimensions::new(Size::new(side, side))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// A stack widened by a rigid sibling hands the resolved cross extent to a
/// child whose main extent follows it. Because placement also allocates the
/// main axis from the resolved bounds, that child takes the room the rigid
/// sibling leaves (100 of 110) instead of growing to the 200 the cross extent
/// alone would let it claim, and the children end where the stack reported.
#[test]
fn placement_allocates_the_main_axis_from_the_resolved_bounds() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let layout = stack(axis);
        let square = SquareLeaf(100.0);
        let rigid = RangeLeaf {
            axis,
            minimum: 10.0,
            ideal: 10.0,
            maximum: 10.0,
            cross: 200.0,
        };
        let proposal = ProposalSize::new(None, None);
        let measured = layout.size_that_fits(proposal, &[&square, &rigid]);
        assert_extent(cross_extent(axis, measured), 200.0, "stack cross extent");
        assert_extent(main_extent(axis, measured), 110.0, "stack main extent");

        let bounds = Rect::from_size(measured);
        let placements = layout.place(bounds, proposal, &[&square, &rigid]);
        assert_eq!(cross_proposal(axis, placements[0].proposal), Some(200.0));
        assert_eq!(main_proposal(axis, placements[0].proposal), Some(100.0));
        assert_extent(
            main_extent(axis, *placements[0].frame.size()),
            100.0,
            "square main extent",
        );
        let end =
            main_origin(axis, placements[1].frame) + main_extent(axis, *placements[1].frame.size());
        assert_extent(end, 110.0, "children end where the stack reported");
    }
}

/// A leaf carrying an explicit guide.
struct GuidedLeaf {
    size: Size,
    top: f32,
}

impl SubView for GuidedLeaf {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        let mut dimensions = ViewDimensions::new(self.size);
        dimensions.set_vertical(VerticalAlignment::Top, self.top);
        dimensions
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Explicit guides are honoured on every alignment and never clamped: a row
/// aligned on `top` whose child declares its top guide 10 above its edge is
/// the envelope of the guides, and the child sits 10 below the row's line.
#[test]
fn explicit_guides_shape_the_envelope_on_edge_alignments() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Top,
        spacing: Computed::constant(0.0),
    };
    let raised = GuidedLeaf {
        size: Size::new(10.0, 20.0),
        top: -10.0,
    };
    let plain = RangeLeaf {
        axis: Axis::Horizontal,
        minimum: 10.0,
        ideal: 10.0,
        maximum: 10.0,
        cross: 30.0,
    };
    let proposal = ProposalSize::new(None, None);
    let measured = layout.size_that_fits(proposal, &[&raised, &plain]);
    // Line at max guide 0 (the plain child); extents past the line: 30 for
    // the plain child, 20 - (-10) = 30 for the raised one.
    assert_extent(measured.height, 30.0, "row height");
    let placements = layout.place(Rect::from_size(measured), proposal, &[&raised, &plain]);
    assert_extent(
        placements[0].frame.y(),
        10.0,
        "raised child sits below the line",
    );
    assert_extent(placements[1].frame.y(), 0.0, "plain child sits on the line");

    // Alone, the raised child's guide is the line itself: the row is as tall
    // as the child and the child starts at the row's top.
    let measured = layout.size_that_fits(proposal, &[&raised]);
    assert_extent(measured.height, 20.0, "row height around one raised child");
    let placements = layout.place(Rect::from_size(measured), proposal, &[&raised]);
    assert_extent(
        placements[0].frame.y(),
        0.0,
        "lone raised child starts at the top",
    );
}

const fn cross_extent(axis: Axis, size: Size) -> f32 {
    if axis.is_horizontal() {
        size.height
    } else {
        size.width
    }
}
