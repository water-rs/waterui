use crate::dimensions::{
    STEPPER_BUTTON_INTRINSIC_SIZE, STEPPER_BUTTON_MAX_SIZE, STEPPER_BUTTON_MIN_SIZE,
    STEPPER_BUTTON_SPACING, STEPPER_CONTAINER_RADIUS, STEPPER_ICON_SIZE, STEPPER_ICON_STROKE_WIDTH,
    STEPPER_LABEL_SPACING, STEPPER_PRESSED_CONTAINER_RADIUS,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, StepperMetrics, WidgetInteractionState};
use vello::kurbo::{Point, Rect};

pub const fn metrics() -> StepperMetrics {
    StepperMetrics::new(
        STEPPER_BUTTON_MIN_SIZE,
        STEPPER_BUTTON_MAX_SIZE,
        STEPPER_BUTTON_INTRINSIC_SIZE,
        STEPPER_BUTTON_SPACING,
        STEPPER_LABEL_SPACING,
    )
}

pub fn draw_button(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        STEPPER_CONTAINER_RADIUS.into(),
        &Brush::from(colors.surface_container.peniko()),
    );
}

pub fn draw_decrement_icon(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw_minus(colors, draw, bounds);
}

pub fn draw_increment_icon(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw_minus(colors, draw, bounds);
    let center = bounds.center();
    let half = STEPPER_ICON_SIZE * 0.5;
    draw.stroke_line(
        Point::new(center.x, center.y - half),
        Point::new(center.x, center.y + half),
        &Brush::from(colors.on_surface_variant.peniko()),
        STEPPER_ICON_STROKE_WIDTH,
    );
}

fn draw_minus(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    let center = bounds.center();
    let half = STEPPER_ICON_SIZE * 0.5;
    draw.stroke_line(
        Point::new(center.x - half, center.y),
        Point::new(center.x + half, center.y),
        &Brush::from(colors.on_surface_variant.peniko()),
        STEPPER_ICON_STROKE_WIDTH,
    );
}

pub fn draw_button_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    let radius = if state.pressed {
        STEPPER_PRESSED_CONTAINER_RADIUS
    } else {
        STEPPER_CONTAINER_RADIUS
    };
    state_layer::draw_bounded(
        draw,
        bounds,
        radius.into(),
        colors.on_surface_variant.peniko(),
        state,
    );
}

#[cfg(test)]
mod tests {
    use super::metrics;
    use crate::dimensions::{
        STEPPER_BUTTON_INTRINSIC_SIZE, STEPPER_BUTTON_MAX_SIZE, STEPPER_BUTTON_MIN_SIZE,
        STEPPER_BUTTON_SPACING, STEPPER_CONTAINER_RADIUS, STEPPER_ICON_SIZE, STEPPER_LABEL_SPACING,
    };

    #[test]
    fn stepper_uses_material_icon_button_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.button_min_size, STEPPER_BUTTON_MIN_SIZE);
        assert_eq!(metrics.button_max_size, STEPPER_BUTTON_MAX_SIZE);
        assert_eq!(metrics.button_intrinsic_size, STEPPER_BUTTON_INTRINSIC_SIZE);
        assert_eq!(metrics.button_spacing, STEPPER_BUTTON_SPACING);
        assert_eq!(metrics.label_spacing, STEPPER_LABEL_SPACING);
        assert_eq!(STEPPER_BUTTON_INTRINSIC_SIZE, 40.0);
        assert_eq!(STEPPER_ICON_SIZE, 24.0);
        assert_eq!(STEPPER_CONTAINER_RADIUS, 20.0);
    }
}
