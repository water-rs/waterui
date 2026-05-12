use crate::colors::{ACCENT, OUTLINE_SUBTLE, THUMB_OUTLINE};
use crate::dimensions::{
    SLIDER_HORIZONTAL_INSET, SLIDER_HORIZONTAL_SPACING, SLIDER_MIN_TRACK_WIDTH,
    SLIDER_THUMB_RADIUS, SLIDER_TRACK_HEIGHT, SLIDER_VERTICAL_SPACING,
};
use crate::theme::state_layer;
use crate::{Brush, DrawContext, SliderMetrics, WidgetInteractionState};

pub const fn metrics() -> SliderMetrics {
    SliderMetrics::new(
        SLIDER_HORIZONTAL_INSET,
        SLIDER_HORIZONTAL_SPACING,
        SLIDER_VERTICAL_SPACING,
        SLIDER_MIN_TRACK_WIDTH,
        SLIDER_TRACK_HEIGHT,
        SLIDER_THUMB_RADIUS,
    )
}

pub fn draw_track(
    draw: &mut dyn DrawContext,
    track_rect: vello::kurbo::Rect,
    fill_rect: vello::kurbo::Rect,
) {
    draw.fill_rounded_rect(track_rect, 2.0.into(), &Brush::from(OUTLINE_SUBTLE));
    draw.fill_rounded_rect(fill_rect, 2.0.into(), &Brush::from(ACCENT));
}

pub fn draw_thumb(draw: &mut dyn DrawContext, center: vello::kurbo::Point, radius: f64) {
    draw.fill_circle(center, radius, &Brush::from(THUMB_OUTLINE));
}

pub fn draw_thumb_state_layer(
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    _radius: f64,
    state: WidgetInteractionState,
) {
    state_layer::draw_unbounded_circle(draw, center, 20.0, ACCENT, state);
}
