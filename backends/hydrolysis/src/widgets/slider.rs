#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::slider_step_for_range;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_slider_intrinsic, measure_view_intrinsic, normalize_view_for_render,
    slider_value_epsilon, transformed_rect,
};
use waterui_core::AnyView;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui_controls::slider::SliderConfig;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::Size as LayoutSize;

use super::util::widget_theme;

impl HydroNativeView for Native<SliderConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_slider(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_slider_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let slider = view.as_inner();
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Slider),
            );
            let default_label = renderer.accessibility_label_from_label(&slider.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let start = *slider.range.start();
            let end = *slider.range.end();
            assert!(start < end, "hydrolysis slider requires range start < end");
            let current = renderer.read_signal(&slider.value).clamp(start, end);
            node.set_numeric_value(current);
            node.set_min_numeric_value(start);
            node.set_max_numeric_value(end);
            node.set_numeric_value_step(slider_step_for_range(slider.range.clone()));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Increment);
            node.add_action(AccessibilityAction::Decrement);
            node.add_action(AccessibilityAction::SetValue);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::Slider {
                    value: slider.value.clone(),
                    range: slider.range.clone(),
                    step: slider_step_for_range(slider.range.clone()),
                }),
            );
        }
    }
}

pub(crate) fn render_slider(
    ctx: &mut WidgetRenderContext<'_>,
    slider: Native<SliderConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.slider_metrics();
    let mut slider = slider.into_inner();
    let label_view = normalize_view_for_render(AnyView::new(slider.label), env);
    slider.min_value_label = normalize_view_for_render(slider.min_value_label, env);
    slider.max_value_label = normalize_view_for_render(slider.max_value_label, env);
    let label_height = if ctx.bounds.height() >= 36.0 {
        f64::from(measure_view_intrinsic(&label_view, ctx.state_mut(), env).height).max(20.0)
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        ctx.dispatch_in_rect_without_accessibility(env, label_view, label_rect);
    }

    let min_label_size = measure_view_intrinsic(&slider.min_value_label, ctx.state_mut(), env);
    let max_label_size = measure_view_intrinsic(&slider.max_value_label, ctx.state_mut(), env);
    let min_label_width = f64::from(min_label_size.width);
    let max_label_width = f64::from(max_label_size.width);
    let min_label_x0 = ctx.bounds.x0 + metrics.horizontal_inset;
    let min_label_x1 = min_label_x0 + min_label_width;
    let max_label_x1 = ctx.bounds.x1 - metrics.horizontal_inset;
    let max_label_x0 = max_label_x1 - max_label_width;
    let control_top = ctx.bounds.y0 + label_height;
    let control_bottom = ctx.bounds.y1;
    let control_height = control_bottom - control_top;

    if min_label_width > 0.0 && control_height > 0.0 {
        let min_label_rect =
            vello::kurbo::Rect::new(min_label_x0, control_top, min_label_x1, control_bottom);
        ctx.dispatch_in_rect_without_accessibility(env, slider.min_value_label, min_label_rect);
    }
    if max_label_width > 0.0 && control_height > 0.0 {
        let max_label_rect =
            vello::kurbo::Rect::new(max_label_x0, control_top, max_label_x1, control_bottom);
        ctx.dispatch_in_rect_without_accessibility(env, slider.max_value_label, max_label_rect);
    }

    let range_start = *slider.range.start();
    let range_end = *slider.range.end();
    let span = range_end - range_start;
    assert!(span > 0.0, "hydrolysis slider requires range start < end");

    let track_left = if min_label_width > 0.0 {
        min_label_x1 + metrics.horizontal_spacing
    } else {
        ctx.bounds.x0 + metrics.horizontal_inset
    };
    let track_right = if max_label_width > 0.0 {
        max_label_x0 - metrics.horizontal_spacing
    } else {
        ctx.bounds.x1 - metrics.horizontal_inset
    };
    let track_center_y = control_top + control_height / 2.0;
    let track_rect = vello::kurbo::Rect::new(
        track_left,
        track_center_y - metrics.track_height / 2.0,
        track_right,
        track_center_y + metrics.track_height / 2.0,
    );

    let clamped = ctx
        .renderer_mut()
        .read_signal(&slider.value)
        .clamp(range_start, range_end);
    let progress = (clamped - range_start) / span;
    let fill_right = track_left + (track_right - track_left) * progress;
    let fill_rect = vello::kurbo::Rect::new(
        track_left,
        track_center_y - metrics.track_height / 2.0,
        fill_right,
        track_center_y + metrics.track_height / 2.0,
    );
    {
        let mut draw = ctx.draw_context();
        theme.draw_slider_track(&mut draw, track_rect, fill_rect);
        theme.draw_slider_thumb(
            &mut draw,
            vello::kurbo::Point::new(fill_right, track_center_y),
            metrics.thumb_radius,
        );
    }

    let hit_bounds = transformed_rect(
        ctx.hit_transform,
        vello::kurbo::Rect::new(
            track_left - metrics.thumb_radius,
            control_top,
            track_right + metrics.thumb_radius,
            control_bottom,
        ),
    );
    let value_binding = slider.value;
    let usable_track = track_right - track_left;
    assert!(
        usable_track > 0.0,
        "hydrolysis slider resolved a non-positive track width"
    );
    let inverse_transform = ctx.hit_transform.inverse();
    let value_epsilon = slider_value_epsilon(span, usable_track);
    ctx.renderer_mut()
        .register_pointer_drag_target(hit_bounds, move |_renderer, point, _env| {
            let local_point = inverse_transform * point;
            let x = local_point.x.clamp(track_left, track_right);
            let t = (x - track_left) / usable_track;
            let next = range_start + span * t;
            if (value_binding.get() - next).abs() <= value_epsilon {
                return false;
            }
            value_binding.set(next);
            true
        });
}
