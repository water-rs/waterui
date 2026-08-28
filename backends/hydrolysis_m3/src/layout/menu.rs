use crate::dimensions::{
    TEXT_CONTEXT_MENU_CONTAINER_SHAPE, TEXT_CONTEXT_MENU_HORIZONTAL_PADDING,
    TEXT_CONTEXT_MENU_MAX_WIDTH, TEXT_CONTEXT_MENU_MIN_WIDTH, TEXT_CONTEXT_MENU_ROW_HEIGHT,
    TEXT_CONTEXT_MENU_SEPARATOR_HORIZONTAL_INSET, TEXT_CONTEXT_MENU_SEPARATOR_THICKNESS,
    TEXT_CONTEXT_MENU_VERTICAL_PADDING, TEXT_CONTEXT_MENU_WIDTH_PER_CHAR,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, TextContextMenuMetrics};

use vello::kurbo::{Rect, RoundedRectRadii};

pub const fn text_context_metrics() -> TextContextMenuMetrics {
    TextContextMenuMetrics {
        row_height: TEXT_CONTEXT_MENU_ROW_HEIGHT,
        horizontal_padding: TEXT_CONTEXT_MENU_HORIZONTAL_PADDING,
        vertical_padding: TEXT_CONTEXT_MENU_VERTICAL_PADDING,
        min_width: TEXT_CONTEXT_MENU_MIN_WIDTH,
        max_width: TEXT_CONTEXT_MENU_MAX_WIDTH,
        width_per_char: TEXT_CONTEXT_MENU_WIDTH_PER_CHAR,
        corner_radius: TEXT_CONTEXT_MENU_CONTAINER_SHAPE,
        separator_horizontal_inset: TEXT_CONTEXT_MENU_SEPARATOR_HORIZONTAL_INSET,
        separator_thickness: TEXT_CONTEXT_MENU_SEPARATOR_THICKNESS,
    }
}

pub fn draw_text_context_panel(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
) {
    draw.fill_rounded_rect(
        bounds,
        RoundedRectRadii::from_single_radius(TEXT_CONTEXT_MENU_CONTAINER_SHAPE),
        &Brush::from(colors.surface_container.peniko()),
    );
}

pub fn draw_text_context_separator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
) {
    draw.fill_rect(bounds, &Brush::from(colors.outline_variant.peniko()));
}

#[cfg(test)]
mod tests {
    use super::text_context_metrics;
    use crate::dimensions::{
        TEXT_CONTEXT_MENU_CONTAINER_SHAPE, TEXT_CONTEXT_MENU_HORIZONTAL_PADDING,
        TEXT_CONTEXT_MENU_MIN_WIDTH, TEXT_CONTEXT_MENU_ROW_HEIGHT,
    };

    #[test]
    fn text_context_menu_metrics_match_material_menu_tokens() {
        let metrics = text_context_metrics();

        assert_eq!(metrics.row_height, TEXT_CONTEXT_MENU_ROW_HEIGHT);
        assert_eq!(metrics.row_height, 56.0);
        assert_eq!(
            metrics.horizontal_padding,
            TEXT_CONTEXT_MENU_HORIZONTAL_PADDING
        );
        assert_eq!(metrics.min_width, TEXT_CONTEXT_MENU_MIN_WIDTH);
        assert_eq!(metrics.min_width, 112.0);
        assert_eq!(metrics.corner_radius, TEXT_CONTEXT_MENU_CONTAINER_SHAPE);
        assert_eq!(metrics.corner_radius, 4.0);
    }
}
