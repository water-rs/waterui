use crate::colors::{ACCENT_FILL, OUTLINE_SUBTLE};
use crate::dimensions::{
    PROGRESS_CIRCULAR_DIAMETER, PROGRESS_LINEAR_BAR_HEIGHT, PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
    PROGRESS_LINEAR_BAR_TOP_OFFSET, PROGRESS_LINEAR_LABEL_HEIGHT, PROGRESS_LINEAR_MIN_TRACK_WIDTH,
    PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
};
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
        ProgressIndicatorStyle::Circular => ProgressMetrics::circular(PROGRESS_CIRCULAR_DIAMETER),
    }
}

pub fn draw_linear_track(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 2.0.into(), &Brush::from(OUTLINE_SUBTLE));
}

pub fn draw_linear_fill(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 2.0.into(), &Brush::from(ACCENT_FILL));
}

pub fn draw_circular_track(
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    width: f64,
) {
    draw.stroke_circle(center, radius, &Brush::from(OUTLINE_SUBTLE), width);
}

pub fn draw_circular_fill(draw: &mut dyn DrawContext, path: &vello::kurbo::BezPath, width: f64) {
    draw.stroke_path(path, &Brush::from(ACCENT_FILL), width);
}
