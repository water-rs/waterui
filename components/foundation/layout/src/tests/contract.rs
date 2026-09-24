use alloc::{boxed::Box, vec, vec::Vec};

use nami::Computed;

use crate::stack::{Axis, HStackLayout, VStackLayout};
use crate::{
    HorizontalAlignment, Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement, VerticalAlignment, ViewDimensions, measure_layout,
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
            .filter(|child| !child.is_empty())
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

const fn cross_origin(axis: Axis, frame: Rect) -> f32 {
    if axis.is_horizontal() {
        frame.y()
    } else {
        frame.x()
    }
}

/// A stack child that toggles between rendering a view and rendering
/// nothing — the shape a `when(condition, ..)` presents to its stack.
struct ConditionalChild {
    renders_nothing: bool,
    size: Size,
}

impl SubView for ConditionalChild {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(self.size)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        self.renders_nothing
    }
}

/// §4.4 — a child that renders nothing is not a stack member: it takes no
/// slot and no spacing, and a conditional switching between rendering
/// nothing and rendering a view is a membership change.
#[test]
fn a_child_that_renders_nothing_is_not_a_stack_member() {
    let spacing = 10.0f32;
    let layout = VStackLayout {
        spacing: Computed::constant(spacing),
        ..VStackLayout::default()
    };
    let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(80.0, 400.0));
    let sibling = Size::new(40.0, 20.0);

    // Hidden: two siblings and a conditional that renders nothing between
    // them — the siblings sit exactly one spacing apart.
    let first = ConditionalChild {
        renders_nothing: false,
        size: sibling,
    };
    let hidden = ConditionalChild {
        renders_nothing: true,
        size: sibling,
    };
    let last = ConditionalChild {
        renders_nothing: false,
        size: sibling,
    };
    let children: [&dyn SubView; 3] = [&first, &hidden, &last];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
    assert_extent(
        size.height,
        sibling.height * 2.0 + spacing,
        "hidden conditional: stack height is the two members and one gap",
    );
    let placements = layout.place(bounds, ProposalSize::UNSPECIFIED, &children);
    assert_eq!(
        placements.len(),
        children.len(),
        "place answers one placement per child, members or not"
    );
    assert_extent(placements[0].frame.y(), 0.0, "first member at the top");
    assert_extent(
        placements[1].frame.height(),
        0.0,
        "the hidden child is placed with no slot",
    );
    assert_extent(
        placements[2].frame.y(),
        sibling.height + spacing,
        "second member one spacing below the first",
    );

    // Shown: the conditional is a member again — the stack returns to three
    // children and two gaps.
    let first = ConditionalChild {
        renders_nothing: false,
        size: sibling,
    };
    let shown = ConditionalChild {
        renders_nothing: false,
        size: sibling,
    };
    let last = ConditionalChild {
        renders_nothing: false,
        size: sibling,
    };
    let children: [&dyn SubView; 3] = [&first, &shown, &last];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
    assert_extent(
        size.height,
        sibling.height * 3.0 + spacing * 2.0,
        "shown conditional: stack height is three members and two gaps",
    );
    let placements = layout.place(bounds, ProposalSize::UNSPECIFIED, &children);
    assert_extent(
        placements[2].frame.y(),
        sibling.height * 2.0 + spacing * 2.0,
        "second member two spacings below the first",
    );
}

/// A zero measured size is not emptiness: a member that happens to measure
/// nothing — a collapsed `Spacer` — still claims its slot and the gaps
/// around it.
#[test]
fn a_zero_sized_child_is_still_a_member() {
    let spacing = 10.0f32;
    let layout = VStackLayout {
        spacing: Computed::constant(spacing),
        ..VStackLayout::default()
    };
    let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(80.0, 400.0));

    let collapsed = ConditionalChild {
        renders_nothing: false,
        size: Size::zero(),
    };
    let leaf = ConditionalChild {
        renders_nothing: false,
        size: Size::new(40.0, 20.0),
    };
    let children: [&dyn SubView; 3] = [&leaf, &collapsed, &leaf];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
    assert_extent(
        size.height,
        20.0 * 2.0 + spacing * 2.0,
        "a zero-size member still counts for spacing",
    );
    let placements = layout.place(bounds, ProposalSize::UNSPECIFIED, &children);
    assert_extent(
        placements[2].frame.y(),
        20.0 + spacing * 2.0,
        "the last member keeps both gaps around the zero-size one",
    );
}

/// A child that renders nothing while still declaring an explicit guide and
/// a stretch axis — the shape §4.4's membership rule must exclude from the
/// envelope, the exported guides, and the container's stretch, not only
/// from slots and spacing.
struct EmptyGuideLeaf {
    top: f32,
    stretch: StretchAxis,
}

impl SubView for EmptyGuideLeaf {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::zero()).with_vertical(VerticalAlignment::Top, self.top)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        true
    }
}

/// §4.4 — a non-member contributes to neither the alignment envelope, nor
/// the exported guides, nor the container's stretch. First leg is the
/// audit's fixture: an empty child whose explicit top guide sits a hundred
/// points above the stack leaves the row 40x10 with both members at y=0.
/// Second leg: a non-member claiming `Both` must not stretch its stack.
#[test]
fn a_non_member_exports_neither_guides_nor_stretch() {
    let rigid = || RangeLeaf {
        axis: Axis::Horizontal,
        minimum: 20.0,
        ideal: 20.0,
        maximum: 20.0,
        cross: 10.0,
    };
    let top_stack = || {
        Box::new(HStackLayout {
            alignment: VerticalAlignment::Top,
            spacing: Computed::constant(0.0),
        }) as Box<dyn Layout>
    };
    let outer = HStackLayout {
        alignment: VerticalAlignment::Top,
        spacing: Computed::constant(0.0),
    };

    // Guide leg: the empty child's -100 top guide must stay out of the inner
    // stack's exported guide and the outer envelope.
    let inner = LayoutNode {
        layout: top_stack(),
        children: vec![
            Box::new(rigid()),
            Box::new(EmptyGuideLeaf {
                top: -100.0,
                stretch: StretchAxis::None,
            }),
        ],
    };
    let sibling = rigid();
    let children: [&dyn SubView; 2] = [&inner, &sibling];
    let measured = measure_layout(&outer, ProposalSize::UNSPECIFIED, &children);
    assert_extent(measured.size.width, 40.0, "outer width is the two members");
    assert_extent(
        measured.size.height,
        10.0,
        "the non-member's guide cannot raise the envelope",
    );
    let placements = outer.place(
        Rect::from_size(measured.size),
        ProposalSize::UNSPECIFIED,
        &children,
    );
    assert_extent(placements[0].frame.y(), 0.0, "inner member on the line");
    assert_extent(placements[1].frame.y(), 0.0, "sibling member on the line");
    assert_extent(
        placements[1].frame.x(),
        20.0,
        "sibling follows the inner stack",
    );

    // Stretch leg: the non-member's `Both` must not make its stack greedy —
    // under a finite offer the outer row stays content-sized.
    let inner = LayoutNode {
        layout: top_stack(),
        children: vec![
            Box::new(rigid()),
            Box::new(EmptyGuideLeaf {
                top: 0.0,
                stretch: StretchAxis::Both,
            }),
        ],
    };
    let sibling = rigid();
    let children: [&dyn SubView; 2] = [&inner, &sibling];
    let measured = measure_layout(
        &outer,
        ProposalSize::new(Some(100.0), Some(60.0)),
        &children,
    );
    assert_extent(
        measured.size.width,
        40.0,
        "the non-member's claim cannot stretch the row",
    );
    assert_extent(
        measured.size.height,
        10.0,
        "height stays the members' answer",
    );
}

/// A leaf that cannot always fill the proposal it is compressed into:
/// finite offers come back `decline` short, the way a truncated text line
/// reports its drawn advance rather than the bound (§6). The unbounded
/// probes report its ideal, so the ∞-answer over-claims what a finite bound
/// can hold.
struct DecliningLeaf {
    axis: Axis,
    minimum: f32,
    ideal: f32,
    decline: f32,
    cross: f32,
    priority: i32,
}

impl SubView for DecliningLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(self.ideal, |value| {
            if value == 0.0 {
                self.minimum
            } else if value.is_infinite() {
                self.ideal
            } else {
                (value - self.decline).clamp(self.minimum, self.ideal)
            }
        });
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

/// The `Spacer` stand-in: `MainAxis` stretch, answers its `minimum` to every
/// measurement on that axis, and sits in the lowest band by default — so its
/// reported extent is `max(minimum, offer)` (§5).
struct FillLeaf {
    axis: Axis,
    minimum: f32,
    cross: f32,
    priority: i32,
}

impl SubView for FillLeaf {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(axis_size(self.axis, self.minimum, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::MainAxis
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

/// A leaf that accepts whatever it is offered between its minimum and its
/// maximum — the content the declined reserve should reach before the
/// lowest-band stretcher sees it.
struct AcceptingLeaf {
    axis: Axis,
    minimum: f32,
    maximum: f32,
    cross: f32,
    priority: i32,
}

impl SubView for AcceptingLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(self.maximum, |value| {
            value.clamp(self.minimum, self.maximum)
        });
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

/// A leaf that rounds a finite offer down to a `grain` boundary, up to its
/// `ideal` — the rounding behaviour a truncating text has, sharpened so a
/// one-point difference in the offer is visible in the answer.
struct RoundDownLeaf {
    axis: Axis,
    grain: f32,
    ideal: f32,
    cross: f32,
}

impl SubView for RoundDownLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(self.ideal, |value| {
            if value.is_infinite() {
                self.ideal
            } else {
                (self.grain * (value / self.grain).floor()).min(self.ideal)
            }
        });
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// A leaf with a plateau: it accepts the first ten points of any offer, then
/// resumes growing once the offer passes a hundred. Monotone, but its
/// maximum-probe answer does not reveal the plateau.
struct PlateauLeaf {
    axis: Axis,
    cross: f32,
}

impl SubView for PlateauLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(200.0, |value| {
            if value.is_infinite() {
                200.0
            } else if value <= 100.0 {
                value.min(10.0)
            } else {
                (value - 90.0).clamp(10.0, 200.0)
            }
        });
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// A leaf that answers more than its finite offer: ninety points whenever it
/// is proposed anything positive. A negotiation must not clamp the answer
/// back to the offer to hide the overflow.
struct OversizeLeaf {
    axis: Axis,
    answer: f32,
    cross: f32,
    priority: i32,
}

impl SubView for OversizeLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(self.answer, |value| {
            if value == 0.0 { 0.0 } else { self.answer }
        });
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

/// A leaf that declines exactly one ulp: it answers `f32`'s next
/// representable value below its finite offer. The declined sliver is
/// honest space and must reach the stretcher, not be rounded away.
struct NextDownLeaf {
    axis: Axis,
    ideal: f32,
    cross: f32,
}

impl SubView for NextDownLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(self.ideal, |value| {
            if value.is_infinite() {
                self.ideal
            } else if value <= 0.0 {
                0.0
            } else {
                f32::from_bits(value.to_bits() - 1)
            }
        });
        ViewDimensions::new(axis_size(self.axis, main, self.cross))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The stack's cross-alignment guide key for `axis`.
fn cross_guide(axis: Axis, dimensions: ViewDimensions, guide: f32) -> ViewDimensions {
    if axis.is_horizontal() {
        dimensions.with_vertical(VerticalAlignment::Top, guide)
    } else {
        dimensions.with_horizontal(HorizontalAlignment::Leading, guide)
    }
}

/// A `Spacer`-like child whose cross extent and guide depend on the main
/// proposal it was measured with: (30, 20) at main zero, (12, 9) at the six
/// the negotiation offers it. An envelope built from any earlier probe is
/// visibly wrong.
struct GuidedFillLeaf {
    axis: Axis,
}

impl SubView for GuidedFillLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let (cross, guide) = if main_proposal(self.axis, proposal) == Some(0.0) {
            (30.0, 20.0)
        } else {
            (12.0, 9.0)
        };
        cross_guide(
            self.axis,
            ViewDimensions::new(axis_size(self.axis, 0.0, cross)),
            guide,
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::MainAxis
    }

    fn priority(&self) -> i32 {
        i32::MIN
    }
}

/// A declining leaf with an explicit guide on the stack's cross alignment —
/// the "(10, 8)" siblings of the guided-envelope case.
struct DecliningGuidedLeaf {
    axis: Axis,
    minimum: f32,
    ideal: f32,
    decline: f32,
    cross: f32,
    guide: f32,
}

impl SubView for DecliningGuidedLeaf {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let main = main_proposal(self.axis, proposal).map_or(self.ideal, |value| {
            if value == 0.0 {
                self.minimum
            } else if value.is_infinite() {
                self.ideal
            } else {
                (value - self.decline).clamp(self.minimum, self.ideal)
            }
        });
        cross_guide(
            self.axis,
            ViewDimensions::new(axis_size(self.axis, main, self.cross)),
            self.guide,
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Measures the stack at `measure_main` on the main axis, then places it in
/// bounds of `place_main`, returning its answer and the placements in
/// logical member order.
fn place_after_proposal(
    axis: Axis,
    spacing: f32,
    measure_main: f32,
    place_main: f32,
    children: &[&dyn SubView],
) -> (Size, Vec<SubviewPlacement>) {
    let layout: Box<dyn Layout> = if axis.is_horizontal() {
        Box::new(HStackLayout {
            alignment: VerticalAlignment::Top,
            spacing: Computed::constant(spacing),
        })
    } else {
        Box::new(VStackLayout {
            alignment: HorizontalAlignment::Leading,
            spacing: Computed::constant(spacing),
        })
    };
    let proposal = axis_proposal(axis, Some(measure_main), Some(10.0));
    let size = layout.size_that_fits(proposal, children);
    let bounds = Rect::new(Point::new(0.0, 0.0), axis_size(axis, place_main, 10.0));
    let placements = layout.place(bounds, proposal, children);
    (size, placements)
}

/// Asserts every member's main origin, reported extent, and selected main
/// proposal, in logical order, across the negotiated `spacing`.
fn assert_negotiation(
    axis: Axis,
    placements: &[SubviewPlacement],
    spacing: f32,
    expected: &[(f32, f32)],
) {
    assert_eq!(placements.len(), expected.len(), "one placement per member");
    let mut cursor = 0.0;
    for (placement, &(extent, proposal)) in placements.iter().zip(expected) {
        assert_extent(
            main_origin(axis, placement.frame),
            cursor,
            "member main origin",
        );
        assert_extent(
            main_extent(axis, *placement.frame.size()),
            extent,
            "member reported extent",
        );
        assert_extent(
            main_proposal(axis, placement.proposal).expect("a selected main proposal"),
            proposal,
            "member selected proposal",
        );
        cursor += extent + spacing;
    }
}

/// A compressed leaf that reports less than its offer leaves surplus inside
/// the stack's bounds; the sequential negotiation offers the spacer the six
/// points the text declined and the trailing badge still ends on the edge
/// (water-rs/waterui#1219).
#[test]
fn a_declined_extent_reaches_the_stretch_child_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 6.0,
                cross: 10.0,
                priority: 0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_extent(main_extent(axis, size), 100.0, "stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(74.0, 80.0), (6.0, 6.0), (20.0, 20.0)],
        );
    }
}

/// §9 (a): several stretchers divide a declined surplus by the same
/// minimum-reserving rule — equal shares in logical order.
#[test]
fn several_stretchers_share_the_decline_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 12.0,
                cross: 10.0,
                priority: 0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 120.0, 120.0, &refs);
        assert_extent(main_extent(axis, size), 120.0, "stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(88.0, 100.0), (6.0, 6.0), (6.0, 6.0), (20.0, 20.0)],
        );
    }
}

/// §9 (b): a stretcher's own minimum is reserved like any other — the S40
/// spacer negotiates its forty before the unreserved share is offered.
#[test]
fn a_stretchers_minimum_is_reserved_before_peer_shares_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 12.0,
                cross: 10.0,
                priority: 0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 40.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 120.0, 120.0, &refs);
        assert_extent(main_extent(axis, size), 120.0, "stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(48.0, 60.0), (40.0, 40.0), (12.0, 12.0), (20.0, 20.0)],
        );
    }
}

/// §9 (c): bands negotiate in priority order — the priority-1 accepter is
/// offered the twenty the priority-2 decliner left before the lowest-band
/// spacer sees it.
#[test]
fn higher_bands_negotiate_before_the_spacer_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 20.0,
                cross: 10.0,
                priority: 2,
            }),
            Box::new(AcceptingLeaf {
                axis,
                minimum: 0.0,
                maximum: 200.0,
                cross: 10.0,
                priority: 1,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_extent(main_extent(axis, size), 100.0, "stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(60.0, 80.0), (20.0, 20.0), (0.0, 0.0), (20.0, 20.0)],
        );
    }
}

/// §9's target-cap case: the accepting child's own maximum limits what the
/// decline restores to it — the eight points it cannot use go to the spacer.
#[test]
fn a_declined_offer_respects_the_lower_bands_target_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 20.0,
                cross: 10.0,
                priority: 2,
            }),
            Box::new(AcceptingLeaf {
                axis,
                minimum: 0.0,
                maximum: 12.0,
                cross: 10.0,
                priority: 1,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_extent(main_extent(axis, size), 100.0, "stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(60.0, 80.0), (12.0, 12.0), (8.0, 8.0), (20.0, 20.0)],
        );
    }
}

/// §9's unequal-stretcher case: stretchers in different bands negotiate in
/// priority order, so the higher spacer takes the whole decline.
#[test]
fn stretchers_negotiate_in_priority_order_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 12.0,
                cross: 10.0,
                priority: 0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: -1,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 120.0, 120.0, &refs);
        assert_extent(main_extent(axis, size), 120.0, "stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(88.0, 100.0), (12.0, 12.0), (0.0, 0.0), (20.0, 20.0)],
        );
    }
}

/// §9 (d): equal flexibility is resolved by logical member order — the
/// first decliner negotiates before the reserve grows, so sibling order
/// changes the outcome and identical leaves are not equalized.
#[test]
fn decliners_negotiate_in_logical_order_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for (declines, expected) in [
            (
                [6.0, 10.0],
                [(44.0, 50.0), (46.0, 56.0), (10.0, 10.0), (20.0, 20.0)],
            ),
            (
                [10.0, 6.0],
                [(40.0, 50.0), (54.0, 60.0), (6.0, 6.0), (20.0, 20.0)],
            ),
            (
                [6.0, 6.0],
                [(44.0, 50.0), (50.0, 56.0), (6.0, 6.0), (20.0, 20.0)],
            ),
        ] {
            let children: Vec<Box<dyn SubView>> = declines
                .iter()
                .map(|&decline| {
                    Box::new(DecliningLeaf {
                        axis,
                        minimum: 0.0,
                        ideal: 200.0,
                        decline,
                        cross: 10.0,
                        priority: 0,
                    }) as Box<dyn SubView>
                })
                .chain([
                    Box::new(FillLeaf {
                        axis,
                        minimum: 0.0,
                        cross: 0.0,
                        priority: i32::MIN,
                    }) as Box<dyn SubView>,
                    Box::new(RangeLeaf {
                        axis,
                        minimum: 20.0,
                        ideal: 20.0,
                        maximum: 20.0,
                        cross: 10.0,
                    }),
                ])
                .collect();
            let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
            let (size, placements) = place_after_proposal(axis, 0.0, 120.0, 120.0, &refs);
            assert_extent(main_extent(axis, size), 120.0, "stack main extent");
            assert_negotiation(axis, &placements, 0.0, &expected);
        }
    }
}

/// §9 (e): a nested stack's declined space stays inside its own frame —
/// placing the inner row at its narrower resolved bound negotiates afresh
/// and the leaf declines again from the new offer.
#[test]
fn declined_space_inside_a_nested_stack_stays_nested_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let leaf: Box<dyn SubView> = Box::new(DecliningLeaf {
            axis,
            minimum: 0.0,
            ideal: 200.0,
            decline: 6.0,
            cross: 10.0,
            priority: 0,
        });
        let inner = LayoutNode {
            layout: stack(axis),
            children: vec![leaf],
        };
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(inner),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_extent(main_extent(axis, size), 100.0, "outer stack main extent");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(74.0, 80.0), (6.0, 6.0), (20.0, 20.0)],
        );

        // Inside the 74-point frame the inner negotiation offers the leaf
        // 74, not the 80 the outer negotiation selected — and the leaf
        // answers 68.
        let inner_leaf = DecliningLeaf {
            axis,
            minimum: 0.0,
            ideal: 200.0,
            decline: 6.0,
            cross: 10.0,
            priority: 0,
        };
        let inner_stack = stack(axis);
        let inner_proposal = axis_proposal(axis, Some(74.0), Some(10.0));
        let inner_bounds = Rect::new(Point::new(0.0, 0.0), axis_size(axis, 74.0, 10.0));
        let inner_placements = inner_stack.place(inner_bounds, inner_proposal, &[&inner_leaf]);
        assert_extent(
            main_proposal(axis, inner_placements[0].proposal)
                .expect("the inner leaf is proposed the resolved bound"),
            74.0,
            "inner selected proposal",
        );
        assert_extent(
            main_extent(axis, *inner_placements[0].frame.size()),
            68.0,
            "the inner leaf declines inside the narrower frame",
        );
    }
}

/// §9 (g): with no stretcher the declined extent stays unused — the stack's
/// own answer shrinks to the reported extents, and placement into that
/// answer negotiates the smaller bound again.
#[test]
fn declined_space_without_a_stretcher_stays_unused_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(RoundDownLeaf {
                axis,
                grain: 10.0,
                ideal: 200.0,
                cross: 10.0,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 103.0, 103.0, &refs);
        assert_extent(
            main_extent(axis, size),
            100.0,
            "the stack answers the reported extents",
        );
        assert_negotiation(axis, &placements, 0.0, &[(80.0, 83.0), (20.0, 20.0)]);
        let (_, own_placements) = place_after_proposal(axis, 0.0, 103.0, 100.0, &refs);
        assert_negotiation(axis, &own_placements, 0.0, &[(80.0, 80.0), (20.0, 20.0)]);
    }
}

/// §9's larger-bounds case: placement negotiates afresh against the resolved
/// bounds, so a row placed wider than it was measured offers the decliner
/// the new extent.
#[test]
fn placement_renegotiates_against_larger_bounds_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 6.0,
                cross: 10.0,
                priority: 0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 120.0, &refs);
        assert_extent(main_extent(axis, size), 100.0, "measured at the proposal");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(94.0, 100.0), (6.0, 6.0), (20.0, 20.0)],
        );
    }
}

/// §9's spacing case: member spacing is subtracted before any offer, so the
/// negotiation runs on what remains and the badge ends on the edge.
#[test]
fn spacing_is_reserved_before_negotiation_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 6.0,
                cross: 10.0,
                priority: 0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 4.0, 108.0, 108.0, &refs);
        assert_extent(main_extent(axis, size), 108.0, "stack extent incl. spacing");
        assert_negotiation(
            axis,
            &placements,
            4.0,
            &[(74.0, 80.0), (6.0, 6.0), (20.0, 20.0)],
        );
        assert_extent(
            main_origin(axis, placements[2].frame) + main_extent(axis, *placements[2].frame.size()),
            108.0,
            "the badge ends on the trailing edge",
        );
    }
}

/// §9's infeasible case: when the minima alone overflow the budget every
/// child is offered its minimum and the row overflows rather than
/// collapsing one below it.
#[test]
fn minima_that_overflow_the_budget_are_offered_verbatim_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(FillLeaf {
                axis,
                minimum: 90.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_extent(main_extent(axis, size), 110.0, "the overflow is reported");
        assert_negotiation(axis, &placements, 0.0, &[(90.0, 90.0), (20.0, 20.0)]);
    }
}

/// A decliner that rounds to a ten-point boundary still leaves an offer it
/// can report exactly: §9's rounding case keeps the three-point remainder
/// honest for the spacer.
#[test]
fn a_rounded_decline_leaves_the_exact_remainder_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(RoundDownLeaf {
                axis,
                grain: 10.0,
                ideal: 200.0,
                cross: 10.0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 103.0, 103.0, &refs);
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(80.0, 83.0), (3.0, 3.0), (20.0, 20.0)],
        );
    }
}

/// §9's sub-ulp case: a one-ulp decline is honest space, not rounding noise —
/// the decliner keeps its actual answer and the spacer receives the exact
/// remainder rather than the frame being snapped back to the offer.
#[test]
fn a_one_ulp_decline_is_honest_space_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(NextDownLeaf {
                axis,
                ideal: 200.0,
                cross: 10.0,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_eq!(
            main_extent(axis, *placements[0].frame.size()),
            f32::from_bits(80.0_f32.to_bits() - 1),
            "the decliner keeps its one-ulp answer",
        );
        assert_eq!(
            main_proposal(axis, placements[0].proposal),
            Some(80.0),
            "the decliner was offered 80",
        );
        assert_eq!(
            main_extent(axis, *placements[1].frame.size()),
            1.0_f32 / 131_072.0,
            "the spacer receives the exact ulp remainder",
        );
        assert_eq!(
            main_extent(axis, *placements[2].frame.size()),
            20.0,
            "the badge is untouched",
        );
    }
}

/// §9's resize pin: a decliner beside a plateau leaf records the sequential
/// policy — growing the bounds by two must offer the second child the two
/// new points, never shrink the first.
#[test]
fn growing_the_bounds_cannot_shrink_a_decliner_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(RoundDownLeaf {
                axis,
                grain: 10.0,
                ideal: 200.0,
                cross: 10.0,
            }),
            Box::new(PlateauLeaf { axis, cross: 10.0 }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, at_120) = place_after_proposal(axis, 0.0, 120.0, 120.0, &refs);
        assert_negotiation(
            axis,
            &at_120,
            0.0,
            &[(50.0, 50.0), (10.0, 50.0), (40.0, 40.0), (20.0, 20.0)],
        );
        let (_, at_122) = place_after_proposal(axis, 0.0, 122.0, 122.0, &refs);
        assert_negotiation(
            axis,
            &at_122,
            0.0,
            &[(50.0, 51.0), (10.0, 52.0), (42.0, 42.0), (20.0, 20.0)],
        );
    }
}

/// §9's oversized-answer case: a child that answers above its offer
/// overflows the row on its own — the allocator does not repair the refusal
/// by compressing the badge below its minimum or starving the spacer.
#[test]
fn an_oversized_answer_overflows_without_stealing_minima_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(OversizeLeaf {
                axis,
                answer: 90.0,
                cross: 10.0,
                priority: 1,
            }),
            Box::new(FillLeaf {
                axis,
                minimum: 0.0,
                cross: 0.0,
                priority: i32::MIN,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                maximum: 20.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (size, placements) = place_after_proposal(axis, 0.0, 100.0, 100.0, &refs);
        assert_extent(main_extent(axis, size), 110.0, "the refusal overflows");
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(90.0, 80.0), (0.0, 0.0), (20.0, 20.0)],
        );
    }
}

/// §9's cross-axis case: the alignment envelope and every guide come from
/// the measurements the negotiation selected — not from a probe that ran
/// before the offers were known.
#[test]
fn the_envelope_comes_from_the_selected_measurement_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningGuidedLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 6.0,
                cross: 10.0,
                guide: 8.0,
            }),
            Box::new(GuidedFillLeaf { axis }),
            Box::new(DecliningGuidedLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                decline: 0.0,
                cross: 10.0,
                guide: 8.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let layout = stack(axis);
        let proposal = axis_proposal(axis, Some(100.0), Some(10.0));
        let size = layout.size_that_fits(proposal, &refs);
        // The spacer's selected measurement answered cross extent 12, guide
        // 9; the siblings answer (10, 8). The envelope is 9 + 3 = 12, and a
        // sibling sits one point in from its own guide.
        assert_extent(
            cross_extent(axis, size),
            12.0,
            "the envelope is the selected guides' envelope",
        );
        let bounds = Rect::new(Point::new(0.0, 0.0), axis_size(axis, 100.0, 12.0));
        let placements = layout.place(bounds, proposal, &refs);
        for placement in [&placements[0], &placements[2]] {
            assert_extent(
                cross_origin(axis, placement.frame),
                1.0,
                "the sibling sits at line - guide",
            );
        }
        assert_extent(
            cross_origin(axis, placements[1].frame),
            0.0,
            "the spacer's own guide aligns it to the envelope edge",
        );
    }
}

/// Placement is a fresh negotiation against the same resolved bounds:
/// probes between two placements — at any proposal — change neither the
/// frames nor the selected proposals.
#[test]
fn interleaved_probes_do_not_change_a_repeated_placement_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(DecliningGuidedLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                decline: 6.0,
                cross: 10.0,
                guide: 8.0,
            }),
            Box::new(GuidedFillLeaf { axis }),
            Box::new(DecliningGuidedLeaf {
                axis,
                minimum: 20.0,
                ideal: 20.0,
                decline: 0.0,
                cross: 10.0,
                guide: 8.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let layout = stack(axis);
        let proposal = axis_proposal(axis, Some(100.0), Some(10.0));
        let bounds = Rect::new(Point::new(0.0, 0.0), axis_size(axis, 100.0, 12.0));
        let first = layout.place(bounds, proposal, &refs);
        for probe in [
            Some(0.0),
            Some(60.0),
            Some(200.0),
            None,
            Some(f32::INFINITY),
        ] {
            layout.size_that_fits(axis_proposal(axis, probe, Some(10.0)), &refs);
        }
        let second = layout.place(bounds, proposal, &refs);
        assert_eq!(first, second, "a repeated placement is identical");
    }
}

/// Equal-flexibility children divide a band's budget equally: seven equal
/// columns 56 pt short all lose the same 8 pt — the rule that replaced
/// water-filling keeps its calendar-row property.
#[test]
fn equal_children_share_the_band_budget_equally_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = (0..7)
            .map(|_| {
                Box::new(RangeLeaf {
                    axis,
                    minimum: 0.0,
                    ideal: 40.0,
                    maximum: 40.0,
                    cross: 10.0,
                }) as Box<dyn SubView>
            })
            .collect();
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 224.0, 224.0, &refs);
        assert_negotiation(axis, &placements, 0.0, &[(32.0, 32.0); 7]);
    }
}

/// A rigid child beside a wide flexible one: the wide child negotiates
/// after the rigid peer's share is reserved and absorbs the deficit alone.
#[test]
fn the_widest_child_absorbs_the_deficit_alone_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(RangeLeaf {
                axis,
                minimum: 0.0,
                ideal: 50.0,
                maximum: 50.0,
                cross: 10.0,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 0.0,
                ideal: 200.0,
                maximum: 200.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 140.0, 140.0, &refs);
        assert_negotiation(axis, &placements, 0.0, &[(50.0, 50.0), (90.0, 90.0)]);
    }
}

/// The flexible child in the middle of rigid neighbours receives exactly
/// the remainder the peer-minimum reservations leave it.
#[test]
fn one_flexible_child_receives_the_exact_remainder_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(RangeLeaf {
                axis,
                minimum: 50.0,
                ideal: 50.0,
                maximum: 50.0,
                cross: 10.0,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 0.0,
                ideal: 280.0,
                maximum: 280.0,
                cross: 10.0,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 80.0,
                ideal: 80.0,
                maximum: 80.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 280.0, 280.0, &refs);
        assert_negotiation(
            axis,
            &placements,
            0.0,
            &[(50.0, 50.0), (150.0, 150.0), (80.0, 80.0)],
        );
    }
}

/// A child's measured minimum is a hard floor: the negotiation cannot push
/// the 80-point child below it, so the first child takes the whole deficit
/// within its own floor.
#[test]
fn a_child_never_negotiates_below_its_reported_minimum_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(RangeLeaf {
                axis,
                minimum: 20.0,
                ideal: 100.0,
                maximum: 100.0,
                cross: 10.0,
            }),
            Box::new(RangeLeaf {
                axis,
                minimum: 80.0,
                ideal: 100.0,
                maximum: 100.0,
                cross: 10.0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 120.0, 120.0, &refs);
        assert_negotiation(axis, &placements, 0.0, &[(40.0, 40.0), (80.0, 80.0)]);
    }
}

/// A lower band's whole ideal is given up before the higher band's offer is
/// reduced: the priority-1 child is proposed its target and the band-0
/// child absorbs the deficit.
#[test]
fn the_lower_band_gives_way_before_the_higher_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let children: Vec<Box<dyn SubView>> = vec![
            Box::new(AcceptingLeaf {
                axis,
                minimum: 0.0,
                maximum: 100.0,
                cross: 10.0,
                priority: 1,
            }),
            Box::new(AcceptingLeaf {
                axis,
                minimum: 0.0,
                maximum: 100.0,
                cross: 10.0,
                priority: 0,
            }),
        ];
        let refs: Vec<&dyn SubView> = children.iter().map(AsRef::as_ref).collect();
        let (_, placements) = place_after_proposal(axis, 0.0, 150.0, 150.0, &refs);
        assert_negotiation(axis, &placements, 0.0, &[(100.0, 100.0), (50.0, 50.0)]);
    }
}
