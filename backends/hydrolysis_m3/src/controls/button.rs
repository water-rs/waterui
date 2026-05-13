use crate::dimensions::{
    BUTTON_CONTAINER_RADIUS, BUTTON_LINK_HORIZONTAL_PADDING, BUTTON_LINK_UNDERLINE_BOTTOM_INSET,
    BUTTON_LINK_UNDERLINE_THICKNESS, BUTTON_LINK_VERTICAL_PADDING, BUTTON_MIN_HEIGHT,
    BUTTON_MIN_WIDTH, BUTTON_TEXT_HORIZONTAL_PADDING, BUTTON_TEXT_VERTICAL_PADDING,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, ButtonMetrics, DrawContext, WidgetInteractionState};
use waterui_controls::button::ButtonStyle;
use waterui_graphics::color::Color;

pub fn metrics(style: ButtonStyle) -> ButtonMetrics {
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered => {
            ButtonMetrics::new(24.0, 10.0, BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT)
        }
        ButtonStyle::Plain | ButtonStyle::Borderless => ButtonMetrics::new(
            BUTTON_TEXT_HORIZONTAL_PADDING,
            BUTTON_TEXT_VERTICAL_PADDING,
            BUTTON_MIN_WIDTH,
            BUTTON_MIN_HEIGHT,
        ),
        ButtonStyle::Link => ButtonMetrics::new(
            BUTTON_LINK_HORIZONTAL_PADDING,
            BUTTON_LINK_VERTICAL_PADDING,
            0.0,
            0.0,
        ),
        ButtonStyle::BorderedProminent => {
            ButtonMetrics::new(24.0, 10.0, BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT)
        }
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::{draw_chrome, metrics};
    use crate::dimensions::{
        BUTTON_CONTAINER_RADIUS, BUTTON_MIN_HEIGHT, BUTTON_MIN_WIDTH,
        BUTTON_TEXT_HORIZONTAL_PADDING, BUTTON_TEXT_VERTICAL_PADDING,
    };
    use crate::{Brush, DrawContext, MaterialColorScheme};
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
    use waterui_controls::button::ButtonStyle;

    fn assert_text_button_metrics(style: ButtonStyle) {
        let metrics = metrics(style);

        assert_eq!(metrics.padding_x, BUTTON_TEXT_HORIZONTAL_PADDING);
        assert_eq!(metrics.padding_y, BUTTON_TEXT_VERTICAL_PADDING);
        assert_eq!(metrics.min_width, BUTTON_MIN_WIDTH);
        assert_eq!(metrics.min_height, BUTTON_MIN_HEIGHT);
    }

    #[test]
    fn text_button_styles_match_material_web_latest_medium_container_metrics() {
        assert_text_button_metrics(ButtonStyle::Plain);
        assert_text_button_metrics(ButtonStyle::Borderless);
        assert_eq!(BUTTON_MIN_HEIGHT, 56.0);
        assert_eq!(BUTTON_CONTAINER_RADIUS, 28.0);
        assert_eq!(BUTTON_TEXT_HORIZONTAL_PADDING, 24.0);
    }

    #[derive(Default)]
    struct RecordingDrawContext {
        rounded_strokes: Vec<(Rect, RoundedRectRadii, Brush, f64)>,
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}

        fn fill_rounded_rect(&mut self, _rect: Rect, _radii: RoundedRectRadii, _brush: &Brush) {}

        fn stroke_rect(&mut self, _rect: Rect, _brush: &Brush, _width: f64) {}

        fn stroke_rounded_rect(
            &mut self,
            rect: Rect,
            radii: RoundedRectRadii,
            brush: &Brush,
            width: f64,
        ) {
            self.rounded_strokes
                .push((rect, radii, brush.clone(), width));
        }

        fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, _width: f64) {}

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
    fn outlined_button_uses_latest_material_outline_role() {
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();

        draw_chrome(
            &colors,
            &mut draw,
            Rect::new(0.0, 0.0, 120.0, BUTTON_MIN_HEIGHT),
            ButtonStyle::Bordered,
        );

        assert_eq!(draw.rounded_strokes.len(), 1);
        assert!(matches!(
            &draw.rounded_strokes[0].2,
            Brush::Solid(color) if *color == colors.outline_variant.peniko()
        ));
        assert_eq!(draw.rounded_strokes[0].3, 1.0);
    }
}

pub fn label_color(colors: &MaterialColorScheme, style: ButtonStyle) -> Option<Color> {
    match style {
        ButtonStyle::BorderedProminent => Some(colors.on_primary.view_color()),
        ButtonStyle::Automatic => Some(colors.on_secondary_container.view_color()),
        ButtonStyle::Bordered => Some(colors.on_surface_variant.view_color()),
        ButtonStyle::Plain | ButtonStyle::Link | ButtonStyle::Borderless => {
            Some(colors.primary.view_color())
        }
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn draw_chrome(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    style: ButtonStyle,
) {
    match style {
        ButtonStyle::Automatic => {
            draw.fill_rounded_rect(
                bounds,
                BUTTON_CONTAINER_RADIUS.into(),
                &Brush::from(colors.secondary_container.peniko()),
            );
        }
        ButtonStyle::Bordered => {
            draw.stroke_rounded_rect(
                bounds,
                BUTTON_CONTAINER_RADIUS.into(),
                &Brush::from(colors.outline_variant.peniko()),
                1.0,
            );
        }
        ButtonStyle::BorderedProminent => {
            draw.fill_rounded_rect(
                bounds,
                BUTTON_CONTAINER_RADIUS.into(),
                &Brush::from(colors.primary.peniko()),
            );
        }
        ButtonStyle::Link => {
            let underline_y = (bounds.y1 - BUTTON_LINK_UNDERLINE_BOTTOM_INSET).max(bounds.y0);
            draw.stroke_line(
                vello::kurbo::Point::new(bounds.x0 + BUTTON_LINK_HORIZONTAL_PADDING, underline_y),
                vello::kurbo::Point::new(bounds.x1 - BUTTON_LINK_HORIZONTAL_PADDING, underline_y),
                &Brush::from(colors.primary.peniko()),
                BUTTON_LINK_UNDERLINE_THICKNESS,
            );
        }
        ButtonStyle::Plain | ButtonStyle::Borderless => {}
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn draw_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    style: ButtonStyle,
    state: WidgetInteractionState,
) {
    let color = match style {
        ButtonStyle::BorderedProminent => colors.on_primary.peniko(),
        ButtonStyle::Automatic => colors.on_secondary_container.peniko(),
        ButtonStyle::Bordered => colors.on_surface_variant.peniko(),
        ButtonStyle::Link | ButtonStyle::Plain | ButtonStyle::Borderless => colors.primary.peniko(),
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    };
    state_layer::draw_bounded(draw, bounds, BUTTON_CONTAINER_RADIUS.into(), color, state);
}
