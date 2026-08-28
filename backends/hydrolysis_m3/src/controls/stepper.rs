//! Material Design 3 stepper chrome.
//!
//! Material has no stepper component, but it does have a rule for a pair of
//! buttons that belong together: a connected button group. The two far ends are
//! fully round, the corners either side of the seam are tucked in, and holding
//! one tightens its corners further. That is what a stepper is, so it is drawn
//! with `ConnectedButtonGroupSmallTokens` rather than as two loose circles.

use crate::dimensions::{
    STEPPER_BUTTON_INTRINSIC_SIZE, STEPPER_BUTTON_MAX_SIZE, STEPPER_BUTTON_MIN_SIZE,
    STEPPER_BUTTON_SPACING, STEPPER_ICON_SIZE, STEPPER_INNER_CORNER_RADIUS, STEPPER_LABEL_SPACING,
    STEPPER_PRESSED_INNER_CORNER_RADIUS,
};
use crate::icon_paths::{IconGrid, add, remove};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, StepperEnd, StepperMetrics, WidgetInteractionState};
use vello::kurbo::{Rect, RoundedRectRadii};

pub const fn metrics() -> StepperMetrics {
    StepperMetrics::new(
        STEPPER_BUTTON_MIN_SIZE,
        STEPPER_BUTTON_MAX_SIZE,
        STEPPER_BUTTON_INTRINSIC_SIZE,
        STEPPER_BUTTON_SPACING,
        STEPPER_LABEL_SPACING,
    )
}

/// The four corner radii for one end of the pair.
///
/// The outer corners are `CornerFull`, which on a container of this height is
/// half of it. The inner pair is `InnerCornerCornerSize`, tightening to
/// `PressedInnerCornerCornerSize` as a press grows.
fn radii(bounds: Rect, end: StepperEnd, press: f64) -> RoundedRectRadii {
    let outer = bounds.height() / 2.0;
    let inner = (STEPPER_PRESSED_INNER_CORNER_RADIUS - STEPPER_INNER_CORNER_RADIUS)
        .mul_add(press.clamp(0.0, 1.0), STEPPER_INNER_CORNER_RADIUS)
        .min(outer);
    match end {
        StepperEnd::Decrement => RoundedRectRadii::new(outer, inner, inner, outer),
        StepperEnd::Increment => RoundedRectRadii::new(inner, outer, outer, inner),
    }
}

/// How far the most recent press has grown, which the corner morph follows.
fn press_progress(state: WidgetInteractionState) -> f64 {
    f64::from(
        state
            .press_waves
            .latest()
            .map_or(0.0, |wave| wave.progress)
            .clamp(0.0, 1.0),
    )
}

pub fn draw_button(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    end: StepperEnd,
    state: WidgetInteractionState,
) {
    draw.fill_rounded_rect(
        bounds,
        radii(bounds, end, press_progress(state)),
        &Brush::from(colors.surface_container.peniko()),
    );
}

pub fn draw_decrement_icon(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    let grid = IconGrid::centered(bounds.center(), STEPPER_ICON_SIZE);
    draw.fill_path(
        &remove(grid),
        &Brush::from(colors.on_surface_variant.peniko()),
    );
}

pub fn draw_increment_icon(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    let grid = IconGrid::centered(bounds.center(), STEPPER_ICON_SIZE);
    draw.fill_path(&add(grid), &Brush::from(colors.on_surface_variant.peniko()));
}

pub fn draw_button_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    end: StepperEnd,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(
        draw,
        bounds,
        radii(bounds, end, press_progress(state)),
        colors.on_surface_variant.peniko(),
        state,
    );
}

#[cfg(test)]
mod tests {
    use super::{metrics, radii};
    use crate::StepperEnd;
    use crate::dimensions::{
        STEPPER_BUTTON_INTRINSIC_SIZE, STEPPER_BUTTON_MAX_SIZE, STEPPER_BUTTON_MIN_SIZE,
        STEPPER_BUTTON_SPACING, STEPPER_ICON_SIZE, STEPPER_INNER_CORNER_RADIUS,
        STEPPER_LABEL_SPACING, STEPPER_PRESSED_INNER_CORNER_RADIUS,
    };
    use vello::kurbo::Rect;

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
        // ConnectedButtonGroupSmallTokens: BetweenSpace, InnerCornerCornerSize
        // (CornerValueSmall) and PressedInnerCornerCornerSize
        // (CornerValueExtraSmall).
        assert_eq!(STEPPER_BUTTON_SPACING, 2.0);
        assert_eq!(STEPPER_INNER_CORNER_RADIUS, 8.0);
        assert_eq!(STEPPER_PRESSED_INNER_CORNER_RADIUS, 4.0);
    }

    /// The pair reads as one silhouette: round on the outside, tucked at the
    /// seam, and each end tucks on the side that faces the other.
    #[test]
    fn only_the_pairs_outer_corners_are_fully_round() {
        let bounds = Rect::new(0.0, 0.0, 40.0, 40.0);
        let outer = bounds.height() / 2.0;

        let minus = radii(bounds, StepperEnd::Decrement, 0.0);
        assert_eq!(minus.top_left, outer);
        assert_eq!(minus.bottom_left, outer);
        assert_eq!(minus.top_right, STEPPER_INNER_CORNER_RADIUS);
        assert_eq!(minus.bottom_right, STEPPER_INNER_CORNER_RADIUS);

        let plus = radii(bounds, StepperEnd::Increment, 0.0);
        assert_eq!(plus.top_right, outer);
        assert_eq!(plus.bottom_right, outer);
        assert_eq!(plus.top_left, STEPPER_INNER_CORNER_RADIUS);
        assert_eq!(plus.bottom_left, STEPPER_INNER_CORNER_RADIUS);
    }

    /// A press tightens the seam corners and leaves the outer ones alone.
    #[test]
    fn a_press_tightens_only_the_seam_corners() {
        let bounds = Rect::new(0.0, 0.0, 40.0, 40.0);
        let pressed = radii(bounds, StepperEnd::Decrement, 1.0);

        assert_eq!(pressed.top_right, STEPPER_PRESSED_INNER_CORNER_RADIUS);
        assert_eq!(pressed.bottom_right, STEPPER_PRESSED_INNER_CORNER_RADIUS);
        assert_eq!(pressed.top_left, bounds.height() / 2.0);
    }
}
