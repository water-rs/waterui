use crate::colors::{OUTLINE_DEFAULT, OUTLINE_SUBTLE, SURFACE_SUBTLE};
use crate::{Brush, DrawContext};
use vello::kurbo::{Point, Rect};

pub fn draw_header_background(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(SURFACE_SUBTLE));
}

pub fn draw_cell_border(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.stroke_rect(bounds, &Brush::from(OUTLINE_SUBTLE), 1.0);
}

pub fn draw_column_separator(draw: &mut dyn DrawContext, from: Point, to: Point) {
    draw.stroke_line(from, to, &Brush::from(OUTLINE_DEFAULT), 1.0);
}
