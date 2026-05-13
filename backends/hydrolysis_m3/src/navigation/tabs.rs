use crate::dimensions::{
    TABS_ACTIVE_INDICATOR_HEIGHT, TABS_ACTIVE_INDICATOR_RADIUS, TABS_BAR_HEIGHT,
};
use crate::theme::colors::MaterialColorScheme;
use crate::theme::state_layer;
use crate::{Brush, DrawContext, TabsMetrics, WidgetInteractionState};
use vello::kurbo::{Rect, RoundedRectRadii};

pub const fn metrics() -> TabsMetrics {
    TabsMetrics::new(
        TABS_BAR_HEIGHT,
        TABS_ACTIVE_INDICATOR_HEIGHT,
        TABS_ACTIVE_INDICATOR_RADIUS,
    )
}

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
        RoundedRectRadii::new(
            TABS_ACTIVE_INDICATOR_RADIUS,
            TABS_ACTIVE_INDICATOR_RADIUS,
            0.0,
            0.0,
        ),
        &Brush::from(colors.primary.peniko()),
    );
}

pub fn draw_button_state_layer(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
    selected: bool,
    state: WidgetInteractionState,
) {
    state_layer::draw_bounded(
        draw,
        bounds,
        0.0.into(),
        if selected || state.pressed {
            colors.primary.peniko()
        } else {
            colors.on_surface.peniko()
        },
        state,
    );
}

#[cfg(test)]
mod tests {
    use super::metrics;
    use crate::dimensions::{
        TABS_ACTIVE_INDICATOR_HEIGHT, TABS_ACTIVE_INDICATOR_RADIUS, TABS_BAR_HEIGHT,
    };

    #[test]
    fn primary_tab_metrics_match_material_web_latest_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.bar_height, TABS_BAR_HEIGHT);
        assert_eq!(
            metrics.active_indicator_height,
            TABS_ACTIVE_INDICATOR_HEIGHT
        );
        assert_eq!(
            metrics.active_indicator_radius,
            TABS_ACTIVE_INDICATOR_RADIUS
        );
        assert_eq!(TABS_BAR_HEIGHT, 48.0);
        assert_eq!(TABS_ACTIVE_INDICATOR_HEIGHT, 3.0);
        assert_eq!(TABS_ACTIVE_INDICATOR_RADIUS, 3.0);
    }
}
