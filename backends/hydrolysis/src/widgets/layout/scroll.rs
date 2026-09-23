#[cfg(feature = "accessibility")]
use crate::renderer::{AccessibilityActionTarget, HydrolysisRenderer};
use crate::renderer::{HydroNativeView, HydroState, WidgetRenderContext, measure_view_intrinsic};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::Size as LayoutSize;
use waterui_layout::scroll::{Axis as ScrollAxis, ScrollView};

use crate::widgets::widget_theme;

impl HydroNativeView for Native<ScrollView> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (_axis, content) = view.as_inner().as_parts();
        measure_view_intrinsic(content, state, env)
    }
}

#[cfg(feature = "accessibility")]
pub(crate) fn register_scroll_accessibility_node(
    renderer: &mut HydrolysisRenderer,
    env: &Environment,
    bounds: vello::kurbo::Rect,
    handle: &crate::scroll::ScrollHandle,
    metrics: crate::scroll::ScrollMetrics,
    axis: ScrollAxis,
) {
    let mut node = AccessibilityNode::new(
        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ScrollView),
    );
    let label = renderer.resolve_accessibility_label(env, None);
    if let Some(label) = label {
        node.set_label(label);
    }
    node.set_scroll_x(metrics.offset_x);
    node.set_scroll_x_min(0.0);
    node.set_scroll_x_max(metrics.max_x);
    node.set_scroll_y(metrics.offset_y);
    node.set_scroll_y_min(0.0);
    node.set_scroll_y_max(metrics.max_y);
    match axis {
        ScrollAxis::Horizontal => {
            node.add_action(AccessibilityAction::ScrollLeft);
            node.add_action(AccessibilityAction::ScrollRight);
        }
        ScrollAxis::Vertical => {
            node.add_action(AccessibilityAction::ScrollUp);
            node.add_action(AccessibilityAction::ScrollDown);
        }
        ScrollAxis::All => {
            node.add_action(AccessibilityAction::ScrollLeft);
            node.add_action(AccessibilityAction::ScrollRight);
            node.add_action(AccessibilityAction::ScrollUp);
            node.add_action(AccessibilityAction::ScrollDown);
        }
        _ => panic!("scroll axis variant is not supported by hydrolysis"),
    }
    let _ = renderer.register_accessibility_node(
        node,
        bounds,
        env,
        Some(AccessibilityActionTarget::Scroll {
            handle: handle.clone(),
            axis,
        }),
    );
}

pub(crate) fn draw_scroll_indicators(
    ctx: &mut WidgetRenderContext<'_>,
    env: &Environment,
    viewport: vello::kurbo::Rect,
    metrics: crate::scroll::ScrollMetrics,
    axis: ScrollAxis,
) {
    let theme = widget_theme(env);
    let mut draw = ctx.draw_context();
    match axis {
        ScrollAxis::Vertical | ScrollAxis::All if metrics.max_y > 0.0 => {
            let track_height = viewport.height();
            let min_thumb_height = track_height.min(12.0);
            let thumb_height = (track_height * (metrics.viewport_height / metrics.content_height))
                .clamp(min_thumb_height, track_height);
            let travel = track_height - thumb_height;
            let progress = metrics.offset_y / metrics.max_y;
            let thumb_y = viewport.y0 + travel * progress;
            theme.draw_scroll_indicator(
                &mut draw,
                vello::kurbo::Rect::new(
                    viewport.x1 - 4.0,
                    thumb_y,
                    viewport.x1 - 1.5,
                    thumb_y + thumb_height,
                ),
            );
        }
        _ => {}
    }

    match axis {
        ScrollAxis::Horizontal | ScrollAxis::All if metrics.max_x > 0.0 => {
            let track_width = viewport.width();
            let min_thumb_width = track_width.min(12.0);
            let thumb_width = (track_width * (metrics.viewport_width / metrics.content_width))
                .clamp(min_thumb_width, track_width);
            let travel = track_width - thumb_width;
            let progress = metrics.offset_x / metrics.max_x;
            let thumb_x = viewport.x0 + travel * progress;
            theme.draw_scroll_indicator(
                &mut draw,
                vello::kurbo::Rect::new(
                    thumb_x,
                    viewport.y1 - 4.0,
                    thumb_x + thumb_width,
                    viewport.y1 - 1.5,
                ),
            );
        }
        _ => {}
    }
}
