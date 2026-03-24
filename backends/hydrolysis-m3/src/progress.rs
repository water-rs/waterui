use crate::colors::{ACCENT_FILL, OUTLINE_SUBTLE};
use crate::dimensions::{
    PROGRESS_CIRCULAR_DIAMETER, PROGRESS_LINEAR_BAR_HEIGHT, PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
    PROGRESS_LINEAR_BAR_TOP_OFFSET, PROGRESS_LINEAR_LABEL_HEIGHT, PROGRESS_LINEAR_MIN_TRACK_WIDTH,
    PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
};
use crate::{Brush, DrawContext, ProgressMetrics};
use waterui::component::progress::ProgressStyle;

pub(crate) fn metrics(style: ProgressStyle) -> ProgressMetrics {
    match style {
        ProgressStyle::Linear => ProgressMetrics::linear(
            PROGRESS_LINEAR_LABEL_HEIGHT,
            PROGRESS_LINEAR_BAR_TOP_OFFSET,
            PROGRESS_LINEAR_BAR_HEIGHT,
            PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
            PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
            PROGRESS_LINEAR_MIN_TRACK_WIDTH,
        ),
        ProgressStyle::Circular => ProgressMetrics::circular(PROGRESS_CIRCULAR_DIAMETER),
        _ => panic!("hydrolysis ProgressStyle variant is not implemented"),
    }
}

pub(crate) fn draw_linear_track(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 4.0.into(), &Brush::from(OUTLINE_SUBTLE));
}

pub(crate) fn draw_linear_fill(draw: &mut dyn DrawContext, bounds: vello::kurbo::Rect) {
    draw.fill_rounded_rect(bounds, 4.0.into(), &Brush::from(ACCENT_FILL));
}

pub(crate) fn draw_circular_track(
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    width: f64,
) {
    draw.stroke_circle(center, radius, &Brush::from(OUTLINE_SUBTLE), width);
}

pub(crate) fn draw_circular_fill(
    draw: &mut dyn DrawContext,
    path: &vello::kurbo::BezPath,
    width: f64,
) {
    draw.stroke_path(path, &Brush::from(ACCENT_FILL), width);
}
