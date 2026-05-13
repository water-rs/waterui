use crate::renderer::{HydrolysisRenderer, RenderContext, WidgetRenderContext};
use waterui::widget::Divider;
use waterui_core::Environment;
use waterui_layout::stack::Axis as StackAxis;

use crate::widgets::util::widget_theme;

pub(crate) fn render_divider(
    ctx: &mut WidgetRenderContext<'_>,
    _divider: Divider,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let metrics = theme.divider_metrics();
    let vertical = matches!(env.get::<StackAxis>(), Some(StackAxis::Horizontal));
    let rect = if vertical {
        vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x0 + metrics.thickness,
            ctx.bounds.y1,
        )
    } else {
        vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            ctx.bounds.y0 + metrics.thickness,
        )
    };

    let mut draw = ctx.draw_context();
    theme.draw_divider(&mut draw, rect);
}

pub(crate) fn render_divider_with_renderer(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    divider: Divider,
    env: &Environment,
) {
    let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
    render_divider(&mut widget_ctx, divider, env);
}
