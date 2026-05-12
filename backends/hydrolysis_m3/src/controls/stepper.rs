use crate::colors::{FOREGROUND_STRONG, SURFACE_SUBTLE};
use crate::dimensions::{
    STEPPER_BUTTON_INTRINSIC_SIZE, STEPPER_BUTTON_MAX_SIZE, STEPPER_BUTTON_MIN_SIZE,
};
use crate::theme::state_layer;
use crate::{Brush, DrawContext, StepperMetrics, WidgetInteractionState};

pub const fn metrics() -> StepperMetrics {
    StepperMetrics::new(
        STEPPER_BUTTON_MIN_SIZE,
        STEPPER_BUTTON_MAX_SIZE,
        STEPPER_BUTTON_INTRINSIC_SIZE,
    )
}

pub fn draw_button(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 6.0.into(), &Brush::from(SURFACE_SUBTLE));
}

pub fn draw_button_state_layer(
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(draw, bounds, 6.0.into(), FOREGROUND_STRONG, state);
}
