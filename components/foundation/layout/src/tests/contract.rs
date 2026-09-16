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
