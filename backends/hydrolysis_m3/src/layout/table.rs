use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext};
use vello::kurbo::{Point, Rect};

pub fn draw_header_background(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
) {
    draw.fill_rect(bounds, &Brush::from(colors.surface_container_low.peniko()));
}

pub fn draw_cell_border(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.stroke_rect(bounds, &Brush::from(colors.outline_variant.peniko()), 1.0);
}

pub fn draw_column_separator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    from: Point,
    to: Point,
) {
    draw.stroke_line(from, to, &Brush::from(colors.outline.peniko()), 1.0);
}
