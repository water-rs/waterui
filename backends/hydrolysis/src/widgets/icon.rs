use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{Environment, Native};
use waterui_icon::SystemIcon;
use waterui_text::styled::StyledStr;

impl HydroNativeView for Native<SystemIcon> {
    fn render(
        ctx: &mut WidgetRenderContext<'_>,
        view: Self,
        env: &Environment
    ) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_system_icon(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            StyledStr::plain(view.as_inner().name.clone()),
            env,
        )
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let icon = view.as_inner();
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
            );
            let label = renderer.resolve_accessibility_label(env, Some(icon.name.as_str().to_owned()));
            if let Some(label) = label {
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
