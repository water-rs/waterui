//! Layout-related render nodes.

use waterui_layout::{
    Layout,
    spacer::{Spacer, SpacerLayout},
};

use crate::{LayoutCtx, LayoutResult, RenderCtx, RenderNode, Size};

/// Placeholder node for `FixedContainer` views until full layout plumbing exists.
#[derive(Debug)]
pub struct FixedContainerNode {
    layout: Box<dyn Layout>,
}

/// Render node used for `Spacer` views (fills flexible layout gaps).
#[derive(Debug)]
pub struct SpacerNode {
    layout: SpacerLayout,
}

impl FixedContainerNode {
    /// Creates a new layout node from a boxed [`Layout`].
    #[must_use]
    pub fn new(layout: Box<dyn Layout>) -> Self {
        Self { layout }
    }
}

impl SpacerNode {
    /// Creates a spacer node with the provided spacer definition.
    #[must_use]
    pub fn new(spacer: Spacer) -> Self {
        Self {
            layout: SpacerLayout::from(spacer),
        }
    }
}

impl RenderNode for FixedContainerNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        // TODO(layout): call into `self.layout` with child metadata and propagate Rects.
        LayoutResult {
            size: Size::default(),
        }
    }

    fn paint(&mut self, _ctx: &mut RenderCtx<'_>) {
        // Layout nodes do not draw anything themselves.
        let _ = self.layout.as_mut();
    }
}

impl RenderNode for SpacerNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        let _ = &self.layout;
        LayoutResult {
            // TODO(layout): integrate with LayoutEngine to size spacers properly.
            size: Size::default(),
        }
    }

    fn paint(&mut self, _ctx: &mut RenderCtx<'_>) {}
}
