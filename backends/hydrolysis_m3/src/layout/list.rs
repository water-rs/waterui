use crate::dimensions::LIST_CONTROL_CORNER_RADIUS;
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, WidgetInteractionState};
use vello::kurbo::{Point, Rect};

pub fn draw_row_background(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    alternate: bool,
) {
    let color = if alternate {
        colors.surface_container_low.peniko()
    } else {
        colors.surface.peniko()
    };
    draw.fill_rect(bounds, &Brush::from(color));
}

pub fn draw_move_control(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(colors.surface.peniko()),
    );
    draw.stroke_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(colors.outline_variant.peniko()),
        1.0,
    );
    let split = bounds.y0 + bounds.height() / 2.0;
    draw.stroke_line(
        Point::new(bounds.x0 + 3.0, split),
        Point::new(bounds.x1 - 3.0, split),
        &Brush::from(colors.on_surface_variant.peniko()),
        1.0,
    );
}

pub fn draw_move_control_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    draw_control_state_layer(draw, bounds, colors.on_surface.peniko(), state);
}

pub fn draw_delete_control(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        LIST_CONTROL_CORNER_RADIUS.into(),
        &Brush::from(colors.error.peniko()),
    );
}

pub fn draw_delete_control_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    draw_control_state_layer(draw, bounds, colors.on_error.peniko(), state);
}

pub fn draw_separator(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(colors.outline_variant.peniko()));
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
