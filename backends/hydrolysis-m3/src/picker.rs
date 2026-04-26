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

pub fn metrics(style: PickerStyle) -> PickerMetrics {
    match style {
        PickerStyle::Automatic | PickerStyle::Menu | PickerStyle::Radio => material_metrics(),
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
    }
}

const fn material_metrics() -> PickerMetrics {
    PickerMetrics {
        min_width: PICKER_MIN_WIDTH,
        min_height: PICKER_MIN_HEIGHT,
        horizontal_inset: PICKER_HORIZONTAL_INSET,
        vertical_inset: PICKER_VERTICAL_INSET,
        indicator_space: PICKER_INDICATOR_SPACE,
        radio_indicator_size: PICKER_RADIO_INDICATOR_SIZE,
        radio_label_spacing: PICKER_RADIO_LABEL_SPACING,
        radio_row_spacing: PICKER_RADIO_ROW_SPACING,
        popup_top_spacing: PICKER_MENU_POPUP_TOP_SPACING,
        popup_corner_radius: PICKER_MENU_POPUP_CORNER_RADIUS,
    }
}

pub fn draw_indicator(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    let center_x = PICKER_INDICATOR_SPACE.mul_add(-0.5, bounds.x1 - PICKER_HORIZONTAL_INSET);
    let center_y = bounds.height().mul_add(0.5, bounds.y0);
    let chevron = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(center_x - 4.0, center_y - 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x, center_y + 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x + 4.0, center_y - 2.0)),
    ]);
    draw.stroke_path(&chevron, &Brush::from(FOREGROUND_MUTED), 1.5);
}

pub fn draw_popup(draw: &mut dyn DrawContext, popup_rect: vello::kurbo::Rect) {
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

pub fn draw_popup_row_background(
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

pub fn draw_separator(draw: &mut dyn DrawContext, separator: vello::kurbo::Rect) {
    draw.fill_rect(separator, &Brush::from(OUTLINE_SUBTLE));
}

pub fn draw_radio_indicator(
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
