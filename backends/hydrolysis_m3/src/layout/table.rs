use crate::dimensions::{
    TABLE_CELL_HORIZONTAL_PADDING, TABLE_CELL_VERTICAL_INSET, TABLE_HEADER_HEIGHT,
    TABLE_MIN_COLUMN_WIDTH, TABLE_OUTLINE_WIDTH, TABLE_ROW_HEIGHT,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, TableMetrics};
use vello::kurbo::{Point, Rect};

pub const fn metrics() -> TableMetrics {
    TableMetrics {
        min_column_width: TABLE_MIN_COLUMN_WIDTH,
        cell_horizontal_padding: TABLE_CELL_HORIZONTAL_PADDING,
        cell_vertical_inset: TABLE_CELL_VERTICAL_INSET,
        header_height: TABLE_HEADER_HEIGHT,
        row_height: TABLE_ROW_HEIGHT,
        outline_width: TABLE_OUTLINE_WIDTH,
    }
}

pub fn draw_background(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(colors.surface.peniko()));
}

pub fn draw_header_background(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: Rect,
) {
    draw.fill_rect(bounds, &Brush::from(colors.surface.peniko()));
}

pub fn draw_cell_border(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.stroke_rect(
        bounds,
        &Brush::from(colors.outline_variant.peniko()),
        TABLE_OUTLINE_WIDTH,
    );
}

pub fn draw_column_separator(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    from: Point,
    to: Point,
) {
    draw.stroke_line(
        from,
        to,
        &Brush::from(colors.outline_variant.peniko()),
        TABLE_OUTLINE_WIDTH,
    );
}

#[cfg(test)]
mod tests {
    use super::metrics;
    use crate::dimensions::{
        TABLE_CELL_HORIZONTAL_PADDING, TABLE_CELL_VERTICAL_INSET, TABLE_HEADER_HEIGHT,
        TABLE_MIN_COLUMN_WIDTH, TABLE_OUTLINE_WIDTH, TABLE_ROW_HEIGHT,
    };

    #[test]
    fn table_metrics_match_material_web_latest_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.min_column_width, TABLE_MIN_COLUMN_WIDTH);
        assert_eq!(
            metrics.cell_horizontal_padding,
            TABLE_CELL_HORIZONTAL_PADDING
        );
        assert_eq!(metrics.cell_vertical_inset, TABLE_CELL_VERTICAL_INSET);
        assert_eq!(metrics.header_height, TABLE_HEADER_HEIGHT);
        assert_eq!(metrics.row_height, TABLE_ROW_HEIGHT);
        assert_eq!(metrics.outline_width, TABLE_OUTLINE_WIDTH);
        assert_eq!(TABLE_HEADER_HEIGHT, 56.0);
        assert_eq!(TABLE_ROW_HEIGHT, 52.0);
        assert_eq!(TABLE_CELL_HORIZONTAL_PADDING, 32.0);
        assert_eq!(TABLE_CELL_VERTICAL_INSET, 16.0);
        assert_eq!(TABLE_OUTLINE_WIDTH, 1.0);
    }
}
