//! [`ScrollView`]: a clipped viewport over content measured without a
//! constraint on the scroll axis.
//!
//! Dew has no interaction runtime yet, so content renders at offset zero;
//! the viewport clip is retained per command, keeping overflow invisible
//! while the flat display list stays diffable for partial flushes.

use std::cell::RefCell;

use waterui_core::layout::{Point, ProposalSize, Rect as LayoutRect, Size, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_layout::scroll::{Axis, ScrollView};

use crate::dispatch::{DewRenderer, RenderContext, measure_view};
use crate::text::DewState;
use crate::views::to_f32;

/// Renders the content at offset zero, clipped to the viewport.
pub fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<ScrollView>,
    env: &Environment,
) {
    let (axis, content) = view.into_inner().into_inner();
    let viewport = ctx.bounds;
    let proposal = ProposalSize::new(
        Some(to_f32(viewport.width())),
        Some(to_f32(viewport.height())),
    );
    let intrinsic = measure_view(
        renderer.state_cell(),
        &content,
        env,
        content_proposal(axis, proposal),
    )
    .size;
    let (content_width, content_height) = content_size(axis, viewport, intrinsic);

    let clip = ctx.transform.transform_rect_bbox(viewport);
    renderer.list_mut().push_clip(clip);
    let frame = LayoutRect::new(
        Point::new(0.0, 0.0),
        Size::new(content_width, content_height),
    );
    renderer.dispatch_child(content, env, ctx.child(frame));
    renderer.list_mut().pop_clip();
}

/// Stretches to the proposal on both axes; the content's intrinsic size
/// (measured unconstrained along the scroll axis) backstops unspecified
/// proposals.
pub fn measure(
    state: &RefCell<DewState>,
    scroll: &ScrollView,
    env: &Environment,
    proposal: ProposalSize,
) -> ViewDimensions {
    let (axis, content) = scroll.as_parts();
    let intrinsic = measure_view(state, content, env, content_proposal(axis, proposal)).size;
    ViewDimensions::new(Size::new(
        proposal.width.unwrap_or(intrinsic.width),
        proposal.height.unwrap_or(intrinsic.height),
    ))
}

/// The content proposal: the scroll axis is unconstrained, the cross axis
/// keeps the viewport constraint.
fn content_proposal(axis: Axis, proposal: ProposalSize) -> ProposalSize {
    match axis {
        Axis::Horizontal => ProposalSize::new(None, proposal.height),
        Axis::Vertical => ProposalSize::new(proposal.width, None),
        Axis::All => ProposalSize::UNSPECIFIED,
        _ => panic!("dew does not support scroll axis {axis:?}"),
    }
}

/// The placed content size: the viewport on the cross axis, the intrinsic
/// extent (at least the viewport) on the scroll axis.
fn content_size(axis: Axis, viewport: kurbo::Rect, intrinsic: Size) -> (f32, f32) {
    let viewport_width = to_f32(viewport.width());
    let viewport_height = to_f32(viewport.height());
    match axis {
        Axis::Horizontal => (intrinsic.width.max(viewport_width), viewport_height),
        Axis::Vertical => (viewport_width, intrinsic.height.max(viewport_height)),
        Axis::All => (
            intrinsic.width.max(viewport_width),
            intrinsic.height.max(viewport_height),
        ),
        _ => panic!("dew does not support scroll axis {axis:?}"),
    }
}
