use crate::colors::SURFACE_SUBTLE;
use crate::dimensions::{
    STEPPER_BUTTON_INTRINSIC_SIZE, STEPPER_BUTTON_MAX_SIZE, STEPPER_BUTTON_MIN_SIZE,
};
use crate::{Brush, DrawContext, StepperMetrics};

pub(crate) fn metrics() -> StepperMetrics {
    StepperMetrics::new(
        STEPPER_BUTTON_MIN_SIZE,
        STEPPER_BUTTON_MAX_SIZE,
        STEPPER_BUTTON_INTRINSIC_SIZE,
    )
}

pub(crate) fn draw_button(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 6.0.into(), &Brush::from(SURFACE_SUBTLE));
}
