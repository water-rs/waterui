use crate::dimensions::{
    PROGRESS_CIRCULAR_DIAMETER, PROGRESS_CIRCULAR_STROKE_WIDTH, PROGRESS_LINEAR_BAR_HEIGHT,
    PROGRESS_LINEAR_BAR_HORIZONTAL_INSET, PROGRESS_LINEAR_BAR_TOP_OFFSET,
    PROGRESS_LINEAR_LABEL_HEIGHT, PROGRESS_LINEAR_MIN_TRACK_WIDTH,
    PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
};
use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DrawContext, ProgressIndicatorStyle, ProgressMetrics};

pub fn metrics(style: ProgressIndicatorStyle) -> ProgressMetrics {
    match style {
        ProgressIndicatorStyle::Linear => ProgressMetrics::linear(
            PROGRESS_LINEAR_LABEL_HEIGHT,
            PROGRESS_LINEAR_BAR_TOP_OFFSET,
            PROGRESS_LINEAR_BAR_HEIGHT,
            PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
            PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
            PROGRESS_LINEAR_MIN_TRACK_WIDTH,
        ),
        ProgressIndicatorStyle::Circular => {
            ProgressMetrics::circular(PROGRESS_CIRCULAR_DIAMETER, PROGRESS_CIRCULAR_STROKE_WIDTH)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgressIndicatorStyle, metrics};

    #[test]
    fn progress_metrics_match_material_web_v0_192() {
        let linear = metrics(ProgressIndicatorStyle::Linear);
        assert_eq!(linear.bar_height, 4.0);
        assert_eq!(linear.min_track_width, 80.0);

        let circular = metrics(ProgressIndicatorStyle::Circular);
        assert_eq!(circular.circular_diameter, 48.0);
        assert_eq!(circular.circular_stroke_width, 4.0);
    }
}

pub fn draw_linear_track(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    draw.fill_rect(
        bounds,
        &Brush::from(colors.surface_container_highest.peniko()),
    );
}

pub fn draw_linear_fill(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    bounds: vello::kurbo::Rect,
) {
    draw.fill_rect(bounds, &Brush::from(colors.primary.peniko()));
}

pub fn draw_circular_track(
    _colors: &MaterialColorScheme,
    _draw: &mut dyn DrawContext,
    _center: vello::kurbo::Point,
    _radius: f64,
    _width: f64,
) {
}

pub fn draw_circular_fill(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    path: &vello::kurbo::BezPath,
    width: f64,
) {
    draw.stroke_path(path, &Brush::from(colors.primary.peniko()), width);
}
