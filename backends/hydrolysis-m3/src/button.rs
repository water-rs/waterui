use crate::colors::{ACCENT, ACCENT_STRONG, OUTLINE_STRONG, SURFACE_DEFAULT, SURFACE_SUBTLE};
use crate::dimensions::{
    BUTTON_LINK_HORIZONTAL_PADDING, BUTTON_LINK_UNDERLINE_BOTTOM_INSET,
    BUTTON_LINK_UNDERLINE_THICKNESS, BUTTON_LINK_VERTICAL_PADDING, BUTTON_MIN_HEIGHT,
    BUTTON_MIN_WIDTH,
};
use crate::{Brush, ButtonMetrics, DrawContext};
use waterui_controls::button::ButtonStyle;

pub fn metrics(style: ButtonStyle) -> ButtonMetrics {
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered => {
            ButtonMetrics::new(8.0, 4.0, BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT)
        }
        ButtonStyle::Plain => ButtonMetrics::new(0.0, 0.0, 0.0, 0.0),
        ButtonStyle::Link => ButtonMetrics::new(
            BUTTON_LINK_HORIZONTAL_PADDING,
            BUTTON_LINK_VERTICAL_PADDING,
            0.0,
            0.0,
        ),
        ButtonStyle::Borderless => ButtonMetrics::new(4.0, 2.0, 0.0, 0.0),
        ButtonStyle::BorderedProminent => {
            ButtonMetrics::new(10.0, 5.0, BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT)
        }
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

pub fn draw_chrome(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect, style: ButtonStyle) {
    match style {
        ButtonStyle::Automatic => {
            draw.fill_rounded_rect(bounds, 6.0.into(), &Brush::from(SURFACE_SUBTLE));
            draw.stroke_rounded_rect(bounds, 6.0.into(), &Brush::from(OUTLINE_STRONG), 1.0);
        }
        ButtonStyle::Bordered => {
            draw.fill_rounded_rect(bounds, 6.0.into(), &Brush::from(SURFACE_DEFAULT));
            draw.stroke_rounded_rect(bounds, 6.0.into(), &Brush::from(OUTLINE_STRONG), 1.0);
        }
        ButtonStyle::BorderedProminent => {
            draw.fill_rounded_rect(bounds, 6.0.into(), &Brush::from(ACCENT));
            draw.stroke_rounded_rect(bounds, 6.0.into(), &Brush::from(ACCENT_STRONG), 1.0);
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
