use crate::dimensions::{
    INPUT_FIELD_HORIZONTAL_INSET, INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH,
    INPUT_FIELD_VERTICAL_INSET, INPUT_LABEL_HEIGHT,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, InputFieldMetrics, WidgetInteractionState};
use waterui_graphics::color::Color;

pub const fn metrics() -> InputFieldMetrics {
    InputFieldMetrics::new(
        INPUT_LABEL_HEIGHT,
        INPUT_FIELD_MIN_WIDTH,
        INPUT_FIELD_MIN_HEIGHT,
        INPUT_FIELD_HORIZONTAL_INSET,
        INPUT_FIELD_VERTICAL_INSET,
    )
}

pub fn placeholder_color(colors: &MaterialColorScheme) -> Color {
    colors.on_surface_variant.view_color()
}

pub fn draw_field(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    draw.fill_rounded_rect(
        bounds,
        4.0.into(),
        &Brush::from(colors.surface_container_highest.peniko()),
    );
    draw.stroke_line(
        vello::kurbo::Point::new(bounds.x0, bounds.y1),
        vello::kurbo::Point::new(bounds.x1, bounds.y1),
        &Brush::from(colors.on_surface_variant.peniko()),
        1.0,
    );
}

pub fn draw_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(draw, bounds, 4.0.into(), colors.on_surface.peniko(), state);
}
