use crate::dimensions::SCROLL_INDICATOR_CORNER_RADIUS;
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext};
use vello::kurbo::Rect;

pub fn draw_indicator(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        SCROLL_INDICATOR_CORNER_RADIUS.into(),
        &Brush::from(colors.on_surface_variant.peniko().with_alpha(0.55)),
    );
}
