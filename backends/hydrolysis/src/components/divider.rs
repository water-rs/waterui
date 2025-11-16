//! Divider render node (horizontal rule).

use waterui::prelude::Divider;

use crate::{DrawCommand, LayoutCtx, LayoutResult, RenderCtx, RenderNode, Size};

/// Placeholder divider node (renders a thin line).
#[derive(Debug, Default)]
pub struct DividerNode;

impl DividerNode {
    #[must_use]
    /// Creates a divider node from the source view (currently unused).
    pub const fn new(_divider: Divider) -> Self {
        Self
    }
}

impl RenderNode for DividerNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        LayoutResult {
            size: Size::new(200.0, 1.0),
        }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        ctx.push(DrawCommand::Placeholder("Divider"));
    }
}
