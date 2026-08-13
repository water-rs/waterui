//! Persistent [`SliderConfig`] node.

use core::cell::{Cell, RefCell};
use core::ops::RangeInclusive;
use std::rc::Rc;

use kurbo::{Circle, Rect, RoundedRect, Stroke};
use nami::{Binding, Computed, Signal};

use waterui_controls::slider::SliderConfig;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal, build_node};
use crate::pointer::{PointerHandler, PointerTargetHandle};
use crate::text::DewState;
use crate::views::{LabelText, child_in_rect, to_f32};

const TRACK_HEIGHT: f64 = 4.0;
const THUMB_RADIUS: f64 = 10.0;
const LABEL_SPACING: f64 = 8.0;
const ROW_SPACING: f64 = 4.0;
const MIN_TRACK_WIDTH: f64 = 96.0;
const THUMB_BORDER: f64 = 1.0;

struct SliderNode {
    label: LabelText,
    min_value_label: Box<dyn DewNode>,
    max_value_label: Box<dyn DewNode>,
    range: RangeInclusive<f64>,
    /// The bound value, subscribed once at build.
    value: WatchedSignal<Binding<f64>>,
    /// The environment's disabled scope, subscribed once at build.
    disabled: WatchedSignal<Computed<bool>>,
    env: Environment,
    track: Rc<Cell<Rect>>,
    pointer: PointerTargetHandle,
}

struct SliderPointer {
    range: RangeInclusive<f64>,
    value: Binding<f64>,
    disabled: Computed<bool>,
    track: Rc<Cell<Rect>>,
}

#[derive(Clone, Copy)]
struct SliderTrackGeometry {
    left: f64,
    right: f64,
    center_y: f64,
    fill_x: f64,
}

impl SliderPointer {
    fn update(&self, point: kurbo::Point) -> bool {
        if self.disabled.get() {
            return false;
        }
        let track = self.track.get();
        assert!(
            track.width() > 0.0,
            "dew slider input requires rendered track geometry"
        );
        let start = *self.range.start();
        let end = *self.range.end();
        let progress = ((point.x - track.x0) / track.width()).clamp(0.0, 1.0);
        let next = (end - start).mul_add(progress, start);
        if self.value.get().to_bits() == next.to_bits() {
            return false;
        }
        self.value.set(next);
        true
    }
}

impl PointerHandler for SliderPointer {
    fn pointer_down(&mut self, point: kurbo::Point, _bounds: Rect, _env: &Environment) -> bool {
        self.update(point)
    }

    fn pointer_move(&mut self, point: kurbo::Point, _bounds: Rect, _env: &Environment) -> bool {
        self.update(point)
    }

    fn pointer_up(&mut self, point: kurbo::Point, _bounds: Rect, _env: &Environment) -> bool {
        self.update(point)
    }
}

pub fn build(
    renderer: &mut DewRenderer,
    config: SliderConfig,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let track = Rc::new(Cell::new(Rect::ZERO));
    let pointer = PointerTargetHandle::new(SliderPointer {
        range: config.range.clone(),
        value: config.value.clone(),
        disabled: crate::views::view_disabled(env),
        track: Rc::clone(&track),
    });
    Box::new(SliderNode {
        label: LabelText::new(&config.label, env, renderer.signals()),
        min_value_label: build_node(renderer, config.min_value_label, env, depth),
        max_value_label: build_node(renderer, config.max_value_label, env, depth),
        range: config.range,
        value: WatchedSignal::new(config.value, renderer.signals()),
        disabled: WatchedSignal::new(crate::views::view_disabled(env), renderer.signals()),
        env: env.clone(),
        track,
        pointer,
    })
}

impl DewNode for SliderNode {
    fn measure(&self, state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        let label = self.label.measure(state, &self.env);
        let min_label = self
            .min_value_label
            .measure(state, ProposalSize::UNSPECIFIED)
            .size;
        let max_label = self
            .max_value_label
            .measure(state, ProposalSize::UNSPECIFIED)
            .size;

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
        ViewDimensions::new(Size::new(
            to_f32(f64::from(label.width).max(track_row_width)),
            to_f32(height),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let bounds = ctx.bounds;
        let range_start = *self.range.start();
        let range_end = *self.range.end();
        let span = range_end - range_start;
        assert!(span > 0.0, "dew slider requires range start < end");

        let disabled = self.disabled.get();
        let label_size = self.label.measure(renderer.state_cell(), &self.env);
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
            self.label.render(renderer, ctx, label_rect, &self.env);
        }

        let control_top = bounds.y0 + label_height;
        let control_bottom = bounds.y1;
        let center_y = control_top.midpoint(control_bottom);
        let min_size = self
            .min_value_label
            .measure(renderer.state_cell(), ProposalSize::UNSPECIFIED)
            .size;
        let max_size = self
            .max_value_label
            .measure(renderer.state_cell(), ProposalSize::UNSPECIFIED)
            .size;
        let mut track_left = bounds.x0 + THUMB_RADIUS;
        let mut track_right = bounds.x1 - THUMB_RADIUS;
        if min_size.width > 0.0 {
            let rect = Rect::new(
                bounds.x0,
                control_top,
                bounds.x0 + f64::from(min_size.width),
                control_bottom,
            );
            self.min_value_label
                .render(renderer, child_in_rect(ctx, rect));
            track_left += f64::from(min_size.width) + LABEL_SPACING;
        }
        if max_size.width > 0.0 {
            let rect = Rect::new(
                bounds.x1 - f64::from(max_size.width),
                control_top,
                bounds.x1,
                control_bottom,
            );
            self.max_value_label
                .render(renderer, child_in_rect(ctx, rect));
            track_right -= f64::from(max_size.width) + LABEL_SPACING;
        }
        assert!(
            track_right > track_left,
            "dew slider resolved a non-positive track width"
        );

        let clamped = self.value.get().clamp(range_start, range_end);
        let progress = (clamped - range_start) / span;
        let fill_x = (track_right - track_left).mul_add(progress, track_left);
        self.render_track(
            renderer,
            ctx,
            SliderTrackGeometry {
                left: track_left,
                right: track_right,
                center_y,
                fill_x,
            },
            disabled,
        );
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.min_value_label.patch(renderer) | self.max_value_label.patch(renderer)
    }
}

impl SliderNode {
    fn render_track(
        &self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        geometry: SliderTrackGeometry,
        disabled: bool,
    ) {
        let track_radius = TRACK_HEIGHT / 2.0;
        let track = Rect::new(
            geometry.left,
            geometry.center_y - track_radius,
            geometry.right,
            geometry.center_y + track_radius,
        );
        let window_track = ctx.transform.transform_rect_bbox(track);
        self.track.set(window_track);
        if !disabled {
            let hit = Rect::new(
                window_track.x0 - THUMB_RADIUS,
                window_track.y0 - THUMB_RADIUS,
                window_track.x1 + THUMB_RADIUS,
                window_track.y1 + THUMB_RADIUS,
            );
            renderer.register_pointer_target(hit, self.pointer.clone());
        }
        let track_color = renderer.theme().track();
        let accent = renderer.theme().accent();
        let thumb_color = renderer.theme().thumb();
        let border_color = renderer.theme().border();
        renderer.list_mut().fill(
            &RoundedRect::from_rect(track, track_radius),
            ctx.transform,
            track_color,
        );
        let fill = Rect::new(
            geometry.left,
            geometry.center_y - track_radius,
            geometry.fill_x,
            geometry.center_y + track_radius,
        );
        renderer
            .list_mut()
            .push_clip(ctx.transform.transform_rect_bbox(fill));
        renderer.list_mut().fill(
            &RoundedRect::from_rect(track, track_radius),
            ctx.transform,
            accent,
        );
        renderer.list_mut().pop_clip();
        let thumb = Circle::new((geometry.fill_x, geometry.center_y), THUMB_RADIUS);
        renderer.list_mut().fill(&thumb, ctx.transform, thumb_color);
        renderer.list_mut().stroke(
            &thumb,
            ctx.transform,
            Stroke::new(THUMB_BORDER),
            border_color,
        );
    }
}
