//! Retained [`Divider`] node.

use core::cell::RefCell;

use kurbo::Rect;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_layout::stack::Axis;

use crate::dispatch::{DewNode, DewRenderer, RenderContext};
use crate::text::DewState;
use crate::theme;
use crate::views::to_f32;

const THICKNESS: f64 = 1.0;

struct DividerNode {
    vertical: bool,
}

pub fn build(env: &Environment) -> Box<dyn DewNode> {
    Box::new(DividerNode {
        vertical: matches!(env.get::<Axis>(), Some(Axis::Horizontal)),
    })
}

impl DewNode for DividerNode {
    fn measure(&self, _state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(to_f32(THICKNESS), to_f32(THICKNESS)))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let bounds = ctx.bounds;
        let line = if self.vertical {
            Rect::new(bounds.x0, bounds.y0, bounds.x0 + THICKNESS, bounds.y1)
        } else {
            Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + THICKNESS)
        };
        renderer
            .list_mut()
            .fill(&line, ctx.transform, theme::BORDER);
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::CrossAxis
    }
}
