//! Persistent linear progress node.

use core::cell::RefCell;

use accesskit::{Node as AccessibilityNode, NodeId, Role};
use kurbo::{Rect, RoundedRect};
use nami::Computed;
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal, build_node};
use crate::text::DewState;
use crate::views::{child_in_rect, to_f32};

const BAR_HEIGHT: f64 = 4.0;
const BAR_SPACING: f64 = 6.0;
const VALUE_SPACING: f64 = 4.0;
const MIN_TRACK_WIDTH: f64 = 96.0;

struct ProgressNode {
    label: Box<dyn DewNode>,
    value_label: Box<dyn DewNode>,
    /// The progress fraction, subscribed once at build.
    value: WatchedSignal<Computed<f64>>,
    accessibility_id: NodeId,
}

pub fn build(
    renderer: &mut DewRenderer,
    config: ProgressConfig,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    assert!(
        matches!(config.style, ProgressStyle::Linear),
        "dew does not implement ProgressStyle::{:?}",
        config.style
    );
    Box::new(ProgressNode {
        label: build_node(renderer, config.label, env, depth),
        value_label: build_node(renderer, config.value_label, env, depth),
        value: WatchedSignal::new(config.value, renderer.signals()),
        accessibility_id: renderer.allocate_accessibility_id(),
    })
}

impl DewNode for ProgressNode {
    fn measure(&self, state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        let label = self.label.measure(state, ProposalSize::UNSPECIFIED).size;
        let value_label = self
            .value_label
            .measure(state, ProposalSize::UNSPECIFIED)
            .size;
        let mut height = BAR_HEIGHT;
        if label.height > 0.0 {
            height += f64::from(label.height) + BAR_SPACING;
        }
        if value_label.height > 0.0 {
            height += VALUE_SPACING + f64::from(value_label.height);
        }
        ViewDimensions::new(Size::new(
            to_f32(
                MIN_TRACK_WIDTH
                    .max(f64::from(label.width))
                    .max(f64::from(value_label.width)),
            ),
            to_f32(height),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let value = self.value.get();
        assert!(
            value.is_finite(),
            "dew does not render indeterminate progress"
        );
        let fraction = value.clamp(0.0, 1.0);
        if renderer.accessibility_enabled() {
            renderer.register_built_accessibility_node(
                self.accessibility_id,
                ctx.window_bounds(),
                || {
                    let mut accessibility = AccessibilityNode::new(Role::ProgressIndicator);
                    accessibility.set_min_numeric_value(0.0);
                    accessibility.set_max_numeric_value(1.0);
                    accessibility.set_numeric_value(fraction);
                    (accessibility, None)
                },
            );
        }
        let bounds = ctx.bounds;
        let label_size = self
            .label
            .measure(renderer.state_cell(), ProposalSize::UNSPECIFIED)
            .size;
        let label_height = f64::from(label_size.height);
        if label_height > 0.0 {
            let rect = Rect::new(
                bounds.x0,
                bounds.y0,
                bounds.x1,
                (bounds.y0 + label_height).min(bounds.y1),
            );
            self.label.render(renderer, child_in_rect(ctx, rect));
        }
        let bar_top = bounds.y0 + label_height + if label_height > 0.0 { BAR_SPACING } else { 0.0 };
        let track = Rect::new(bounds.x0, bar_top, bounds.x1, bar_top + BAR_HEIGHT);
        let radius = BAR_HEIGHT / 2.0;
        let track_color = renderer.theme().track();
        let accent = renderer.theme().accent();
        renderer.list_mut().fill(
            &RoundedRect::from_rect(track, radius),
            ctx.transform,
            track_color,
        );
        let fill = Rect::new(
            track.x0,
            track.y0,
            track.width().mul_add(fraction, track.x0),
            track.y1,
        );
        renderer
            .list_mut()
            .push_clip(ctx.transform.transform_rect_bbox(fill));
        renderer.list_mut().fill(
            &RoundedRect::from_rect(track, radius),
            ctx.transform,
            accent,
        );
        renderer.list_mut().pop_clip();
        let value_label_size = self
            .value_label
            .measure(renderer.state_cell(), ProposalSize::UNSPECIFIED)
            .size;
        if value_label_size.height > 0.0 {
            let rect = Rect::new(
                bounds.x0,
                (track.y1 + VALUE_SPACING).min(bounds.y1),
                bounds.x1,
                bounds.y1,
            );
            if rect.height() > 0.0 {
                self.value_label.render(renderer, child_in_rect(ctx, rect));
            }
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.label.patch(renderer) | self.value_label.patch(renderer)
    }
}
