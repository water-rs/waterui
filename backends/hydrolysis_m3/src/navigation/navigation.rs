use crate::dimensions::NAVIGATION_BAR_CORNER_RADIUS;
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext};
use vello::kurbo::{BezPath, Point, Rect};

pub fn draw_bar(draw: &mut dyn DrawContext, bounds: Rect, background: &Brush) {
    draw.fill_rect(bounds, background);
}

pub fn draw_bar_separator(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(colors.outline_variant.peniko()));
}

pub fn draw_back_button(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        NAVIGATION_BAR_CORNER_RADIUS.into(),
        &Brush::from(colors.surface_container_high.peniko()),
    );
    let mut chevron = BezPath::new();
    chevron.move_to(Point::new(bounds.x0 + 19.0, bounds.y0 + 10.0));
    chevron.line_to(Point::new(bounds.x0 + 12.0, bounds.y0 + 15.0));
    chevron.line_to(Point::new(bounds.x0 + 19.0, bounds.y0 + 20.0));
    draw.stroke_path(&chevron, &Brush::from(colors.on_surface.peniko()), 2.0);
}
