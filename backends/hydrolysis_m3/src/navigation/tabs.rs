use crate::dimensions::TABS_HIGHLIGHT_CORNER_RADIUS;
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, WidgetInteractionState};
use vello::kurbo::Rect;

pub fn draw_bar(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    top_edge: bool,
) {
    draw.fill_rect(bounds, &Brush::from(colors.surface_container.peniko()));
    let separator = if top_edge {
        Rect::new(bounds.x0, bounds.y1 - 1.0, bounds.x1, bounds.y1)
    } else {
        Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + 1.0)
    };
    draw.fill_rect(separator, &Brush::from(colors.outline_variant.peniko()));
}

pub fn draw_highlight(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rounded_rect(
        bounds,
        TABS_HIGHLIGHT_CORNER_RADIUS.into(),
        &Brush::from(colors.primary_container.peniko()),
    );
}

pub fn draw_button_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    selected: bool,
    state: WidgetInteractionState,
) {
    let layer = Rect::new(
        bounds.x0 + 4.0,
        bounds.y0 + 6.0,
        bounds.x1 - 4.0,
        bounds.y1 - 6.0,
    );
    state_layer::draw_bounded(
        draw,
        layer,
        TABS_HIGHLIGHT_CORNER_RADIUS.into(),
        if selected {
            colors.on_primary_container.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}
