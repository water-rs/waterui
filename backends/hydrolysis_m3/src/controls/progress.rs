use crate::dimensions::{
    PROGRESS_CIRCULAR_DIAMETER, PROGRESS_LINEAR_BAR_HEIGHT, PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
    PROGRESS_LINEAR_BAR_TOP_OFFSET, PROGRESS_LINEAR_LABEL_HEIGHT, PROGRESS_LINEAR_MIN_TRACK_WIDTH,
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
        ProgressIndicatorStyle::Circular => ProgressMetrics::circular(PROGRESS_CIRCULAR_DIAMETER),
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
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    center: vello::kurbo::Point,
    radius: f64,
    width: f64,
) {
    draw.stroke_circle(
        center,
        radius,
        &Brush::from(colors.surface_container_highest.peniko()),
        width,
    );
}

pub fn draw_circular_fill(
    colors: &MaterialColorScheme,
    draw: &mut dyn DrawContext,
    path: &vello::kurbo::BezPath,
    width: f64,
) {
    draw.stroke_path(path, &Brush::from(colors.primary.peniko()), width);
}
