//! [`ToggleConfig`]: a sliding pill switch driven by a `bool` binding.

use std::cell::RefCell;

use kurbo::{Circle, Rect, RoundedRect, Stroke};
use nami::{Binding, Computed, Signal};
use waterui_controls::toggle::{ToggleConfig, ToggleStyle};
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};

use crate::dispatch::{DewNode, DewRenderer, RenderContext};
use crate::pointer::{PointerHandler, PointerTargetHandle};
use crate::text::DewState;
use crate::theme;
use crate::views::{measure_label, render_label, to_f32};

/// Switch track width in logical pixels (the familiar pill footprint).
const TRACK_WIDTH: f64 = 51.0;
/// Switch track height in logical pixels.
const TRACK_HEIGHT: f64 = 31.0;
/// Gap between the label and the switch.
const LABEL_SPACING: f64 = 8.0;
/// Inset of the thumb circle from the track edge.
const THUMB_INSET: f64 = 2.0;
/// Outline width of the thumb circle.
const THUMB_BORDER: f64 = 1.0;

fn require_switch_style(style: ToggleStyle) {
    assert!(
        matches!(style, ToggleStyle::Automatic | ToggleStyle::Switch),
        "dew does not implement ToggleStyle::{style:?}"
    );
}

struct ToggleNode {
    config: ToggleConfig,
    env: Environment,
    pointer: PointerTargetHandle,
}

struct TogglePointer {
    toggle: Binding<bool>,
    disabled: Computed<bool>,
    armed: bool,
}

impl PointerHandler for TogglePointer {
    fn pointer_down(&mut self, _point: kurbo::Point, _bounds: Rect, _env: &Environment) -> bool {
        self.armed = !self.disabled.get();
        false
    }

    fn pointer_up(&mut self, point: kurbo::Point, bounds: Rect, _env: &Environment) -> bool {
        let activate =
            core::mem::take(&mut self.armed) && bounds.contains(point) && !self.disabled.get();
        if activate {
            self.toggle.set(!self.toggle.get());
        }
        activate
    }

    fn pointer_cancel(&mut self, _env: &Environment) -> bool {
        self.armed = false;
        false
    }
}

pub fn build(config: ToggleConfig, env: &Environment) -> Box<dyn DewNode> {
    require_switch_style(config.style);
    let pointer = PointerTargetHandle::new(TogglePointer {
        toggle: config.toggle.clone(),
        disabled: crate::views::view_disabled(env),
        armed: false,
    });
    Box::new(ToggleNode {
        config,
        env: env.clone(),
        pointer,
    })
}

impl DewNode for ToggleNode {
    fn measure(&self, state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(measure(state, &self.config, &self.env))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        render(renderer, ctx, &self.config, &self.env);
        if !crate::views::view_disabled(&self.env).get() {
            renderer.register_pointer_target(ctx.window_bounds(), self.pointer.clone());
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    config: &ToggleConfig,
    env: &Environment,
) {
    require_switch_style(config.style);
    let on = renderer.read_signal(&config.toggle);
    let disabled = renderer.read_signal(&crate::views::view_disabled(env));
    let bounds = ctx.bounds;

    let track_left = (bounds.x1 - TRACK_WIDTH).max(bounds.x0);
    let track_top = bounds.y0 + ((bounds.height() - TRACK_HEIGHT) / 2.0).max(0.0);
    let track = Rect::new(
        track_left,
        track_top,
        track_left + TRACK_WIDTH,
        track_top + TRACK_HEIGHT,
    );

    let label_rect = Rect::new(
        bounds.x0,
        bounds.y0,
        (track_left - LABEL_SPACING).max(bounds.x0),
        bounds.y1,
    );
    if label_rect.width() > 0.0 {
        render_label(renderer, ctx, label_rect, &config.label, env);
    }

    let radius = TRACK_HEIGHT / 2.0;
    let track_color = if disabled {
        theme::BORDER
    } else if on {
        theme::ACCENT
    } else {
        theme::TRACK
    };
    renderer.list_mut().fill(
        &RoundedRect::from_rect(track, radius),
        ctx.transform,
        track_color,
    );

    let thumb_center_x = if on {
        track.x1 - radius
    } else {
        track.x0 + radius
    };
    let thumb = Circle::new((thumb_center_x, track.y0 + radius), radius - THUMB_INSET);
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

/// Label width plus the switch footprint; the dispatcher's horizontal
/// stretch lets the row expand so the switch trails at the far edge.
fn measure(state: &RefCell<DewState>, config: &ToggleConfig, env: &Environment) -> Size {
    require_switch_style(config.style);
    let label = measure_label(state, &config.label, env);
    let width = if label.width > 0.0 {
        f64::from(label.width) + LABEL_SPACING + TRACK_WIDTH
    } else {
        TRACK_WIDTH
    };
    Size::new(to_f32(width), label.height.max(to_f32(TRACK_HEIGHT)))
}
