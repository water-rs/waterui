use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    circle_arc_path, measure_progress_intrinsic, measure_view_intrinsic, normalize_view_for_render,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use core::f64::consts::TAU;
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::Size as LayoutSize;

use super::util::widget_theme;

impl HydroNativeView for Native<ProgressConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        render_progress(ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_progress_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let progress = view.as_inner();
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ProgressIndicator),
            );
            let default_label = renderer.accessibility_label_from_view(&progress.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let value = renderer.read_signal(&progress.value).clamp(0.0, 1.0);
            node.set_numeric_value(value);
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(1.0);
            let bounds = crate::renderer::transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(node, bounds, env, None);
        }
    }
}

pub(crate) fn render_progress(
    ctx: &mut WidgetRenderContext<'_>,
    progress: Native<ProgressConfig>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let mut progress = progress.into_inner();
    progress.label = normalize_view_for_render(progress.label, env);
    progress.value_label = normalize_view_for_render(progress.value_label, env);
    let clamped = ctx
        .renderer_mut()
        .read_signal(&progress.value)
        .clamp(0.0, 1.0) as f64;

    match progress.style {
        ProgressStyle::Linear => {
            let metrics = theme.progress_metrics(ProgressStyle::Linear);
            let label_size = measure_view_intrinsic(&progress.label, ctx.state_mut(), env);
            let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
                f64::from(label_size.height).max(metrics.label_height)
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
                ctx.dispatch_in_rect_without_accessibility(env, progress.label, label_rect);
            }

            let bar_y = ctx.bounds.y0 + label_height + metrics.bar_top_offset;
            let bar_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0 + metrics.bar_horizontal_inset,
                bar_y,
                ctx.bounds.x1 - metrics.bar_horizontal_inset,
                bar_y + metrics.bar_height,
            );
            let fill_rect = vello::kurbo::Rect::new(
                bar_rect.x0,
                bar_rect.y0,
                bar_rect.x0 + bar_rect.width() * clamped,
                bar_rect.y1,
            );
            {
                let mut draw = ctx.draw_context();
                theme.draw_progress_linear_track(&mut draw, bar_rect);
                theme.draw_progress_linear_fill(&mut draw, fill_rect);
            }

            let value_label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                bar_rect.y1 + metrics.value_label_top_spacing,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            if value_label_rect.height() > 0.0 {
                ctx.dispatch_in_rect_without_accessibility(
                    env,
                    progress.value_label,
                    value_label_rect,
                );
            }
        }
        ProgressStyle::Circular => {
            let center = vello::kurbo::Point::new(
                ctx.bounds.x0 + ctx.bounds.width() / 2.0,
                ctx.bounds.y0 + ctx.bounds.height() / 2.0,
            );
            let radius = (ctx.bounds.width().min(ctx.bounds.height()) / 2.0 - 6.0).max(2.0);
            let arc = circle_arc_path(center, radius, -core::f64::consts::FRAC_PI_2, TAU * clamped);
            let mut draw = ctx.draw_context();
            theme.draw_progress_circular_track(&mut draw, center, radius, 5.0);
            theme.draw_progress_circular_fill(&mut draw, &arc, 5.0);
        }
        _ => {
            panic!("hydrolysis ProgressStyle variant is not implemented");
        }
    }
}
