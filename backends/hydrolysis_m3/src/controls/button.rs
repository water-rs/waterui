use crate::colors::{
    ACCENT, ACCENT_STRONG, FOREGROUND_STRONG, OUTLINE_STRONG, SURFACE_DEFAULT, SURFACE_SUBTLE,
};
use crate::dimensions::{
    BUTTON_LINK_HORIZONTAL_PADDING, BUTTON_LINK_UNDERLINE_BOTTOM_INSET,
    BUTTON_LINK_UNDERLINE_THICKNESS, BUTTON_LINK_VERTICAL_PADDING, BUTTON_MIN_HEIGHT,
    BUTTON_MIN_WIDTH,
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
        ButtonStyle::Plain => ButtonMetrics::new(0.0, 0.0, 0.0, 0.0),
        ButtonStyle::Link => ButtonMetrics::new(
            BUTTON_LINK_HORIZONTAL_PADDING,
            BUTTON_LINK_VERTICAL_PADDING,
            0.0,
            0.0,
        ),
        ButtonStyle::Borderless => ButtonMetrics::new(12.0, 10.0, 0.0, BUTTON_MIN_HEIGHT),
        ButtonStyle::BorderedProminent => {
            ButtonMetrics::new(24.0, 10.0, BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT)
        }
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn label_color(colors: &MaterialColorScheme, style: ButtonStyle) -> Option<Color> {
    match style {
        ButtonStyle::BorderedProminent => Some(colors.on_primary.view_color()),
        ButtonStyle::Automatic
        | ButtonStyle::Bordered
        | ButtonStyle::Plain
        | ButtonStyle::Link
        | ButtonStyle::Borderless => None,
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn draw_chrome(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect, style: ButtonStyle) {
    match style {
        ButtonStyle::Automatic => {
            draw.fill_rounded_rect(bounds, 20.0.into(), &Brush::from(SURFACE_SUBTLE));
        }
        ButtonStyle::Bordered => {
            draw.fill_rounded_rect(bounds, 20.0.into(), &Brush::from(SURFACE_DEFAULT));
            draw.stroke_rounded_rect(bounds, 20.0.into(), &Brush::from(OUTLINE_STRONG), 1.0);
        }
        ButtonStyle::BorderedProminent => {
            draw.fill_rounded_rect(bounds, 20.0.into(), &Brush::from(ACCENT));
            draw.stroke_rounded_rect(bounds, 20.0.into(), &Brush::from(ACCENT_STRONG), 1.0);
        }
        ButtonStyle::Link => {
            let underline_y = (bounds.y1 - BUTTON_LINK_UNDERLINE_BOTTOM_INSET).max(bounds.y0);
            draw.stroke_line(
                vello::kurbo::Point::new(bounds.x0 + BUTTON_LINK_HORIZONTAL_PADDING, underline_y),
                vello::kurbo::Point::new(bounds.x1 - BUTTON_LINK_HORIZONTAL_PADDING, underline_y),
                &Brush::from(ACCENT),
                BUTTON_LINK_UNDERLINE_THICKNESS,
            );
        }
        ButtonStyle::Plain | ButtonStyle::Borderless => {}
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn draw_state_layer(
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    style: ButtonStyle,
    state: WidgetInteractionState,
) {
    let color = match style {
        ButtonStyle::BorderedProminent => SURFACE_DEFAULT,
        ButtonStyle::Automatic | ButtonStyle::Bordered | ButtonStyle::Link => ACCENT,
        ButtonStyle::Plain | ButtonStyle::Borderless => FOREGROUND_STRONG,
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    };
    state_layer::draw_bounded(draw, bounds, 20.0.into(), color, state);
}
