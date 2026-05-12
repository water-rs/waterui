use crate::dimensions::{
    INPUT_FIELD_HORIZONTAL_INSET, INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH,
    INPUT_FIELD_VERTICAL_INSET, INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT,
    INPUT_FILLED_CONTAINER_TOP_RADIUS, INPUT_LABEL_HEIGHT,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, InputFieldMetrics, WidgetInteractionState};
use vello::kurbo::{Point, Rect, RoundedRectRadii};
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

pub fn draw_field(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        RoundedRectRadii::new(
            INPUT_FILLED_CONTAINER_TOP_RADIUS,
            INPUT_FILLED_CONTAINER_TOP_RADIUS,
            0.0,
            0.0,
        ),
        &Brush::from(colors.surface_container_highest.peniko()),
    );
    draw.stroke_line(
        Point::new(bounds.x0, bounds.y1),
        Point::new(bounds.x1, bounds.y1),
        &Brush::from(colors.on_surface_variant.peniko()),
        INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT,
    );
}

pub fn draw_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(
        draw,
        bounds,
        RoundedRectRadii::new(
            INPUT_FILLED_CONTAINER_TOP_RADIUS,
            INPUT_FILLED_CONTAINER_TOP_RADIUS,
            0.0,
            0.0,
        ),
        colors.on_surface.peniko(),
        state,
    );
}

#[cfg(test)]
mod tests {
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};

    use super::{MaterialColorScheme, draw_field, metrics};
    use crate::dimensions::{
        INPUT_FIELD_MIN_HEIGHT, INPUT_FIELD_MIN_WIDTH, INPUT_FILLED_ACTIVE_INDICATOR_HEIGHT,
        INPUT_FILLED_CONTAINER_TOP_RADIUS,
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
        draw_field(&colors, &mut draw, Rect::new(0.0, 0.0, 120.0, 56.0));

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
}
