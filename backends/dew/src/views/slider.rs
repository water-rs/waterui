//! [`SliderConfig`]: a horizontal track with an accent fill and a round
//! thumb at the bound value.

use std::cell::RefCell;

use kurbo::{Circle, Rect, RoundedRect, Stroke};
use waterui_controls::slider::SliderConfig;
use waterui_core::layout::Size;
use waterui_core::{Environment, Native};

use crate::dispatch::{DewRenderer, RenderContext};
use crate::text::DewState;
use crate::theme;
use crate::views::{child_in_rect, measure_label, measure_subview, render_label, to_f32};

/// Track thickness in logical pixels.
const TRACK_HEIGHT: f64 = 4.0;
/// Thumb radius in logical pixels; also sets the control row height.
const THUMB_RADIUS: f64 = 10.0;
/// Gap between the end-value labels and the track.
const LABEL_SPACING: f64 = 8.0;
/// Gap between the title label row and the control row.
const ROW_SPACING: f64 = 4.0;
/// Minimum usable track length for measurement.
const MIN_TRACK_WIDTH: f64 = 96.0;
/// Outline width of the thumb circle.
const THUMB_BORDER: f64 = 1.0;

/// Draws the optional label row, the end-value labels, the track with its
/// accent fill, and the thumb at the current value.
pub fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<SliderConfig>,
    env: &Environment,
) {
    let config = view.into_inner();
    let bounds = ctx.bounds;
    let range_start = *config.range.start();
    let range_end = *config.range.end();
    let span = range_end - range_start;
    assert!(span > 0.0, "dew slider requires range start < end");

    let label_size = measure_label(renderer.state_cell(), &config.label, env);
    let label_height = if label_size.height > 0.0 {
        f64::from(label_size.height) + ROW_SPACING
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_rect = Rect::new(
            bounds.x0,
            bounds.y0,
            bounds.x1,
            (bounds.y0 + f64::from(label_size.height)).min(bounds.y1),
        );
        render_label(renderer, ctx, label_rect, &config.label, env);
    }

    let control_top = bounds.y0 + label_height;
    let control_bottom = bounds.y1;
    let center_y = control_top.midpoint(control_bottom);

    let min_size = measure_subview(renderer.state_cell(), &config.min_value_label, env);
    let max_size = measure_subview(renderer.state_cell(), &config.max_value_label, env);
    let mut track_left = bounds.x0 + THUMB_RADIUS;
    let mut track_right = bounds.x1 - THUMB_RADIUS;
    if min_size.width > 0.0 {
        let rect = Rect::new(
            bounds.x0,
            control_top,
            bounds.x0 + f64::from(min_size.width),
            control_bottom,
        );
        renderer.dispatch_child(config.min_value_label, env, child_in_rect(ctx, rect));
        track_left += f64::from(min_size.width) + LABEL_SPACING;
    }
    if max_size.width > 0.0 {
        let rect = Rect::new(
            bounds.x1 - f64::from(max_size.width),
            control_top,
            bounds.x1,
            control_bottom,
        );
        renderer.dispatch_child(config.max_value_label, env, child_in_rect(ctx, rect));
        track_right -= f64::from(max_size.width) + LABEL_SPACING;
    }
    assert!(
        track_right > track_left,
        "dew slider resolved a non-positive track width"
    );

    let clamped = renderer
        .read_signal(&config.value)
        .clamp(range_start, range_end);
    let progress = (clamped - range_start) / span;
    let fill_x = (track_right - track_left).mul_add(progress, track_left);

    let track_radius = TRACK_HEIGHT / 2.0;
    let track = Rect::new(
        track_left,
        center_y - track_radius,
        track_right,
        center_y + track_radius,
    );
    renderer.list_mut().fill(
        &RoundedRect::from_rect(track, track_radius),
        ctx.transform,
        theme::TRACK,
    );
    let fill = Rect::new(
        track_left,
        center_y - track_radius,
        fill_x,
        center_y + track_radius,
    );
    if fill.width() > 0.0 {
        renderer.list_mut().fill(
            &RoundedRect::from_rect(fill, track_radius),
            ctx.transform,
            theme::ACCENT,
        );
    }

    let thumb = Circle::new((fill_x, center_y), THUMB_RADIUS);
    renderer
        .list_mut()
        .fill(&thumb, ctx.transform, theme::THUMB);
    renderer.list_mut().stroke(
        &thumb,
        ctx.transform,
        Stroke::new(THUMB_BORDER),
        theme::BORDER,
    );
}

/// Intrinsic size: an optional label row above a thumb-tall control row;
/// the minimum width keeps a usable track between the end labels.
pub fn measure(state: &RefCell<DewState>, config: &SliderConfig, env: &Environment) -> Size {
    let label = measure_label(state, &config.label, env);
    let min_label = measure_subview(state, &config.min_value_label, env);
    let max_label = measure_subview(state, &config.max_value_label, env);

    let control_row = (THUMB_RADIUS * 2.0)
        .max(f64::from(min_label.height))
        .max(f64::from(max_label.height));
    let height = if label.height > 0.0 {
        f64::from(label.height) + ROW_SPACING + control_row
    } else {
        control_row
    };

    let mut track_row_width = THUMB_RADIUS.mul_add(2.0, MIN_TRACK_WIDTH);
    if min_label.width > 0.0 {
        track_row_width += f64::from(min_label.width) + LABEL_SPACING;
    }
    if max_label.width > 0.0 {
        track_row_width += f64::from(max_label.width) + LABEL_SPACING;
    }
    let width = f64::from(label.width).max(track_row_width);
    Size::new(to_f32(width), to_f32(height))
}
