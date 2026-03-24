use crate::colors::{ACCENT_SELECTION, OUTLINE_SUBTLE, SURFACE_MUTED};
use crate::dimensions::TABS_HIGHLIGHT_CORNER_RADIUS;
use crate::{Brush, DrawContext};
use vello::kurbo::Rect;

pub(crate) fn draw_bar(draw: &mut dyn DrawContext, bounds: Rect, top_edge: bool) {
    draw.fill_rect(bounds, &Brush::from(SURFACE_MUTED));
    let separator = if top_edge {
        Rect::new(bounds.x0, bounds.y1 - 1.0, bounds.x1, bounds.y1)
    } else {
        Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + 1.0)
    };
    draw.fill_rect(separator, &Brush::from(OUTLINE_SUBTLE));
}

pub(crate) fn draw_highlight(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        TABS_HIGHLIGHT_CORNER_RADIUS.into(),
        &Brush::from(ACCENT_SELECTION),
    );
}
