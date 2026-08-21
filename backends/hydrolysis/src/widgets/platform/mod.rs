use waterui_core::Environment;

use crate::renderer::WidgetRenderContext;

#[cfg(any(hydrolysis_cef_webview, feature = "chromium"))]
pub(crate) mod browser_cef;
#[cfg(feature = "chromium")]
pub(crate) mod chromium;
pub(crate) mod webview;

/// Publishes the accessibility node for a component whose content is web page
/// content.
///
/// The engine renders the page into a texture or a native subview, neither of
/// which the host tree can see into, so the component itself has to appear or
/// the region is invisible to a screen reader. Shared by every such component —
/// `WebView` and `ChromiumView` alike — because the node they owe the tree is
/// the same one: the role (a group unless `a11y_role` overrides it), the label
/// from `a11y_label`, and the bounds.
pub(crate) fn register_web_surface_accessibility(
    ctx: &mut WidgetRenderContext<'_>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};

        use crate::renderer::transformed_rect;

        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let renderer = ctx.renderer_mut();
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Group),
        );
        if let Some(label) = renderer.resolve_accessibility_label(env, None) {
            node.set_label(label);
        }
        let _ = renderer.register_accessibility_node(node, bounds, env, None);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (ctx, env);
    }
}
