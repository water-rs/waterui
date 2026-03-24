use crate::colors::{ACCENT, OUTLINE_SUBTLE, SURFACE_DEFAULT, THUMB_OUTLINE};
use crate::dimensions::{
    SLIDER_HORIZONTAL_INSET, SLIDER_HORIZONTAL_SPACING, SLIDER_MIN_TRACK_WIDTH,
    SLIDER_THUMB_RADIUS, SLIDER_TRACK_HEIGHT, SLIDER_VERTICAL_SPACING,
};
use crate::{Brush, DrawContext, SliderMetrics};

pub(crate) fn metrics() -> SliderMetrics {
    SliderMetrics::new(
        SLIDER_HORIZONTAL_INSET,
        SLIDER_HORIZONTAL_SPACING,
        SLIDER_VERTICAL_SPACING,
        SLIDER_MIN_TRACK_WIDTH,
        SLIDER_TRACK_HEIGHT,
        SLIDER_THUMB_RADIUS,
    )
}

pub(crate) fn draw_track(
    draw: &mut dyn DrawContext,
    track_rect: vello::kurbo::Rect,
    fill_rect: vello::kurbo::Rect,
) {
    draw.fill_rect(track_rect, &Brush::from(OUTLINE_SUBTLE));
    draw.fill_rect(fill_rect, &Brush::from(ACCENT));
}

pub(crate) fn draw_thumb(draw: &mut dyn DrawContext, center: vello::kurbo::Point, radius: f64) {
    draw.fill_circle(center, radius, &Brush::from(SURFACE_DEFAULT));
    draw.stroke_circle(center, radius, &Brush::from(THUMB_OUTLINE), 1.0);
}
