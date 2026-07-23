//! Persistent [`ScrollView`] node.

use core::cell::RefCell;

use waterui_core::Environment;
use waterui_core::layout::{
    Point, ProposalSize, Rect as LayoutRect, Size, StretchAxis, ViewDimensions,
};
use waterui_layout::scroll::{Axis, ScrollView};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, build_node};
use crate::text::DewState;
use crate::views::to_f32;

struct ScrollNode {
    axis: Axis,
    child: Box<dyn DewNode>,
}

pub fn build(
    renderer: &mut DewRenderer,
    scroll: ScrollView,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let (axis, content) = scroll.into_inner();
    Box::new(ScrollNode {
        axis,
        child: build_node(renderer, content, env, depth),
    })
}

impl DewNode for ScrollNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let intrinsic = self
            .child
            .measure(state, content_proposal(self.axis, proposal))
            .size;
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(intrinsic.width),
            proposal.height.unwrap_or(intrinsic.height),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let viewport = ctx.bounds;
        let proposal = ProposalSize::new(
            Some(to_f32(viewport.width())),
            Some(to_f32(viewport.height())),
        );
        let intrinsic = self
            .child
            .measure(renderer.state_cell(), content_proposal(self.axis, proposal))
            .size;
        let (content_width, content_height) = content_size(self.axis, viewport, intrinsic);
        let clip = ctx.transform.transform_rect_bbox(viewport);
        renderer.list_mut().push_clip(clip);
        self.child.render(
            renderer,
            ctx.child(LayoutRect::new(
                Point::new(0.0, 0.0),
                Size::new(content_width, content_height),
            )),
        );
        renderer.list_mut().pop_clip();
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.child.patch(renderer)
    }
}

fn content_proposal(axis: Axis, proposal: ProposalSize) -> ProposalSize {
    match axis {
        Axis::Horizontal => ProposalSize::new(None, proposal.height),
        Axis::Vertical => ProposalSize::new(proposal.width, None),
        Axis::All => ProposalSize::UNSPECIFIED,
        _ => panic!("dew does not support scroll axis {axis:?}"),
    }
}

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
