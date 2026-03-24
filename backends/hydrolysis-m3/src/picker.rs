use crate::colors::{
    ACCENT, ACCENT_SELECTION, FOREGROUND_MUTED, OUTLINE_SUBTLE, SURFACE_DEFAULT, SURFACE_MUTED,
};
use crate::dimensions::{
    PICKER_HORIZONTAL_INSET, PICKER_INDICATOR_SPACE, PICKER_MENU_POPUP_CORNER_RADIUS,
    PICKER_MENU_POPUP_TOP_SPACING, PICKER_MIN_HEIGHT, PICKER_MIN_WIDTH,
    PICKER_RADIO_INDICATOR_SIZE, PICKER_RADIO_LABEL_SPACING, PICKER_RADIO_ROW_SPACING,
    PICKER_VERTICAL_INSET,
};
use crate::{Brush, DrawContext, PickerMetrics};
use waterui_form::picker::PickerStyle;

pub(crate) fn metrics(style: PickerStyle) -> PickerMetrics {
    match style {
        PickerStyle::Automatic | PickerStyle::Menu => PickerMetrics::new(
            PICKER_MIN_WIDTH,
            PICKER_MIN_HEIGHT,
            PICKER_HORIZONTAL_INSET,
            PICKER_VERTICAL_INSET,
            PICKER_INDICATOR_SPACE,
            PICKER_RADIO_INDICATOR_SIZE,
            PICKER_RADIO_LABEL_SPACING,
            PICKER_RADIO_ROW_SPACING,
            PICKER_MENU_POPUP_TOP_SPACING,
            PICKER_MENU_POPUP_CORNER_RADIUS,
        ),
        PickerStyle::Radio => PickerMetrics::new(
            PICKER_MIN_WIDTH,
            PICKER_MIN_HEIGHT,
            PICKER_HORIZONTAL_INSET,
            PICKER_VERTICAL_INSET,
            PICKER_INDICATOR_SPACE,
            PICKER_RADIO_INDICATOR_SIZE,
            PICKER_RADIO_LABEL_SPACING,
            PICKER_RADIO_ROW_SPACING,
            PICKER_MENU_POPUP_TOP_SPACING,
            PICKER_MENU_POPUP_CORNER_RADIUS,
        ),
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}

pub(crate) fn draw_indicator(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    let center_x = bounds.x1 - PICKER_HORIZONTAL_INSET - PICKER_INDICATOR_SPACE * 0.5;
    let center_y = bounds.y0 + bounds.height() * 0.5;
    let chevron = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(center_x - 4.0, center_y - 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x, center_y + 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x + 4.0, center_y - 2.0)),
    ]);
    draw.stroke_path(&chevron, &Brush::from(FOREGROUND_MUTED), 1.5);
}

pub(crate) fn draw_popup(draw: &mut dyn DrawContext, popup_rect: vello::kurbo::Rect) {
    draw.fill_rounded_rect(
        popup_rect,
        PICKER_MENU_POPUP_CORNER_RADIUS.into(),
        &Brush::from(SURFACE_MUTED),
    );
    draw.stroke_rounded_rect(
        popup_rect,
        PICKER_MENU_POPUP_CORNER_RADIUS.into(),
        &Brush::from(OUTLINE_SUBTLE),
        1.0,
    );
}

pub(crate) fn draw_popup_row_background(
    draw: &mut dyn DrawContext,
    row_rect: vello::kurbo::Rect,
    selected: bool,
) {
    if !selected {
        return;
    }
    let inset = vello::kurbo::Rect::new(
        row_rect.x0 + 2.0,
        row_rect.y0 + 1.0,
        row_rect.x1 - 2.0,
        row_rect.y1 - 1.0,
    );
    draw.fill_rect(inset, &Brush::from(ACCENT_SELECTION));
}

pub(crate) fn draw_separator(draw: &mut dyn DrawContext, separator: vello::kurbo::Rect) {
    draw.fill_rect(separator, &Brush::from(OUTLINE_SUBTLE));
}

pub(crate) fn draw_radio_indicator(
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    selected: bool,
) {
    draw.fill_circle(center, radius, &Brush::from(SURFACE_DEFAULT));
    draw.stroke_circle(
        center,
        radius,
        &Brush::from(if selected { ACCENT } else { OUTLINE_SUBTLE }),
        1.0,
    );
    if selected {
        draw.fill_circle(center, radius * 0.45, &Brush::from(ACCENT));
    }
}
