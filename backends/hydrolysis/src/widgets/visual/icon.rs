use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{Environment, Native};
use waterui_icon::SystemIcon;

/// `SystemIcon` resolves names against an OS-provided icon catalog (SF
/// Symbols). Hydrolysis draws its own pixels and has no such catalog, so the
/// primitive is explicitly unsupported here: portable code should use a
/// packaged icon crate (`waterui-icons-material-icon`,
/// `waterui-icons-lucide`, `waterui-icons-fontawesome7`).
fn unsupported_system_icon(icon: &SystemIcon) -> ! {
    panic!(
        "SystemIcon(\"{}\") requires an OS icon catalog and is not supported by Hydrolysis; \
         use a packaged icon crate such as waterui-icons-material-icon for portable code",
        icon.name
    )
}

impl HydroNativeView for Native<SystemIcon> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let _ = (ctx, env);
        unsupported_system_icon(view.as_inner());
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let _ = (state, env);
        unsupported_system_icon(view.as_inner());
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
            let label =
                renderer.resolve_accessibility_label(env, Some(icon.name.as_str().to_owned()));
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
