//! Progress indicator render node.

use waterui::{
    AnyView,
    component::progress::{ProgressConfig, ProgressStyle},
};
use waterui_color::ResolvedColor;

use crate::{
    DrawCommand, LayoutCtx, LayoutResult, NodeSignal, Point, Rect, RenderCtx, RenderNode, Size,
};

/// Placeholder node for `Progress` views.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ProgressNode {
    value: NodeSignal<f64>,
    style: ProgressStyle,
    label: AnyView,
    size: Size,
}

impl ProgressNode {
    #[must_use]
    /// Creates a progress node from the provided configuration.
    pub fn new(config: ProgressConfig) -> Self {
        Self {
            value: NodeSignal::new(config.value),
            style: config.style,
            label: config.label,
            size: Size::default(),
        }
    }
}

impl RenderNode for ProgressNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        self.value.refresh();
        let size = match self.style {
            ProgressStyle::Circular => Size::new(48.0, 48.0),
            ProgressStyle::Linear => Size::new(160.0, 12.0),
            _ => Size::new(160.0, 12.0),
        };
        self.size = size;
        LayoutResult { size }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        let track_color = ResolvedColor {
            red: 0.7,
            green: 0.7,
            blue: 0.7,
            headroom: 0.0,
            opacity: 1.0,
        };
        let fill_color = ResolvedColor {
            red: 0.2,
            green: 0.6,
            blue: 0.9,
            headroom: 0.0,
            opacity: 1.0,
        };
        let fraction = self.value.current().clamp(0.0, 1.0).min(1.0).max(0.0) as f32;

        match self.style {
            ProgressStyle::Linear => {
                let track = Rect::new(Point::new(0.0, 0.0), self.size);
                ctx.push(DrawCommand::SolidRect {
                    rect: track,
                    color: track_color,
                });
                let fill_width = self.size.width * fraction;
                let fill = Rect::new(
                    Point::new(0.0, 0.0),
                    Size::new(fill_width, self.size.height),
                );
                ctx.push(DrawCommand::SolidRect {
                    rect: fill,
                    color: fill_color,
                });
            }
            ProgressStyle::Circular => {
                // TODO(progress): render circular progress via path commands.
                ctx.push(DrawCommand::Placeholder("Progress circular"));
            }
            _ => ctx.push(DrawCommand::Placeholder("Progress")),
        }
    }
}
