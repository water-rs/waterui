use crate::dimensions::{
    BUTTON_CONTAINER_RADIUS, BUTTON_LINK_HORIZONTAL_PADDING, BUTTON_LINK_UNDERLINE_BOTTOM_INSET,
    BUTTON_LINK_UNDERLINE_THICKNESS, BUTTON_LINK_VERTICAL_PADDING, BUTTON_MIN_WIDTH,
    BUTTON_TEXT_VERTICAL_PADDING, button_size_tokens,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, ButtonMetrics, DrawContext, WidgetInteractionState};
use waterui_controls::button::{ButtonSize, ButtonStyle};
use waterui_graphics::color::Color;

pub fn metrics(style: ButtonStyle, size: ButtonSize) -> ButtonMetrics {
    let tokens = button_size_tokens(size);
    match style {
        ButtonStyle::Automatic
        | ButtonStyle::Bordered
        | ButtonStyle::BorderedProminent
        | ButtonStyle::Plain
        | ButtonStyle::Borderless => ButtonMetrics::new(
            tokens.horizontal_space,
            BUTTON_TEXT_VERTICAL_PADDING,
            BUTTON_MIN_WIDTH,
            tokens.container_height,
        ),
        // A link has no container, so it takes neither the size scale's
        // padding nor its minimum box.
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
            } else if state.focus_visible {
                colors.primary.peniko()
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
        BUTTON_CONTAINER_RADIUS, BUTTON_EXTRA_LARGE, BUTTON_EXTRA_SMALL, BUTTON_LARGE,
        BUTTON_MEDIUM, BUTTON_MIN_WIDTH, BUTTON_SMALL, BUTTON_TEXT_VERTICAL_PADDING,
    };
    use crate::{Brush, DrawContext, MaterialColorScheme};
    use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
    use waterui_controls::button::{ButtonSize, ButtonStyle};

    fn assert_button_metrics(style: ButtonStyle, expected_padding_x: f64) {
        let metrics = metrics(style, ButtonSize::Small);

        assert_eq!(metrics.padding_x, expected_padding_x);
        assert_eq!(metrics.padding_y, BUTTON_TEXT_VERTICAL_PADDING);
        assert_eq!(metrics.min_width, BUTTON_MIN_WIDTH);
        assert_eq!(metrics.min_height, BUTTON_SMALL.container_height);
    }

    /// The Expressive size scale, from Compose's `ButtonXSmallTokens` through
    /// `ButtonXLargeTokens`. Height, padding, icon size and corner shape all
    /// move together, so a size is checked as a whole token set.
    #[test]
    fn button_size_scale_matches_compose_button_size_tokens() {
        // ButtonXSmallTokens
        assert_eq!(BUTTON_EXTRA_SMALL.container_height, 32.0);
        assert_eq!(BUTTON_EXTRA_SMALL.horizontal_space, 16.0);
        assert_eq!(BUTTON_EXTRA_SMALL.icon_size, 20.0);
        assert_eq!(BUTTON_EXTRA_SMALL.icon_label_space, 8.0);
        // ButtonSmallTokens
        assert_eq!(BUTTON_SMALL.container_height, 40.0);
        assert_eq!(BUTTON_SMALL.icon_size, 20.0);
        // ButtonMediumTokens
        assert_eq!(BUTTON_MEDIUM.container_height, 56.0);
        assert_eq!(BUTTON_MEDIUM.horizontal_space, 24.0);
        assert_eq!(BUTTON_MEDIUM.icon_size, 24.0);
        // ButtonLargeTokens
        assert_eq!(BUTTON_LARGE.container_height, 96.0);
        assert_eq!(BUTTON_LARGE.horizontal_space, 48.0);
        assert_eq!(BUTTON_LARGE.icon_size, 32.0);
        assert_eq!(BUTTON_LARGE.icon_label_space, 12.0);
        assert_eq!(BUTTON_LARGE.outline_width, 2.0);
        // ButtonXLargeTokens
        assert_eq!(BUTTON_EXTRA_LARGE.container_height, 136.0);
        assert_eq!(BUTTON_EXTRA_LARGE.horizontal_space, 64.0);
        assert_eq!(BUTTON_EXTRA_LARGE.icon_size, 40.0);
        assert_eq!(BUTTON_EXTRA_LARGE.icon_label_space, 16.0);

        // The scale is monotonic in every dimension it changes.
        let scale = [
            BUTTON_EXTRA_SMALL,
            BUTTON_SMALL,
            BUTTON_MEDIUM,
            BUTTON_LARGE,
            BUTTON_EXTRA_LARGE,
        ];
        for pair in scale.windows(2) {
            let (smaller, larger) = (pair[0], pair[1]);
            assert!(larger.container_height > smaller.container_height);
            assert!(larger.horizontal_space >= smaller.horizontal_space);
            assert!(larger.icon_size >= smaller.icon_size);
        }
    }

    /// A button's measured height follows the size it was given.
    #[test]
    fn button_metrics_follow_the_requested_size() {
        for (size, tokens) in [
            (ButtonSize::ExtraSmall, BUTTON_EXTRA_SMALL),
            (ButtonSize::Small, BUTTON_SMALL),
            (ButtonSize::Medium, BUTTON_MEDIUM),
            (ButtonSize::Large, BUTTON_LARGE),
            (ButtonSize::ExtraLarge, BUTTON_EXTRA_LARGE),
        ] {
            let metrics = metrics(ButtonStyle::BorderedProminent, size);
            assert_eq!(metrics.min_height, tokens.container_height, "{size:?}");
            assert_eq!(metrics.padding_x, tokens.horizontal_space, "{size:?}");
        }
    }

    /// Values from `androidx.compose.material3`: `BaselineButtonTokens` for the
    /// container, `ButtonDefaults` for the minimum size and content padding.
    #[test]
    fn button_metrics_match_compose_button_tokens() {
        for style in [
            ButtonStyle::Plain,
            ButtonStyle::Borderless,
            ButtonStyle::Automatic,
            ButtonStyle::Bordered,
            ButtonStyle::BorderedProminent,
        ] {
            assert_button_metrics(style, BUTTON_SMALL.horizontal_space);
        }
        // ButtonSmallTokens.ContainerHeight
        assert_eq!(BUTTON_SMALL.container_height, 40.0);
        // BaselineButtonTokens.ContainerShapeRound is CornerFull
        assert_eq!(BUTTON_CONTAINER_RADIUS, BUTTON_SMALL.container_height / 2.0);
        // ButtonDefaults.MinWidth
        assert_eq!(BUTTON_MIN_WIDTH, 58.0);
        // BaselineButtonTokens.LeadingSpace / TrailingSpace, which is what
        // `ButtonDefaults.ContentPadding` uses for the default button.
        assert_eq!(BUTTON_SMALL.horizontal_space, 24.0);
        // ButtonDefaults.ContentPadding vertical
        assert_eq!(BUTTON_TEXT_VERTICAL_PADDING, 8.0);
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
            Rect::new(0.0, 0.0, 120.0, BUTTON_SMALL.container_height),
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
            Rect::new(0.0, 0.0, 120.0, BUTTON_SMALL.container_height),
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
