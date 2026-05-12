use crate::{Brush, DrawContext, WidgetInteractionState};
use vello::kurbo::{Point, Rect, RoundedRectRadii};
use vello::peniko::Color;

pub(crate) fn draw_bounded(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    radii: RoundedRectRadii,
    color: Color,
    state: WidgetInteractionState,
) {
    let state_opacity = state.state_layer_opacity();
    if state_opacity > 0.0 {
        draw.push_rounded_layer(state_opacity, bounds, radii);
        draw.fill_rounded_rect(bounds, radii, &Brush::from(color));
        draw.pop_layer();
    }

    let press_opacity = state.press_layer_opacity();
    let Some(origin) = state.press_origin else {
        return;
    };
    if press_opacity == 0.0 {
        return;
    }

    let center = Point::new(
        bounds.x0 + bounds.width() * 0.5,
        bounds.y0 + bounds.height() * 0.5,
    );
    let progress = f64::from(state.press_progress.clamp(0.0, 1.0));
    let max_radius = (bounds.width().hypot(bounds.height()) + 10.0).max(1.0);
    let initial_radius = max_radius * 0.2;
    let radius = initial_radius + (max_radius - initial_radius) * progress;
    let ripple_center = Point::new(
        origin.x + (center.x - origin.x) * progress,
        origin.y + (center.y - origin.y) * progress,
    );

    draw.push_rounded_layer(press_opacity, bounds, radii);
    draw.fill_circle(ripple_center, radius, &Brush::from(color));
    draw.pop_layer();
}

pub(crate) fn draw_unbounded_circle(
    draw: &mut dyn DrawContext,
    center: Point,
    radius: f64,
    color: Color,
    state: WidgetInteractionState,
) {
    let state_opacity = state.state_layer_opacity();
    if state_opacity > 0.0 {
        draw.push_layer(state_opacity, None);
        draw.fill_circle(center, radius, &Brush::from(color));
        draw.pop_layer();
    }

    let press_opacity = state.press_layer_opacity();
    if press_opacity == 0.0 {
        return;
    }
    let progress = f64::from(state.press_progress.clamp(0.0, 1.0));
    let grow_radius = radius.mul_add(0.8 * progress, radius * 0.2);
    draw.push_layer(press_opacity, None);
    draw.fill_circle(center, grow_radius, &Brush::from(color));
    draw.pop_layer();
}
