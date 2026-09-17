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
        if main.is_none() {
            assert_eq!(main_proposal(axis, placement.proposal), None);
        }
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
        for (main, expected) in [
            (None, [40.0, 120.0]),
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

#[test]
fn equal_bounds_preserve_distinct_selected_proposals_after_other_probes() {
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
            for (proposal, expected) in [(unspecified, [40.0, 120.0]), (bounded, [80.0, 80.0])] {
                let placements = layout.place(bounds, proposal, &refs);
                assert_eq!(placements.len(), 2);
                for (placement, expected) in placements.iter().zip(expected) {
                    assert_extent(
                        main_extent(axis, *placement.frame.size()),
                        expected,
                        "probe order",
                    );
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
        assert_eq!(placements[0].proposal, proposal);
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
            main_proposal(axis, proposal),
            "the main axis keeps the measurement proposal"
        );
    }
}

const fn cross_extent(axis: Axis, size: Size) -> f32 {
    if axis.is_horizontal() {
        size.height
    } else {
        size.width
    }
}
