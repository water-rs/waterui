use crate::dimensions::{
    BUTTON_LINK_HORIZONTAL_PADDING, BUTTON_LINK_UNDERLINE_BOTTOM_INSET,
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
    use super::metrics;
    use crate::dimensions::{
        BUTTON_MIN_HEIGHT, BUTTON_MIN_WIDTH, BUTTON_TEXT_HORIZONTAL_PADDING,
        BUTTON_TEXT_VERTICAL_PADDING,
    };
    use waterui_controls::button::ButtonStyle;

    fn assert_text_button_metrics(style: ButtonStyle) {
        let metrics = metrics(style);

        assert_eq!(metrics.padding_x, BUTTON_TEXT_HORIZONTAL_PADDING);
        assert_eq!(metrics.padding_y, BUTTON_TEXT_VERTICAL_PADDING);
        assert_eq!(metrics.min_width, BUTTON_MIN_WIDTH);
        assert_eq!(metrics.min_height, BUTTON_MIN_HEIGHT);
    }

    #[test]
    fn text_button_styles_match_material_web_v0_192_container_metrics() {
        assert_text_button_metrics(ButtonStyle::Plain);
        assert_text_button_metrics(ButtonStyle::Borderless);
    }
}

pub fn label_color(colors: &MaterialColorScheme, style: ButtonStyle) -> Option<Color> {
    match style {
        ButtonStyle::BorderedProminent => Some(colors.on_primary.view_color()),
        ButtonStyle::Automatic => Some(colors.on_secondary_container.view_color()),
        ButtonStyle::Bordered
        | ButtonStyle::Plain
        | ButtonStyle::Link
        | ButtonStyle::Borderless => Some(colors.primary.view_color()),
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
                20.0.into(),
                &Brush::from(colors.secondary_container.peniko()),
            );
        }
        ButtonStyle::Bordered => {
            draw.stroke_rounded_rect(
                bounds,
                20.0.into(),
                &Brush::from(colors.outline.peniko()),
                1.0,
            );
        }
        ButtonStyle::BorderedProminent => {
            draw.fill_rounded_rect(bounds, 20.0.into(), &Brush::from(colors.primary.peniko()));
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
        ButtonStyle::Bordered
        | ButtonStyle::Link
        | ButtonStyle::Plain
        | ButtonStyle::Borderless => colors.primary.peniko(),
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    };
    state_layer::draw_bounded(draw, bounds, 20.0.into(), color, state);
}
