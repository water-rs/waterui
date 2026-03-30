use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{Environment, Native};
use waterui_graphics::color::ResolvedColor;
use waterui_graphics::view_effect::ViewEffectErased;
use waterui_graphics::{GpuSurface, ResolvedGradient, SceneView};
use waterui_shape::ResolvedShape;

impl HydroNativeView for Native<GpuSurface> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_gpu_surface(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        _view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
            );
            if let Some(label) = renderer.resolve_accessibility_label(env, None) {
                node.set_label(label);
            }
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<SceneView> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_scene_view(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        _view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
            );
            if let Some(label) = renderer.resolve_accessibility_label(env, None) {
                node.set_label(label);
            }
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<ViewEffectErased> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_view_effect(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        crate::renderer::measure_view_intrinsic(view.as_inner().content(), state, env)
    }
}

impl HydroNativeView for Native<ResolvedColor> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_resolved_color(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<ResolvedGradient> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_resolved_gradient(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<ResolvedShape> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_resolved_shape(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}
