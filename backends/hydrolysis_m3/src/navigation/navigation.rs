use crate::dimensions::{
    NAVIGATION_BACK_BUTTON_LEADING_INSET, NAVIGATION_BACK_BUTTON_SIZE,
    NAVIGATION_BACK_BUTTON_TOP_INSET, NAVIGATION_BAR_AUTOMATIC_HEIGHT,
    NAVIGATION_BAR_HORIZONTAL_INSET, NAVIGATION_BAR_INLINE_HEIGHT, NAVIGATION_BAR_ITEM_SPACING,
    NAVIGATION_BAR_LARGE_HEIGHT, NAVIGATION_LARGE_TITLE_BOTTOM_INSET, NAVIGATION_SEARCH_HEIGHT,
    NAVIGATION_SEARCH_VERTICAL_INSET, NAVIGATION_TITLE_INLINE_HEIGHT,
    NAVIGATION_TITLE_LARGE_HEIGHT, NAVIGATION_TITLE_LEADING_INSET, NAVIGATION_TITLE_TRAILING_INSET,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, NavigationMetrics};
use vello::kurbo::{BezPath, Point, Rect};

pub const fn metrics() -> NavigationMetrics {
    NavigationMetrics {
        automatic_bar_height: NAVIGATION_BAR_AUTOMATIC_HEIGHT,
        inline_bar_height: NAVIGATION_BAR_INLINE_HEIGHT,
        large_bar_height: NAVIGATION_BAR_LARGE_HEIGHT,
        inline_title_height: NAVIGATION_TITLE_INLINE_HEIGHT,
        large_title_height: NAVIGATION_TITLE_LARGE_HEIGHT,
        title_leading_inset: NAVIGATION_TITLE_LEADING_INSET,
        title_trailing_inset: NAVIGATION_TITLE_TRAILING_INSET,
        large_title_bottom_inset: NAVIGATION_LARGE_TITLE_BOTTOM_INSET,
        horizontal_inset: NAVIGATION_BAR_HORIZONTAL_INSET,
        item_spacing: NAVIGATION_BAR_ITEM_SPACING,
        search_height: NAVIGATION_SEARCH_HEIGHT,
        search_vertical_inset: NAVIGATION_SEARCH_VERTICAL_INSET,
        back_button_size: NAVIGATION_BACK_BUTTON_SIZE,
        back_button_leading_inset: NAVIGATION_BACK_BUTTON_LEADING_INSET,
        back_button_top_inset: NAVIGATION_BACK_BUTTON_TOP_INSET,
    }
}

pub fn draw_bar(draw: &mut dyn DrawContext, bounds: Rect, background: &Brush) {
    draw.fill_rect(bounds, background);
}

pub fn draw_bar_separator(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(colors.outline_variant.peniko()));
}

pub fn draw_back_button(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    let center = Point::new(
        bounds.width().mul_add(0.5, bounds.x0),
        bounds.height().mul_add(0.5, bounds.y0),
    );
    draw.fill_circle(
        center,
        bounds.width().min(bounds.height()) * 0.5,
        &Brush::from(colors.surface_container.peniko()),
    );
    let mut chevron = BezPath::new();
    chevron.move_to(Point::new(center.x + 4.0, center.y - 7.0));
    chevron.line_to(Point::new(center.x - 4.0, center.y));
    chevron.line_to(Point::new(center.x + 4.0, center.y + 7.0));
    draw.stroke_path(&chevron, &Brush::from(colors.on_surface.peniko()), 2.0);
}

#[cfg(test)]
mod tests {
    use super::metrics;
    use crate::dimensions::{
        NAVIGATION_BACK_BUTTON_LEADING_INSET, NAVIGATION_BACK_BUTTON_SIZE,
        NAVIGATION_BACK_BUTTON_TOP_INSET, NAVIGATION_BAR_AUTOMATIC_HEIGHT,
        NAVIGATION_BAR_HORIZONTAL_INSET, NAVIGATION_BAR_INLINE_HEIGHT, NAVIGATION_BAR_ITEM_SPACING,
        NAVIGATION_BAR_LARGE_HEIGHT, NAVIGATION_LARGE_TITLE_BOTTOM_INSET, NAVIGATION_SEARCH_HEIGHT,
        NAVIGATION_SEARCH_VERTICAL_INSET, NAVIGATION_TITLE_LEADING_INSET,
        NAVIGATION_TITLE_TRAILING_INSET,
    };

    #[test]
    fn navigation_metrics_match_compose_app_bar_tokens() {
        let metrics = metrics();

        assert_eq!(
            metrics.automatic_bar_height,
            NAVIGATION_BAR_AUTOMATIC_HEIGHT
        );
        assert_eq!(metrics.inline_bar_height, NAVIGATION_BAR_INLINE_HEIGHT);
        assert_eq!(metrics.large_bar_height, NAVIGATION_BAR_LARGE_HEIGHT);
        assert_eq!(metrics.title_leading_inset, NAVIGATION_TITLE_LEADING_INSET);
        assert_eq!(
            metrics.title_trailing_inset,
            NAVIGATION_TITLE_TRAILING_INSET
        );
        assert_eq!(
            metrics.large_title_bottom_inset,
            NAVIGATION_LARGE_TITLE_BOTTOM_INSET
        );
        assert_eq!(metrics.horizontal_inset, NAVIGATION_BAR_HORIZONTAL_INSET);
        assert_eq!(metrics.item_spacing, NAVIGATION_BAR_ITEM_SPACING);
        assert_eq!(metrics.search_height, NAVIGATION_SEARCH_HEIGHT);
        assert_eq!(
            metrics.search_vertical_inset,
            NAVIGATION_SEARCH_VERTICAL_INSET
        );
        assert_eq!(metrics.back_button_size, NAVIGATION_BACK_BUTTON_SIZE);
        assert_eq!(
            metrics.back_button_leading_inset,
            NAVIGATION_BACK_BUTTON_LEADING_INSET
        );
        assert_eq!(
            metrics.back_button_top_inset,
            NAVIGATION_BACK_BUTTON_TOP_INSET
        );
        assert_eq!(NAVIGATION_BAR_AUTOMATIC_HEIGHT, 64.0);
        assert_eq!(NAVIGATION_BAR_INLINE_HEIGHT, 64.0);
        assert_eq!(NAVIGATION_BAR_LARGE_HEIGHT, 152.0);
        assert_eq!(NAVIGATION_TITLE_LEADING_INSET, 16.0);
        assert_eq!(NAVIGATION_LARGE_TITLE_BOTTOM_INSET, 28.0);
        assert_eq!(NAVIGATION_SEARCH_HEIGHT, 56.0);
        assert_eq!(NAVIGATION_BAR_HORIZONTAL_INSET, 4.0);
        assert_eq!(NAVIGATION_BACK_BUTTON_SIZE, 40.0);
    }
}
