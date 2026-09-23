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
        ButtonStyle::Automatic | ButtonStyle::Bordered | ButtonStyle::BorderedProminent => {
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
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn label_color(colors: &MaterialColorScheme, style: ButtonStyle, disabled: bool) -> Color {
    // MD3 disabled button label: on-surface at the 38% disabled-content
    // opacity, regardless of variant.
    if disabled {
        return colors
            .on_surface
            .view_color()
            .with_opacity(crate::theme::colors::DISABLED_CONTENT_OPACITY);
    }
    match style {
        ButtonStyle::BorderedProminent => colors.on_primary.view_color(),
        ButtonStyle::Automatic => colors.on_secondary_container.view_color(),
        ButtonStyle::Bordered
        | ButtonStyle::Plain
        | ButtonStyle::Link
        | ButtonStyle::Borderless => colors.primary.view_color(),
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn draw_chrome(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    style: ButtonStyle,
    state: WidgetInteractionState,
) {
    // MD3 disabled button: filled/tonal containers drop to on-surface at 12%,
    // the outlined border drops to on-surface at 12%, and the link underline
    // follows the disabled label (on-surface at 38%). Text buttons have no
    // container to dim.
    match style {
        ButtonStyle::Automatic => {
            let fill = if state.disabled {
                colors.on_surface.peniko_disabled_container()
            } else {
                colors.secondary_container.peniko()
            };
            draw.fill_rounded_rect(bounds, BUTTON_CONTAINER_RADIUS.into(), &Brush::from(fill));
        }
        ButtonStyle::Bordered => {
            let border = if state.disabled {
                colors.on_surface.peniko_disabled_container()
            } else {
                colors.outline.peniko()
            };
            draw.stroke_rounded_rect(
                bounds,
                BUTTON_CONTAINER_RADIUS.into(),
                &Brush::from(border),
                1.0,
            );
        }
        ButtonStyle::BorderedProminent => {
            let fill = if state.disabled {
                colors.on_surface.peniko_disabled_container()
            } else {
                colors.primary.peniko()
            };
            draw.fill_rounded_rect(bounds, BUTTON_CONTAINER_RADIUS.into(), &Brush::from(fill));
        }
        ButtonStyle::Link => {
            let underline = if state.disabled {
                colors.on_surface.peniko_disabled_content()
            } else {
                colors.primary.peniko()
            };
            let underline_y = (bounds.y1 - BUTTON_LINK_UNDERLINE_BOTTOM_INSET).max(bounds.y0);
            draw.stroke_line(
                vello::kurbo::Point::new(bounds.x0 + BUTTON_LINK_HORIZONTAL_PADDING, underline_y),
                vello::kurbo::Point::new(bounds.x1 - BUTTON_LINK_HORIZONTAL_PADDING, underline_y),
                &Brush::from(underline),
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
        ButtonStyle::Bordered
        | ButtonStyle::Link
        | ButtonStyle::Plain
        | ButtonStyle::Borderless => colors.primary.peniko(),
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    };
    state_layer::draw_bounded(draw, bounds, BUTTON_CONTAINER_RADIUS.into(), color, state);
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

    fn assert_button_metrics(style: ButtonStyle, expected_padding_x: f64) {
        let metrics = metrics(style);

        assert_eq!(metrics.padding_x, expected_padding_x);
        assert_eq!(metrics.padding_y, BUTTON_TEXT_VERTICAL_PADDING);
        assert_eq!(metrics.min_width, BUTTON_MIN_WIDTH);
        assert_eq!(metrics.min_height, BUTTON_MIN_HEIGHT);
    }

    #[test]
    fn button_metrics_match_material_web_v0_192_tokens() {
        assert_button_metrics(ButtonStyle::Plain, BUTTON_TEXT_HORIZONTAL_PADDING);
        assert_button_metrics(ButtonStyle::Borderless, BUTTON_TEXT_HORIZONTAL_PADDING);
        assert_button_metrics(ButtonStyle::Automatic, 24.0);
        assert_button_metrics(ButtonStyle::Bordered, 24.0);
        assert_button_metrics(ButtonStyle::BorderedProminent, 24.0);
        assert_eq!(BUTTON_MIN_HEIGHT, 40.0);
        assert_eq!(BUTTON_CONTAINER_RADIUS, 20.0);
        assert_eq!(BUTTON_TEXT_HORIZONTAL_PADDING, 12.0);
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
    fn disabled_outlined_button_uses_disabled_border() {
        // MD3 disabled outlined button: the border drops to on-surface at 12%.
        let colors = MaterialColorScheme::baseline_light();
        let mut draw = RecordingDrawContext::default();

        draw_chrome(
            &colors,
            &mut draw,
            Rect::new(0.0, 0.0, 120.0, BUTTON_MIN_HEIGHT),
            ButtonStyle::Bordered,
            crate::WidgetInteractionState {
                disabled: true,
                ..crate::WidgetInteractionState::NONE
            },
        );

        assert_eq!(draw.rounded_strokes.len(), 1);
        assert!(matches!(
            &draw.rounded_strokes[0].2,
            Brush::Solid(color) if *color == colors.on_surface.peniko_disabled_container()
        ));
    }

    #[test]
    fn disabled_button_label_color_is_on_surface_38() {
        // MD3 disabled button label: on-surface at 38% for every variant.
        use waterui_core::Signal as _;
        let colors = MaterialColorScheme::baseline_light();
        for style in [
            ButtonStyle::Automatic,
            ButtonStyle::Bordered,
            ButtonStyle::BorderedProminent,
            ButtonStyle::Plain,
        ] {
            let color = super::label_color(&colors, style, true);
            let resolved = color.resolve(&waterui_core::Environment::new()).get();
            let expected = colors
                .on_surface
                .view_color()
                .with_opacity(crate::theme::colors::DISABLED_CONTENT_OPACITY)
                .resolve(&waterui_core::Environment::new())
                .get();
            assert_eq!(
                (
                    resolved.red,
                    resolved.green,
                    resolved.blue,
                    resolved.opacity
                ),
                (
                    expected.red,
                    expected.green,
                    expected.blue,
                    expected.opacity
                ),
                "style {style:?}"
            );
        }
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
            crate::WidgetInteractionState::NONE,
        );

        assert_eq!(draw.rounded_strokes.len(), 1);
        assert!(matches!(
            &draw.rounded_strokes[0].2,
            Brush::Solid(color) if *color == colors.outline.peniko()
        ));
        assert_eq!(draw.rounded_strokes[0].3, 1.0);
    }
}
