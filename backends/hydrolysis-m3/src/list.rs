use crate::colors::{DESTRUCTIVE, FOREGROUND_MUTED, OUTLINE_SUBTLE, ROW_EVEN, ROW_ODD};
use crate::dimensions::LIST_CONTROL_CORNER_RADIUS;
use crate::{Brush, DrawContext};
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

pub fn draw_delete_control(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(DESTRUCTIVE),
    );
}

pub fn draw_separator(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(OUTLINE_SUBTLE));
}
