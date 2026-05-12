use crate::colors::{
    DESTRUCTIVE, FOREGROUND_MUTED, FOREGROUND_STRONG, OUTLINE_SUBTLE, ROW_EVEN, ROW_ODD,
    SURFACE_DEFAULT,
};
use crate::dimensions::LIST_CONTROL_CORNER_RADIUS;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, WidgetInteractionState};
use vello::kurbo::{Point, Rect};

pub fn draw_row_background(draw: &mut dyn DrawContext, bounds: Rect, alternate: bool) {
    let color = if alternate { ROW_ODD } else { ROW_EVEN };
    draw.fill_rect(bounds, &Brush::from(color));
}

pub fn draw_move_control(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(ROW_EVEN),
    );
    draw.stroke_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(OUTLINE_SUBTLE),
        1.0,
    );
    let split = bounds.y0 + bounds.height() / 2.0;
    draw.stroke_line(
        Point::new(bounds.x0 + 3.0, split),
        Point::new(bounds.x1 - 3.0, split),
        &Brush::from(FOREGROUND_MUTED),
        1.0,
    );
}

pub fn draw_move_control_state_layer(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    draw_control_state_layer(draw, bounds, FOREGROUND_STRONG, state);
}

pub fn draw_delete_control(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(DESTRUCTIVE),
    );
}

pub fn draw_delete_control_state_layer(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    draw_control_state_layer(draw, bounds, SURFACE_DEFAULT, state);
}

pub fn draw_separator(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(OUTLINE_SUBTLE));
}

fn draw_control_state_layer(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    color: vello::peniko::Color,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(
        draw,
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        color,
        state,
    );
}
