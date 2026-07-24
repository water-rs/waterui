use crate::dimensions::{
    INPUT_FIELD_HORIZONTAL_INSET, INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH,
    INPUT_FIELD_VERTICAL_INSET, INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT,
    INPUT_FILLED_CONTAINER_TOP_RADIUS, INPUT_FILLED_FOCUS_ACTIVE_INDICATOR_HEIGHT,
    INPUT_LABEL_HEIGHT,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, InputFieldMetrics, WidgetInteractionState};
use material_color_utils::utils::color_utils::Argb;
use vello::kurbo::{Point, Rect, RoundedRectRadii};
use vello::peniko::Color as PenikoColor;
use waterui_graphics::color::Color;

const INPUT_SELECTION_ALPHA: f32 = 0.28;

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

pub fn selection_brush(colors: &MaterialColorScheme) -> Brush {
    Brush::from(role_with_alpha(
        colors.primary.argb(),
        INPUT_SELECTION_ALPHA,
    ))
}

pub fn caret_brush(colors: &MaterialColorScheme, opacity: f32) -> Brush {
    Brush::from(role_with_alpha(colors.primary.argb(), opacity))
}

fn role_with_alpha(color: Argb, alpha: f32) -> PenikoColor {
    PenikoColor::new([
        f32::from(color.red()) / 255.0,
        f32::from(color.green()) / 255.0,
        f32::from(color.blue()) / 255.0,
        alpha.clamp(0.0, 1.0),
    ])
}

pub fn draw_field(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    let container_color = if state.disabled {
        role_with_alpha(colors.on_surface.argb(), 0.04)
    } else {
        colors.surface_container_highest.peniko()
    };
    draw.fill_rounded_rect(
        bounds,
        RoundedRectRadii::new(
            INPUT_FILLED_CONTAINER_TOP_RADIUS,
            INPUT_FILLED_CONTAINER_TOP_RADIUS,
            0.0,
            0.0,
        ),
        &Brush::from(container_color),
    );
    let baseline_color = if state.disabled {
        role_with_alpha(colors.on_surface.argb(), 0.38)
    } else if state.hovered {
        colors.on_surface.peniko()
    } else {
        colors.on_surface_variant.peniko()
    };
    let baseline_y = bounds.y1 - INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT / 2.0;
    draw.stroke_line(
        Point::new(bounds.x0, baseline_y),
        Point::new(bounds.x1, baseline_y),
        &Brush::from(baseline_color),
        INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT,
    );
    let focus_alpha = if state.focus_progress > 0.0 {
        state.focus_progress
    } else if state.focus_visible {
        1.0
    } else {
        0.0
    };
    if focus_alpha > 0.0 {
        let focus_y = bounds.y1 - INPUT_FILLED_FOCUS_ACTIVE_INDICATOR_HEIGHT / 2.0;
        draw.stroke_line(
            Point::new(bounds.x0, focus_y),
            Point::new(bounds.x1, focus_y),
            &Brush::from(role_with_alpha(colors.primary.argb(), focus_alpha)),
            INPUT_FILLED_FOCUS_ACTIVE_INDICATOR_HEIGHT,
        );
    }
}

pub fn draw_state_layer(
    _colors: &MaterialColorScheme,
    _draw: &mut dyn DrawContext,
    _bounds: Rect,
    _state: WidgetInteractionState,
) {
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};

    use super::{
        MaterialColorScheme, WidgetInteractionState, caret_brush, draw_field, metrics,
        selection_brush,
    };
    use crate::dimensions::{
        INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH, INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT,
        INPUT_FILLED_CONTAINER_TOP_RADIUS, INPUT_FILLED_FOCUS_ACTIVE_INDICATOR_HEIGHT,
    };
    use crate::{Brush, DrawContext};

    #[derive(Default)]
    struct RecordingDrawContext {
        rounded_radii: Option<RoundedRectRadii>,
        stroke_width: Option<f64>,
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}

        fn fill_rounded_rect(&mut self, _rect: Rect, radii: RoundedRectRadii, _brush: &Brush) {
            self.rounded_radii = Some(radii);
        }

        fn stroke_rect(&mut self, _rect: Rect, _brush: &Brush, _width: f64) {}

        fn stroke_rounded_rect(
            &mut self,
            _rect: Rect,
            _radii: RoundedRectRadii,
            _brush: &Brush,
            _width: f64,
        ) {
        }

        fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, width: f64) {
            self.stroke_width = Some(width);
        }

        fn stroke_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush, _width: f64) {}

        fn fill_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush) {}

        fn fill_path(&mut self, _path: &BezPath, _brush: &Brush) {}

        fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {}

        fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}

        fn pop_layer(&mut self) {}

        fn push_transform(&mut self, _affine: Affine) {}

        fn pop_transform(&mut self) {}
    }

    #[test]
    fn filled_text_field_metrics_match_material_web_latest_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.min_height, INPUT_FIELD_MIN_HEIGHT);
        assert_eq!(metrics.min_width, INPUT_FIELD_MIN_WIDTH);
        assert_eq!(INPUT_FIELD_MIN_HEIGHT, 56.0);
        assert_eq!(INPUT_FILLED_CONTAINER_TOP_RADIUS, 4.0);
        assert_eq!(INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT, 1.0);
    }

    #[test]
    fn filled_text_field_uses_top_only_container_shape() {
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();
        draw_field(
            &colors,
            &mut draw,
            Rect::new(0.0, 0.0, 120.0, 56.0),
            WidgetInteractionState::NONE,
        );

        assert_eq!(
            draw.rounded_radii,
            Some(RoundedRectRadii::new(
                INPUT_FILLED_CONTAINER_TOP_RADIUS,
                INPUT_FILLED_CONTAINER_TOP_RADIUS,
                0.0,
                0.0,
            ))
        );
        assert_eq!(
            draw.stroke_width,
            Some(INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT)
        );
    }

    #[test]
    fn filled_text_field_focus_indicator_matches_material_web_latest_tokens() {
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();
        draw_field(
            &colors,
            &mut draw,
            Rect::new(0.0, 0.0, 120.0, 56.0),
            WidgetInteractionState {
                focus_visible: true,
                focus_progress: 1.0,
                ..WidgetInteractionState::NONE
            },
        );

        assert_eq!(
            draw.stroke_width,
            Some(INPUT_FILLED_FOCUS_ACTIVE_INDICATOR_HEIGHT)
        );
    }

    #[test]
    fn filled_text_field_caret_and_selection_use_primary_role() {
        let colors = MaterialColorScheme::baseline_light();

        let Brush::Solid(selection) = selection_brush(&colors) else {
            panic!("Material text selection must be a solid primary color layer");
        };
        let Brush::Solid(caret) = caret_brush(&colors, 0.5) else {
            panic!("Material text caret must be a solid primary color layer");
        };
        let primary = colors.primary.argb();

        assert_eq!(selection.components[0], f32::from(primary.red()) / 255.0);
        assert_eq!(selection.components[1], f32::from(primary.green()) / 255.0);
        assert_eq!(selection.components[2], f32::from(primary.blue()) / 255.0);
        assert_eq!(selection.components[3], 0.28);
        assert_eq!(caret.components[0], f32::from(primary.red()) / 255.0);
        assert_eq!(caret.components[1], f32::from(primary.green()) / 255.0);
        assert_eq!(caret.components[2], f32::from(primary.blue()) / 255.0);
        assert_eq!(caret.components[3], 0.5);
    }
}
