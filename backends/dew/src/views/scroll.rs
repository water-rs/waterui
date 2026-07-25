//! Persistent [`ScrollView`] node.

use core::cell::{Cell, RefCell};

use nami::Signal;
use nami::watcher::BoxWatcherGuard;
use waterui_core::Environment;
use waterui_core::layout::{
    Point, ProposalSize, Rect as LayoutRect, Size, StretchAxis, ViewDimensions,
};
use waterui_layout::scroll::{Axis, ScrollController, ScrollView};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, build_node};
use crate::text::DewState;
use crate::views::to_f32;

struct ScrollNode {
    axis: Axis,
    child: Box<dyn DewNode>,
    controller: Option<ScrollController<Point>>,
    applied_scroll_generation: Cell<i32>,
    offset: Cell<Point>,
    _controller_guard: Option<BoxWatcherGuard>,
}

pub fn build(
    renderer: &mut DewRenderer,
    scroll: ScrollView,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let (axis, content, controller) = scroll.into_inner();
    let controller_guard = controller.as_ref().map(|controller| {
        let signals = renderer.signals();
        controller.generation().watch(move |_| {
            signals.request_refresh();
        })
    });
    Box::new(ScrollNode {
        axis,
        child: build_node(renderer, content, env, depth),
        controller,
        applied_scroll_generation: Cell::new(0),
        offset: Cell::new(Point::zero()),
        _controller_guard: controller_guard,
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
        if let Some(controller) = &self.controller {
            let generation = controller.generation().get();
            if generation != self.applied_scroll_generation.get() {
                let target = controller.target().get();
                let max_x = (content_width - to_f32(viewport.width())).max(0.0);
                let max_y = (content_height - to_f32(viewport.height())).max(0.0);
                let offset = match self.axis {
                    Axis::Horizontal => Point::new(target.x.clamp(0.0, max_x), 0.0),
                    Axis::Vertical => Point::new(0.0, target.y.clamp(0.0, max_y)),
                    Axis::All => Point::new(target.x.clamp(0.0, max_x), target.y.clamp(0.0, max_y)),
                    _ => panic!("dew does not support scroll axis {:?}", self.axis),
                };
                self.offset.set(offset);
                self.applied_scroll_generation.set(generation);
            }
        }
        let clip = ctx.transform.transform_rect_bbox(viewport);
        renderer.list_mut().push_clip(clip);
        let offset = self.offset.get();
        self.child.render(
            renderer,
            ctx.child(LayoutRect::new(
                Point::new(-offset.x, -offset.y),
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
