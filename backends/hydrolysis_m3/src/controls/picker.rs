use crate::dimensions::{
    PICKER_HORIZONTAL_INSET, PICKER_INDICATOR_SPACE, PICKER_MENU_POPUP_CORNER_RADIUS,
    PICKER_MENU_POPUP_TOP_SPACING, PICKER_MIN_HEIGHT, PICKER_MIN_WIDTH,
    PICKER_RADIO_INDICATOR_SIZE, PICKER_RADIO_LABEL_SPACING, PICKER_RADIO_ROW_SPACING,
    PICKER_VERTICAL_INSET,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, PickerMetrics, WidgetInteractionState};
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

pub fn draw_indicator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    let center_x = PICKER_INDICATOR_SPACE.mul_add(-0.5, bounds.x1 - PICKER_HORIZONTAL_INSET);
    let center_y = bounds.height().mul_add(0.5, bounds.y0);
    let chevron = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(center_x - 4.0, center_y - 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x, center_y + 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x + 4.0, center_y - 2.0)),
    ]);
    draw.stroke_path(
        &chevron,
        &Brush::from(colors.on_surface_variant.peniko()),
        1.5,
    );
}

pub fn draw_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(draw, bounds, 4.0.into(), colors.on_surface.peniko(), state);
}

pub fn draw_popup(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    popup_rect: vello::kurbo::Rect,
) {
    draw.fill_rounded_rect(
        popup_rect,
        PICKER_MENU_POPUP_CORNER_RADIUS.into(),
        &Brush::from(colors.surface_container.peniko()),
    );
    draw.stroke_rounded_rect(
        popup_rect,
        PICKER_MENU_POPUP_CORNER_RADIUS.into(),
        &Brush::from(colors.outline_variant.peniko()),
        1.0,
    );
}

pub fn draw_popup_row_background(
    colors: &MaterialColorScheme,
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
    draw.fill_rect(inset, &Brush::from(colors.primary_container.peniko()));
}

pub fn draw_popup_row_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    row_rect: vello::kurbo::Rect,
    selected: bool,
    state: WidgetInteractionState,
) {
    let inset = vello::kurbo::Rect::new(
        row_rect.x0 + 2.0,
        row_rect.y0 + 1.0,
        row_rect.x1 - 2.0,
        row_rect.y1 - 1.0,
    );
    state_layer::draw_bounded(
        draw,
        inset,
        0.0.into(),
        if selected {
            colors.on_primary_container.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}

pub fn draw_separator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    separator: vello::kurbo::Rect,
) {
    draw.fill_rect(separator, &Brush::from(colors.outline_variant.peniko()));
}

pub fn draw_radio_indicator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    selected: bool,
) {
    draw.fill_circle(center, radius, &Brush::from(colors.surface.peniko()));
    draw.stroke_circle(
        center,
        radius,
        &Brush::from(if selected {
            colors.primary.peniko()
        } else {
            colors.on_surface_variant.peniko()
        }),
        1.0,
    );
    if selected {
        draw.fill_circle(center, radius * 0.45, &Brush::from(colors.primary.peniko()));
    }
}

pub fn draw_radio_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    _radius: f64,
    selected: bool,
    state: WidgetInteractionState,
) {
    state_layer::draw_unbounded_circle(
        draw,
        center,
        20.0,
        if selected {
            colors.primary.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}
