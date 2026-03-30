use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use nami::Signal;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_text::TextConfig;

impl HydroNativeView for Native<TextConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_text_config(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.get(),
            view.as_inner().paragraph_alignment.get(),
            env,
            None,
            None,
        )
        .size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.get(),
            view.as_inner().paragraph_alignment.get(),
            env,
            proposal.width,
            None,
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
            let text = view.as_inner();
            let plain = renderer.read_signal(&text.content).to_plain().to_string();
            let default_label = (!plain.is_empty()).then_some(plain);
            let label = renderer.resolve_accessibility_label(env, default_label);
            let Some(label) = label else {
                return;
            };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Label),
            );
            node.set_label(label);
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
                None,
            );
        }
    }
}
