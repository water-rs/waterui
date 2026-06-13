//! [`ProgressConfig`]: a linear bar with an accent fill for the bound
//! fraction, between its label and value-label views.
//!
//! Circular and indeterminate progress need the animation runtime and are
//! explicitly unsupported until it lands; dew fast-fails on them instead of
//! rendering a frozen approximation.

use std::cell::RefCell;

use kurbo::{Rect, RoundedRect};
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui_core::layout::Size;
use waterui_core::{Environment, Native};

use crate::dispatch::{DewRenderer, RenderContext};
use crate::text::DewState;
use crate::theme;
use crate::views::{child_in_rect, measure_subview, to_f32};

/// Bar thickness in logical pixels.
const BAR_HEIGHT: f64 = 4.0;
/// Gap between the label row and the bar.
const BAR_SPACING: f64 = 6.0;
/// Gap between the bar and the value label.
const VALUE_SPACING: f64 = 4.0;
/// Minimum bar length for measurement.
const MIN_TRACK_WIDTH: f64 = 96.0;

fn require_linear(style: ProgressStyle) {
    assert!(
        matches!(style, ProgressStyle::Linear),
        "dew does not implement ProgressStyle::{style:?}"
    );
}

/// Draws the label, the track with its accent fill, and the value label.
pub fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<ProgressConfig>,
    env: &Environment,
) {
    let config = view.into_inner();
    require_linear(config.style);
    let value = renderer.read_signal(&config.value);
    assert!(
        value.is_finite(),
        "dew does not render indeterminate progress"
    );
    let fraction = value.clamp(0.0, 1.0);
    let bounds = ctx.bounds;

    let label_size = measure_subview(renderer.state_cell(), &config.label, env);
    let label_height = f64::from(label_size.height);
    if label_height > 0.0 {
        let rect = Rect::new(
            bounds.x0,
            bounds.y0,
            bounds.x1,
            (bounds.y0 + label_height).min(bounds.y1),
        );
        renderer.dispatch_child(config.label, env, child_in_rect(ctx, rect));
    }

    let bar_top = bounds.y0 + label_height + if label_height > 0.0 { BAR_SPACING } else { 0.0 };
    let track = Rect::new(bounds.x0, bar_top, bounds.x1, bar_top + BAR_HEIGHT);
    let radius = BAR_HEIGHT / 2.0;
    renderer.list_mut().fill(
        &RoundedRect::from_rect(track, radius),
        ctx.transform,
        theme::TRACK,
    );
    let fill = Rect::new(
        track.x0,
        track.y0,
        track.width().mul_add(fraction, track.x0),
        track.y1,
    );
    if fill.width() > 0.0 {
        renderer.list_mut().fill(
            &RoundedRect::from_rect(fill, radius),
            ctx.transform,
            theme::ACCENT,
        );
    }

    let value_label_size = measure_subview(renderer.state_cell(), &config.value_label, env);
    if value_label_size.height > 0.0 {
        let rect = Rect::new(
            bounds.x0,
            (track.y1 + VALUE_SPACING).min(bounds.y1),
            bounds.x1,
            bounds.y1,
        );
        if rect.height() > 0.0 {
            renderer.dispatch_child(config.value_label, env, child_in_rect(ctx, rect));
        }
    }
}

/// Intrinsic size: label row, bar, and value-label row stacked; horizontal
/// stretch expands the bar to the available width during placement.
pub fn measure(state: &RefCell<DewState>, config: &ProgressConfig, env: &Environment) -> Size {
    require_linear(config.style);
    let label = measure_subview(state, &config.label, env);
    let value_label = measure_subview(state, &config.value_label, env);

    let mut height = BAR_HEIGHT;
    if label.height > 0.0 {
        height += f64::from(label.height) + BAR_SPACING;
    }
    if value_label.height > 0.0 {
        height += VALUE_SPACING + f64::from(value_label.height);
    }
    let width = MIN_TRACK_WIDTH
        .max(f64::from(label.width))
        .max(f64::from(value_label.width));
    Size::new(to_f32(width), to_f32(height))
}
