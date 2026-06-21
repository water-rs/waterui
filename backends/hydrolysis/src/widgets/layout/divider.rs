use crate::renderer::{HydroState, WidgetRenderContext};
use std::cell::RefCell;
use std::rc::Rc;
use waterui::widget::Divider;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_layout::stack::Axis as StackAxis;

use crate::widgets::util::widget_theme;

/// Measures a retained divider leaf: a 1×1 minimum, matching the dispatch path's
/// `measure_view_dimensions` for `Divider`. The divider stretches on its cross
/// axis during placement; its intrinsic size is the line thickness floor.
pub(crate) fn measure_divider_node(
    _divider: &Divider,
    _proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::new(1.0, 1.0))
}

/// Renders a retained divider leaf every flush: draws the separator line from the
/// theme metrics, oriented by the enclosing stack axis. No accessibility.
pub(crate) fn render_divider_node(
    ctx: &mut WidgetRenderContext<'_>,
    divider: &Rc<RefCell<Divider>>,
    env: &Environment,
) {
    render_divider_parts(ctx, divider, env);
}

pub(crate) fn render_divider_parts(
    ctx: &mut WidgetRenderContext<'_>,
    _divider: &Rc<RefCell<Divider>>,
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
