use crate::colors::{
    ACCENT, ACCENT_TRACK_OFF, FOREGROUND_STRONG, THUMB_OUTLINE_SOFT, TOGGLE_OUTLINE_OFF,
    TOGGLE_OUTLINE_ON,
};
use crate::dimensions::{TOGGLE_CHECKBOX_SIZE, TOGGLE_SWITCH_HEIGHT, TOGGLE_SWITCH_WIDTH};
use crate::theme::state_layer;
use crate::{Brush, DrawContext, ToggleMetrics, WidgetInteractionState, lerp_color};
use waterui_controls::toggle::ToggleStyle;

pub fn metrics(style: ToggleStyle) -> ToggleMetrics {
    match style {
        ToggleStyle::Automatic | ToggleStyle::Switch => {
            ToggleMetrics::new(TOGGLE_SWITCH_WIDTH, TOGGLE_SWITCH_HEIGHT)
        }
        ToggleStyle::Checkbox => ToggleMetrics::new(TOGGLE_CHECKBOX_SIZE, TOGGLE_CHECKBOX_SIZE),
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

pub fn draw_switch(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect, progress: f32) {
    let track_color = lerp_color(ACCENT_TRACK_OFF, ACCENT, progress);
    let handle_radius = crate::lerp_f64(8.0, 12.0, progress);
    let thumb_center_x = crate::lerp_f64(
        bounds.x0 + 4.0 + handle_radius,
        bounds.x1 - 4.0 - handle_radius,
        progress,
    );
    let thumb_center = vello::kurbo::Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
    draw.fill_rounded_rect(bounds, 16.0.into(), &Brush::from(track_color));
    draw.stroke_rounded_rect(
        bounds,
        16.0.into(),
        &Brush::from(lerp_color(TOGGLE_OUTLINE_OFF, TOGGLE_OUTLINE_ON, progress)),
        2.0,
    );
    draw.fill_circle(
        thumb_center,
        handle_radius,
        &Brush::from(vello::peniko::Color::WHITE),
    );
    draw.stroke_circle(
        thumb_center,
        handle_radius,
        &Brush::from(THUMB_OUTLINE_SOFT),
        1.0,
    );
}

pub fn draw_switch_state_layer(
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    progress: f32,
    state: WidgetInteractionState,
) {
    let handle_radius = crate::lerp_f64(8.0, 12.0, progress);
    let thumb_center_x = crate::lerp_f64(
        bounds.x0 + 4.0 + handle_radius,
        bounds.x1 - 4.0 - handle_radius,
        progress,
    );
    let center = vello::kurbo::Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
    state_layer::draw_unbounded_circle(draw, center, 20.0, FOREGROUND_STRONG, state);
}

pub fn draw_checkbox(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect, progress: f32) {
    draw.fill_rounded_rect(
        bounds,
        2.0.into(),
        &Brush::from(lerp_color(vello::peniko::Color::WHITE, ACCENT, progress)),
    );
    draw.stroke_rounded_rect(
        bounds,
        2.0.into(),
        &Brush::from(lerp_color(TOGGLE_OUTLINE_OFF, TOGGLE_OUTLINE_ON, progress)),
        2.0,
    );
    if progress <= 0.0 {
        return;
    }
    let check = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(
            bounds.width().mul_add(0.25, bounds.x0),
            bounds.height().mul_add(0.55, bounds.y0),
        )),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
            bounds.width().mul_add(0.45, bounds.x0),
            bounds.height().mul_add(0.75, bounds.y0),
        )),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
            bounds.width().mul_add(0.78, bounds.x0),
            bounds.height().mul_add(0.3, bounds.y0),
        )),
    ]);
    draw.stroke_path(
        &check,
        &Brush::from(vello::peniko::Color::new([1.0, 1.0, 1.0, progress])),
        2.0,
    );
}

pub fn draw_checkbox_state_layer(
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    progress: f32,
    state: WidgetInteractionState,
) {
    let center = vello::kurbo::Point::new(
        bounds.x0 + bounds.width() / 2.0,
        bounds.y0 + bounds.height() / 2.0,
    );
    let color = if progress > 0.0 {
        ACCENT
    } else {
        FOREGROUND_STRONG
    };
    state_layer::draw_unbounded_circle(draw, center, 20.0, color, state);
}
