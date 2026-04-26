use crate::colors::{FOREGROUND_STRONG, OUTLINE_SUBTLE, SURFACE_SUBTLE};
use crate::dimensions::NAVIGATION_BAR_CORNER_RADIUS;
use crate::{Brush, DrawContext};
use vello::kurbo::{BezPath, Point, Rect};

pub fn draw_bar(draw: &mut dyn DrawContext, bounds: Rect, background: &Brush) {
    draw.fill_rect(bounds, background);
}

pub fn draw_bar_separator(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(OUTLINE_SUBTLE));
}

pub fn draw_back_button(draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        NAVIGATION_BAR_CORNER_RADIUS.into(),
        &Brush::from(SURFACE_SUBTLE),
    );
    let mut chevron = BezPath::new();
    chevron.move_to(Point::new(bounds.x0 + 19.0, bounds.y0 + 10.0));
    chevron.line_to(Point::new(bounds.x0 + 12.0, bounds.y0 + 15.0));
    chevron.line_to(Point::new(bounds.x0 + 19.0, bounds.y0 + 20.0));
    draw.stroke_path(&chevron, &Brush::from(FOREGROUND_STRONG), 2.0);
}
