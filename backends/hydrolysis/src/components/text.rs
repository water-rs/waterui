//! Render node that draws `waterui_text::Text`.

use waterui_color::{ResolvedColor, Srgb};
use waterui_core::Environment;
use waterui_text::{TextConfig, styled::StyledStr};

use crate::{
    DrawCommand, LayoutCtx, LayoutResult, NodeSignal, Point, Rect, RenderCtx, RenderNode, Size,
};

/// Naive text node; renders plain strings until shaping is implemented.
pub struct TextNode {
    content: NodeSignal<StyledStr>,
    plain: String,
    color: ResolvedColor,
    font_size: f32,
    bounds: Rect,
}

impl core::fmt::Debug for TextNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextNode")
            .field("plain", &self.plain)
            .field("font_size", &self.font_size)
            .finish()
    }
}

impl TextNode {
    /// Builds a text node from the provided `TextConfig`.
    #[must_use]
    pub fn new(config: TextConfig, _env: &Environment) -> Self {
        let content = NodeSignal::new(config.content);
        let plain = content.current().to_plain().to_string();
        Self {
            content,
            plain,
            color: ResolvedColor::from_srgb(Srgb::new(0.0, 0.0, 0.0)),
            font_size: 16.0,
            bounds: Rect::default(),
        }
    }

    fn refresh_plain(&mut self) {
        if self.content.refresh() {
            self.plain = self.content.current().to_plain().to_string();
        }
    }
}

impl RenderNode for TextNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        self.refresh_plain();
        // TODO(text-measurement): integrate real font metrics (cosmic-text).
        let width = self.plain.chars().count() as f32 * (self.font_size * 0.6);
        let height = self.font_size * 1.2;
        self.bounds = Rect::new(Point::new(0.0, 0.0), Size::new(width, height));
        LayoutResult {
            size: self.bounds.size,
        }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        // TODO(text-styling): emit glyph-level draw commands with per-span styles.
        ctx.push(DrawCommand::Text {
            content: self.plain.clone(),
            origin: self.bounds.origin,
            color: self.color,
            size: self.font_size,
        });
    }

    fn update_reactive(&mut self) {
        self.refresh_plain();
    }
}
