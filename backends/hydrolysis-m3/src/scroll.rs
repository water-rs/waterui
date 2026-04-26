use crate::colors::SCROLL_INDICATOR;
use crate::dimensions::SCROLL_INDICATOR_CORNER_RADIUS;
use crate::{Brush, DrawContext};
use vello::kurbo::Rect;

pub fn draw_indicator(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        SCROLL_INDICATOR_CORNER_RADIUS.into(),
        &Brush::from(SCROLL_INDICATOR),
    );
}
