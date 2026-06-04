#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RetainScrollFrameRequest, WidgetRenderContext,
    measure_view_dimensions_with_proposal, measure_view_intrinsic, normalize_layout_view,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::ProposalSize;
use waterui_core::layout::Size as LayoutSize;
use waterui_layout::scroll::{Axis as ScrollAxis, ScrollView};

use crate::widgets::widget_theme;

impl HydroNativeView for Native<ScrollView> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_scroll_view(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (_axis, content) = view.as_inner().as_parts();
        measure_view_intrinsic(content, state, env)
    }

    fn accessibility_is_render_driven() -> bool {
        true
    }

    fn accessibility(
        _renderer: &mut HydrolysisRenderer,
        _ctx: crate::renderer::RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
    }
}

pub(crate) fn render_scroll_view(
    ctx: &mut WidgetRenderContext<'_>,
    scroll: Native<ScrollView>,
    env: &Environment,
) {
    let (axis, content) = scroll.into_inner().into_inner();
    let content = normalize_layout_view(content, env);
    let viewport = ctx.bounds;
    let intrinsic = measure_scroll_content_intrinsic(
        axis,
        viewport,
        &content,
        ctx.renderer_mut().state_mut(),
        env,
    );
    let (content_width, content_height) = scroll_content_size(axis, viewport, intrinsic);

    let handle = ctx.renderer_mut().bind_render_scroll_handle(
        axis,
        viewport.width(),
        viewport.height(),
        content_width,
        content_height,
    );
    let metrics = handle.metrics();
    let transform = ctx.transform;
    let hit_transform = ctx.hit_transform;
    let active_layers = ctx.renderer_mut().active_scene_layers_snapshot();
    let exclusive_root_scroll = active_layers.is_empty() && ctx.renderer_mut().scene_is_empty();
    let needs_viewport_clip =
        !exclusive_root_scroll || !ctx.renderer_mut().viewport_matches_window_bounds(viewport);

    let content_transform = vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
    let content_bounds =
        vello::kurbo::Rect::new(0.0, 0.0, metrics.content_width, metrics.content_height);
    let visible_lazy_viewport = vello::kurbo::Rect::new(
        metrics.offset_x,
        metrics.offset_y,
        metrics.offset_x + viewport.width(),
        metrics.offset_y + viewport.height(),
    );
    let lazy_viewport = visible_lazy_viewport;
    if needs_viewport_clip {
        ctx.push_layer_rect(1.0, viewport);
    }
    ctx.renderer_mut().push_lazy_viewport(lazy_viewport);
    let content_ctx = ctx.child(content_transform, content_bounds);
    let renderer = ctx.renderer_mut();
    let content_render = renderer.render_scroll_content(
        handle.cache_key(),
        lazy_viewport,
        content_ctx,
        env,
        content,
    );
    renderer.retain_scroll_frame(RetainScrollFrameRequest {
        handle: handle.clone(),
        cache_key: handle.cache_key(),
        axis,
        viewport,
        transform,
        hit_transform,
        content_dynamic_morphs: content_render.dynamic_morphs,
        active_layers,
        exclusive_root: exclusive_root_scroll,
    });
    ctx.renderer_mut().pop_lazy_viewport("render_scroll_view");
    if needs_viewport_clip {
        ctx.pop_layer();
    }

    let target_handle = handle.clone();
    ctx.renderer_mut().register_scroll_target(
        transformed_rect(hit_transform, viewport),
        move |dx, dy, is_line_delta| target_handle.apply_scroll_delta(dx, dy, is_line_delta),
    );
    register_scroll_accessibility_node(
        ctx.renderer_mut(),
        env,
        transformed_rect(hit_transform, viewport),
        &handle,
        metrics,
        axis,
    );

    draw_scroll_indicators(ctx, env, viewport, metrics, axis);
}

fn measure_scroll_content_intrinsic(
    axis: ScrollAxis,
    viewport: vello::kurbo::Rect,
    content: &waterui_core::AnyView,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let proposal = match axis {
        ScrollAxis::Horizontal => ProposalSize::new(None, Some(viewport.height() as f32)),
        ScrollAxis::Vertical => ProposalSize::new(Some(viewport.width() as f32), None),
        ScrollAxis::All => ProposalSize::UNSPECIFIED,
        _ => panic!("scroll axis variant is not supported by hydrolysis"),
    };
    measure_view_dimensions_with_proposal(content, proposal, state, env).size
}

fn scroll_content_size(
    axis: ScrollAxis,
    viewport: vello::kurbo::Rect,
    intrinsic: LayoutSize,
) -> (f64, f64) {
    match axis {
        ScrollAxis::Horizontal => (
            f64::from(intrinsic.width).max(viewport.width()),
            viewport.height(),
        ),
        ScrollAxis::Vertical => (
            viewport.width(),
            f64::from(intrinsic.height).max(viewport.height()),
        ),
        ScrollAxis::All => (
            f64::from(intrinsic.width).max(viewport.width()),
            f64::from(intrinsic.height).max(viewport.height()),
        ),
        _ => panic!("scroll axis variant is not supported by hydrolysis"),
    }
}

pub(crate) fn register_scroll_accessibility_node(
    renderer: &mut HydrolysisRenderer,
    env: &Environment,
    bounds: vello::kurbo::Rect,
    handle: &crate::scroll::ScrollHandle,
    metrics: crate::scroll::ScrollMetrics,
    axis: ScrollAxis,
) {
    #[cfg(feature = "accessibility")]
    {
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
        ScrollAxis::Vertical | ScrollAxis::All => {
            if metrics.max_y > 0.0 {
                let track_height = viewport.height();
                let min_thumb_height = track_height.min(12.0);
                let thumb_height = (track_height
                    * (metrics.viewport_height / metrics.content_height))
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
        }
        _ => {}
    }

    match axis {
        ScrollAxis::Horizontal | ScrollAxis::All => {
            if metrics.max_x > 0.0 {
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
        }
        _ => {}
    }
}
