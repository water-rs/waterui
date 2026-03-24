use crate::colors::{OUTLINE_DEFAULT, SURFACE_DEFAULT};
use crate::dimensions::{
    INPUT_FIELD_HORIZONTAL_INSET, INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH,
    INPUT_FIELD_VERTICAL_INSET, INPUT_LABEL_HEIGHT,
};
use crate::{Brush, DrawContext, InputFieldMetrics};
use waterui_graphics::color::Color;

pub(crate) fn metrics() -> InputFieldMetrics {
    InputFieldMetrics::new(
        INPUT_LABEL_HEIGHT,
        INPUT_FIELD_MIN_WIDTH,
        INPUT_FIELD_MIN_HEIGHT,
        INPUT_FIELD_HORIZONTAL_INSET,
        INPUT_FIELD_VERTICAL_INSET,
    )
}

pub(crate) fn placeholder_color() -> Color {
    Color::srgb(142, 142, 147).with_opacity(0.8)
}

pub(crate) fn draw_field(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 6.0.into(), &Brush::from(SURFACE_DEFAULT));
    draw.stroke_rounded_rect(bounds, 6.0.into(), &Brush::from(OUTLINE_DEFAULT), 1.0);
}
