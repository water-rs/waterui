//! [`Divider`]: a one-pixel hairline separating stacked content.

use kurbo::Rect;
use waterui_core::Environment;
use waterui_core::layout::Size;
use waterui_layout::Divider;
use waterui_layout::stack::Axis;

use crate::dispatch::{DewRenderer, RenderContext};
use crate::theme;
use crate::views::to_f32;

/// Hairline thickness in logical pixels.
const THICKNESS: f64 = 1.0;

/// Draws the hairline along the parent stack's cross axis: horizontal in a
/// `VStack`, vertical in an `HStack`.
pub fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    _divider: Divider,
    env: &Environment,
) {
    let bounds = ctx.bounds;
    let vertical = matches!(env.get::<Axis>(), Some(Axis::Horizontal));
    let line = if vertical {
        Rect::new(bounds.x0, bounds.y0, bounds.x0 + THICKNESS, bounds.y1)
    } else {
        Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + THICKNESS)
    };
    renderer
        .list_mut()
        .fill(&line, ctx.transform, theme::BORDER);
}

/// One logical pixel in both directions; the cross-axis stretch reported by
/// the dispatcher expands it along the parent stack.
pub const fn measure() -> Size {
    Size::new(to_f32(THICKNESS), to_f32(THICKNESS))
}
