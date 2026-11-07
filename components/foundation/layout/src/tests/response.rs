//! Response-geometry contract (`docs/layout-spec.md` §2.4, §4, §5.1): where
//! each container positions its own answer inside the bounds it is assigned,
//! independently of where it places its children.

use alloc::{vec, vec::Vec};
use core::num::NonZeroUsize;
use nami::Computed;

use super::MinLengthSpacerView;
use crate::padding::{EdgeInsets, PaddingLayout};
use crate::stack::{
    Alignment, HStackLayout, HorizontalAlignment, VStackLayout, VerticalAlignment, ZStackLayout,
};
use crate::{
    AbsoluteLayout, BackgroundLayout, Layout, LayoutError, OverlayLayout, Point, ProposalSize,
    Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions, grid::GridLayout,
    place_layout, resolve_layout_extent,
};

/// A rigid leaf: one answer whatever it is proposed.
struct Leaf(Size);

impl SubView for Leaf {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(self.0)
    }
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A leaf whose answer depends on its width proposal and is not idempotent
/// under reproposal: 84 → 20 wide, 184 → 40 wide.
struct Reflowing;

impl SubView for Reflowing {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let width = match proposal.width {
            Some(width) if width >= 184.0 => 40.0,
            _ => 20.0,
        };
        ViewDimensions::new(Size::new(width, 10.0))
    }
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
    fn priority(&self) -> i32 {
        0
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(width, height))
}

fn response(layout: &dyn Layout, bounds: Rect, children: &[&dyn SubView]) -> Rect {
    resolve_layout_extent(layout, bounds, ProposalSize::UNSPECIFIED, children)
        .expect("valid geometry")
        .rect
}

fn padding(top: f32, bottom: f32, leading: f32, trailing: f32) -> PaddingLayout {
    PaddingLayout {
        edges: Computed::constant(EdgeInsets::new(top, bottom, leading, trailing)),
    }
}

#[test]
fn a_stack_centres_its_response_on_the_main_axis_and_aligns_it_on_the_cross_axis() {
    let a = Leaf(Size::new(30.0, 10.0));
    let b = Leaf(Size::new(50.0, 20.0));
    let children: [&dyn SubView; 2] = [&a, &b];
    let bounds = rect(0.0, 0.0, 200.0, 100.0);
    // Answer: 30 + 10 + 50 = 90 wide, 20 tall.
    for (alignment, y) in [
        (VerticalAlignment::Top, 0.0),
        (VerticalAlignment::Center, 40.0),
        (VerticalAlignment::Bottom, 80.0),
        (VerticalAlignment::FirstBaseline, 0.0),
    ] {
        let layout = HStackLayout {
            alignment,
            spacing: Computed::constant(10.0),
        };
        assert_eq!(
            response(&layout, bounds, &children),
            rect(55.0, y, 90.0, 20.0),
            "{alignment:?}"
        );
    }
    for (alignment, x) in [
        (HorizontalAlignment::Leading, 0.0),
        (HorizontalAlignment::Center, 75.0),
        (HorizontalAlignment::Trailing, 150.0),
    ] {
        let layout = VStackLayout {
            alignment,
            spacing: Computed::constant(10.0),
        };
        // Answer: 50 wide, 10 + 10 + 20 = 40 tall.
        assert_eq!(
            response(&layout, bounds, &children),
            rect(x, 30.0, 50.0, 40.0),
            "{alignment:?}"
        );
    }
}

#[test]
fn an_underfilled_stack_centres_its_negotiated_block() {
    let a = Leaf(Size::new(30.0, 10.0));
    let b = Leaf(Size::new(50.0, 10.0));
    let children: [&dyn SubView; 2] = [&a, &b];
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: Computed::constant(10.0),
    };
    let bounds = rect(10.0, 0.0, 200.0, 10.0);
    let placement = place_layout(&layout, bounds, ProposalSize::UNSPECIFIED, &children).unwrap();
    // The 90-wide block starts at 10 + (200 - 90) / 2 = 65.
    assert_eq!(placement.children[0].frame, rect(65.0, 0.0, 30.0, 10.0));
    assert_eq!(placement.children[1].frame, rect(105.0, 0.0, 50.0, 10.0));
    assert_eq!(placement.response_rect, rect(65.0, 0.0, 90.0, 10.0));
}

#[test]
fn an_overflowing_stack_block_crosses_both_edges_equally() {
    let a = Leaf(Size::new(60.0, 10.0));
    let b = Leaf(Size::new(60.0, 10.0));
    let children: [&dyn SubView; 2] = [&a, &b];
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: Computed::constant(0.0),
    };
    let placements = layout.place(
        rect(0.0, 0.0, 60.0, 10.0),
        ProposalSize::UNSPECIFIED,
        &children,
    );
    assert_eq!(placements[0].frame.y(), -5.0);
    assert_eq!(placements[1].frame.y(), 5.0);
}

#[test]
fn a_zstack_aligns_its_response_and_ignores_overflowing_layers() {
    let small = Leaf(Size::new(20.0, 20.0));
    let wide = Leaf(Size::new(40.0, 10.0));
    let children: [&dyn SubView; 2] = [&small, &wide];
    let bounds = rect(0.0, 0.0, 100.0, 100.0);
    for (alignment, origin) in [
        (Alignment::TopLeading, Point::new(0.0, 0.0)),
        (Alignment::Center, Point::new(30.0, 40.0)),
        (Alignment::BottomTrailing, Point::new(60.0, 80.0)),
    ] {
        let layout = ZStackLayout { alignment };
        assert_eq!(
            response(&layout, bounds, &children),
            Rect::new(origin, Size::new(40.0, 20.0)),
            "{alignment:?}"
        );
    }
}

#[test]
fn an_overlays_response_is_its_bases_and_decorations_contribute_nothing() {
    let base = Leaf(Size::new(20.0, 10.0));
    let badge = Leaf(Size::new(80.0, 80.0));
    let layout = OverlayLayout::default().alignment(Alignment::TopTrailing);
    let bounds = rect(0.0, 0.0, 100.0, 50.0);
    let with_badge: [&dyn SubView; 2] = [&base, &badge];
    let alone: [&dyn SubView; 1] = [&base];
    let expected = rect(40.0, 20.0, 20.0, 10.0);
    assert_eq!(response(&layout, bounds, &with_badge), expected);
    assert_eq!(response(&layout, bounds, &alone), expected);
}

#[test]
fn adding_a_background_never_moves_the_response() {
    let background = Leaf(Size::new(100.0, 100.0));
    let content = Leaf(Size::new(20.0, 10.0));
    let children: [&dyn SubView; 2] = [&background, &content];
    let bounds = rect(0.0, 0.0, 100.0, 50.0);
    let content_alone = content
        .resolve_extent(bounds, ProposalSize::UNSPECIFIED)
        .unwrap()
        .rect;
    assert_eq!(
        response(&BackgroundLayout, bounds, &children),
        content_alone
    );
}

#[test]
fn a_leading_only_padding_belongs_wholly_to_the_leading_side() {
    // A 10-wide leaf behind 20 points of leading padding, alone in a 160-wide
    // slot: the child's response is centred in the inset region (centre 90)
    // and the padded response is centred in the slot (centre 80).
    let child = Leaf(Size::new(10.0, 10.0));
    let children: [&dyn SubView; 1] = [&child];
    let layout = padding(0.0, 0.0, 20.0, 0.0);
    let bounds = rect(0.0, 0.0, 160.0, 10.0);
    let padded = response(&layout, bounds, &children);
    assert_eq!(padded, rect(65.0, 0.0, 30.0, 10.0));
    assert_eq!(padded.mid_x(), 80.0);
    let placed = layout.place(bounds, ProposalSize::UNSPECIFIED, &children)[0].frame;
    let child_response = child
        .resolve_extent(placed, ProposalSize::UNSPECIFIED)
        .unwrap()
        .rect;
    assert_eq!(child_response.mid_x(), 90.0);
}

#[test]
fn a_padding_that_exactly_fills_its_allocation_responds_at_the_allocation() {
    let child = Leaf(Size::new(10.0, 10.0));
    let children: [&dyn SubView; 1] = [&child];
    let layout = padding(0.0, 0.0, 20.0, 0.0);
    let bounds = rect(0.0, 0.0, 30.0, 10.0);
    assert_eq!(response(&layout, bounds, &children), bounds);
}

#[test]
fn padding_responds_at_its_answer_not_at_the_padded_placement() {
    // Proposed 100 but placed in 200: the answer is 20 + 8 + 8 = 36, while
    // the child, placed under the bounds-derived 184, lays out 40 wide.
    let children: [&dyn SubView; 1] = [&Reflowing];
    let layout = padding(0.0, 0.0, 8.0, 8.0);
    let bounds = rect(0.0, 0.0, 200.0, 10.0);
    let proposal = ProposalSize::new(Some(100.0), Some(10.0));
    let resolved = resolve_layout_extent(&layout, bounds, proposal, &children).unwrap();
    assert_eq!(resolved.answer, Size::new(36.0, 10.0));
    assert_eq!(resolved.rect, rect(82.0, 0.0, 36.0, 10.0));
    let placed = layout.place(bounds, proposal, &children);
    assert_eq!(
        Reflowing.measure(placed[0].proposal).size.width,
        40.0,
        "the child is still placed under the bounds-derived proposal"
    );
}

#[test]
fn negative_padding_keeps_its_origin_and_clamps_only_the_extent() {
    let child = Leaf(Size::new(10.0, 10.0));
    let children: [&dyn SubView; 1] = [&child];
    let layout = padding(0.0, 0.0, -20.0, 0.0);
    let bounds = rect(0.0, 0.0, 100.0, 10.0);
    let resolved =
        resolve_layout_extent(&layout, bounds, ProposalSize::UNSPECIFIED, &children).unwrap();
    // The child sits at the centre of the region inset by -20 (x = -20,
    // width 120): 35..45. The padded answer collapses to zero width at the
    // child's origin less the -20 inset.
    assert_eq!(resolved.rect, rect(55.0, 0.0, 0.0, 10.0));
}

#[test]
fn a_grid_block_is_anchored_top_leading_whatever_its_alignment() {
    let a = Leaf(Size::new(10.0, 10.0));
    let b = Leaf(Size::new(10.0, 10.0));
    let children: [&dyn SubView; 2] = [&a, &b];
    let layout = GridLayout::new(
        NonZeroUsize::new(2).unwrap(),
        Size::new(4.0, 4.0),
        Alignment::BottomTrailing,
    );
    let bounds = rect(5.0, 5.0, 100.0, 100.0);
    assert_eq!(
        response(&layout, bounds, &children),
        rect(5.0, 5.0, 24.0, 10.0)
    );
}

#[test]
fn an_absolute_canvas_responds_at_its_origin_not_at_its_children() {
    let child = Leaf(Size::new(10.0, 10.0));
    let children: [&dyn SubView; 1] = [&child];
    let bounds = rect(3.0, 4.0, 100.0, 50.0);
    let proposal = ProposalSize::new(Some(60.0), Some(30.0));
    assert_eq!(
        resolve_layout_extent(&AbsoluteLayout, bounds, proposal, &children)
            .unwrap()
            .rect,
        rect(3.0, 4.0, 60.0, 30.0)
    );
}

#[test]
fn a_promoted_spacer_keeps_its_zero_response() {
    let spacer_view = MinLengthSpacerView {
        min_length: 0.0,
        axis: StretchAxis::Horizontal,
    };
    let leaf = Leaf(Size::new(20.0, 10.0));
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: Computed::constant(0.0),
    };
    let children: [&dyn SubView; 2] = [&spacer_view, &leaf];
    let bounds = rect(0.0, 0.0, 100.0, 10.0);
    let placements = layout.place(bounds, ProposalSize::UNSPECIFIED, &children);
    assert_eq!(
        placements[0].frame.width(),
        80.0,
        "the stack allocates the gap"
    );
    let spacer_response = spacer_view
        .resolve_extent(placements[0].frame, placements[0].proposal)
        .unwrap();
    assert_eq!(spacer_response.rect.width(), 0.0);
    assert_eq!(spacer_response.rect.mid_x(), 40.0);
}

#[test]
fn every_container_response_is_translation_covariant() {
    let a = Leaf(Size::new(30.0, 10.0));
    let b = Leaf(Size::new(50.0, 20.0));
    let pair: [&dyn SubView; 2] = [&a, &b];
    let single: [&dyn SubView; 1] = [&a];
    let hstack = HStackLayout::default();
    let zstack = ZStackLayout {
        alignment: Alignment::BottomLeading,
    };
    let overlay = OverlayLayout::default();
    let pad = padding(3.0, 1.0, 7.0, 2.0);
    let cases: Vec<(&dyn Layout, &[&dyn SubView])> = vec![
        (&hstack, &pair),
        (&zstack, &pair),
        (&overlay, &pair),
        (&pad, &single),
        (&BackgroundLayout, &pair),
    ];
    let bounds = rect(0.0, 0.0, 200.0, 100.0);
    for (layout, children) in cases {
        let here = response(layout, bounds, children);
        let there = response(layout, bounds.translated(-17.0, 23.0), children);
        assert_eq!(there, here.translated(-17.0, 23.0), "{layout:?}");
    }
}
