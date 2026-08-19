#[cfg(feature = "accessibility")]
use crate::renderer::{AccessibilityActionTarget, HydrolysisRenderer};
use crate::renderer::{
    HydroNativeView, HydroState, WidgetRenderContext, measure_view_intrinsic, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, NodeId as AccessibilityNodeId,
    Role as AccessibilityNodeRole,
};
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::Size as LayoutSize;
use waterui_layout::scroll::{Axis as ScrollAxis, ScrollView};

use crate::widgets::widget_theme;

/// Width of the grabbable scrollbar gutter along the viewport edge, in logical
/// pixels. Wider than the drawn thumb so the bar is comfortable to pick up.
const SCROLL_INDICATOR_GUTTER: f64 = 12.0;
/// Drawn thumb thickness at rest.
const SCROLL_INDICATOR_THICKNESS: f64 = 2.5;
/// Drawn thumb thickness while the thumb is being dragged.
const SCROLL_INDICATOR_DRAG_THICKNESS: f64 = 6.5;
/// Inset of the thumb from the viewport edge.
const SCROLL_INDICATOR_EDGE_INSET: f64 = 1.5;

impl HydroNativeView for Native<ScrollView> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (_axis, content, _controller) = view.as_inner().as_parts();
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
) -> Option<AccessibilityNodeId> {
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
    renderer.register_accessibility_node(
        node,
        bounds,
        env,
        Some(AccessibilityActionTarget::Scroll {
            handle: handle.clone(),
            axis,
        }),
    )
}

/// Geometry of one scroll indicator along its track: where the thumb starts,
/// its extent, and how far it can travel. `None` when the content does not
/// overflow the viewport on that axis.
struct IndicatorGeometry {
    thumb_offset: f64,
    thumb_extent: f64,
    travel: f64,
}

fn indicator_geometry(
    track: f64,
    viewport_extent: f64,
    content_extent: f64,
    max_offset: f64,
    offset: f64,
) -> Option<IndicatorGeometry> {
    if max_offset <= 0.0 {
        return None;
    }
    let min_thumb = track.min(12.0);
    let thumb_extent = (track * (viewport_extent / content_extent)).clamp(min_thumb, track);
    let travel = track - thumb_extent;
    let progress = offset / max_offset;
    Some(IndicatorGeometry {
        thumb_offset: travel * progress,
        thumb_extent,
        travel,
    })
}

/// Draws the scroll indicators for `metrics` and registers their gutters as
/// draggable scrollbar targets: pressing the thumb drags it, pressing the track
/// jumps the thumb to the pointer and keeps dragging. The thumb draws widened
/// while it owns a drag; a drag schedules re-encode frames only, never layout.
pub(crate) fn draw_scroll_indicators(
    ctx: &mut WidgetRenderContext<'_>,
    env: &Environment,
    viewport: vello::kurbo::Rect,
    metrics: crate::scroll::ScrollMetrics,
    axis: ScrollAxis,
    handle: &crate::scroll::ScrollHandle,
) {
    let key = handle.cache_key();
    let dragging = ctx.renderer_mut().scrollbar_drag_active(key);
    let thickness = if dragging {
        SCROLL_INDICATOR_DRAG_THICKNESS
    } else {
        SCROLL_INDICATOR_THICKNESS
    };
    let theme = widget_theme(env);
    let vertical = matches!(axis, ScrollAxis::Vertical | ScrollAxis::All)
        .then(|| {
            indicator_geometry(
                viewport.height(),
                metrics.viewport_height,
                metrics.content_height,
                metrics.max_y,
                metrics.offset_y,
            )
        })
        .flatten();
    let horizontal = matches!(axis, ScrollAxis::Horizontal | ScrollAxis::All)
        .then(|| {
            indicator_geometry(
                viewport.width(),
                metrics.viewport_width,
                metrics.content_width,
                metrics.max_x,
                metrics.offset_x,
            )
        })
        .flatten();

    {
        let mut draw = ctx.draw_context();
        if let Some(geometry) = &vertical {
            let thumb_y = viewport.y0 + geometry.thumb_offset;
            theme.draw_scroll_indicator(
                &mut draw,
                vello::kurbo::Rect::new(
                    viewport.x1 - SCROLL_INDICATOR_EDGE_INSET - thickness,
                    thumb_y,
                    viewport.x1 - SCROLL_INDICATOR_EDGE_INSET,
                    thumb_y + geometry.thumb_extent,
                ),
            );
        }
        if let Some(geometry) = &horizontal {
            let thumb_x = viewport.x0 + geometry.thumb_offset;
            theme.draw_scroll_indicator(
                &mut draw,
                vello::kurbo::Rect::new(
                    thumb_x,
                    viewport.y1 - SCROLL_INDICATOR_EDGE_INSET - thickness,
                    thumb_x + geometry.thumb_extent,
                    viewport.y1 - SCROLL_INDICATOR_EDGE_INSET,
                ),
            );
        }
    }

    let hit_transform = ctx.hit_transform;
    if vertical.is_some_and(|geometry| geometry.travel > 0.0) {
        let gutter = transformed_rect(
            hit_transform,
            vello::kurbo::Rect::new(
                viewport.x1 - SCROLL_INDICATOR_GUTTER,
                viewport.y0,
                viewport.x1,
                viewport.y1,
            ),
        );
        let handle = handle.clone();
        ctx.renderer_mut()
            .register_scrollbar_drag_target(gutter, move |renderer, point, _env| {
                let metrics = handle.metrics();
                let Some(geometry) = indicator_geometry(
                    gutter.height(),
                    metrics.viewport_height,
                    metrics.content_height,
                    metrics.max_y,
                    metrics.offset_y,
                ) else {
                    return false;
                };
                if geometry.travel <= 0.0 {
                    return false;
                }
                let pointer = point.y - gutter.y0;
                let grab = renderer.scrollbar_drag_grab(key).unwrap_or_else(|| {
                    let grab = if pointer >= geometry.thumb_offset
                        && pointer <= geometry.thumb_offset + geometry.thumb_extent
                    {
                        pointer - geometry.thumb_offset
                    } else {
                        geometry.thumb_extent / 2.0
                    };
                    renderer.begin_scrollbar_drag(key, grab);
                    grab
                });
                let target = ((pointer - grab) / geometry.travel).clamp(0.0, 1.0) * metrics.max_y;
                handle.scroll_to(metrics.offset_x, target)
            });
    }
    if horizontal.is_some_and(|geometry| geometry.travel > 0.0) {
        let gutter = transformed_rect(
            hit_transform,
            vello::kurbo::Rect::new(
                viewport.x0,
                viewport.y1 - SCROLL_INDICATOR_GUTTER,
                viewport.x1,
                viewport.y1,
            ),
        );
        let handle = handle.clone();
        ctx.renderer_mut()
            .register_scrollbar_drag_target(gutter, move |renderer, point, _env| {
                let metrics = handle.metrics();
                let Some(geometry) = indicator_geometry(
                    gutter.width(),
                    metrics.viewport_width,
                    metrics.content_width,
                    metrics.max_x,
                    metrics.offset_x,
                ) else {
                    return false;
                };
                if geometry.travel <= 0.0 {
                    return false;
                }
                let pointer = point.x - gutter.x0;
                let grab = renderer.scrollbar_drag_grab(key).unwrap_or_else(|| {
                    let grab = if pointer >= geometry.thumb_offset
                        && pointer <= geometry.thumb_offset + geometry.thumb_extent
                    {
                        pointer - geometry.thumb_offset
                    } else {
                        geometry.thumb_extent / 2.0
                    };
                    renderer.begin_scrollbar_drag(key, grab);
                    grab
                });
                let target = ((pointer - grab) / geometry.travel).clamp(0.0, 1.0) * metrics.max_x;
                handle.scroll_to(target, metrics.offset_y)
            });
    }
}
