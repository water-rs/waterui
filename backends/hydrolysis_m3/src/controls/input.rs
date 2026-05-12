use crate::colors::{FOREGROUND_STRONG, OUTLINE_DEFAULT, SURFACE_SUBTLE};
use crate::dimensions::{
    INPUT_FIELD_HORIZONTAL_INSET, INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH,
    INPUT_FIELD_VERTICAL_INSET, INPUT_LABEL_HEIGHT,
};
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

pub fn placeholder_color() -> Color {
    Color::srgb(73, 69, 79).with_opacity(1.0)
}

pub fn draw_field(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 4.0.into(), &Brush::from(SURFACE_SUBTLE));
    draw.stroke_line(
        vello::kurbo::Point::new(bounds.x0, bounds.y1),
        vello::kurbo::Point::new(bounds.x1, bounds.y1),
        &Brush::from(OUTLINE_DEFAULT),
        1.0,
    );
}

pub fn draw_state_layer(
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(draw, bounds, 4.0.into(), FOREGROUND_STRONG, state);
}
