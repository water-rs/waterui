use crate::renderer::{HydroState, WidgetRenderContext};
use cherenkov_record::Draw as _;
use nami::SignalExt as _;
use std::cell::RefCell;
use std::rc::Rc;
use waterui::theme::color;
use waterui::widget::Divider;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::resolve::Resolvable as _;
use waterui_layout::stack::Axis as StackAxis;

/// Measures a retained divider leaf: a 1×1 minimum, matching the dispatch path's
/// `measure_view_dimensions` for `Divider`. The divider stretches on its cross
/// axis during placement; its intrinsic size is the line thickness floor.
// impl_widget_behavior dispatches every measure fn with a `&T` borrow,
// including the Copy ones.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn measure_divider_node<E: crate::text::TextEngine>(
    _divider: &Divider,
    _proposal: ProposalSize,
    _state: &mut HydroState<E>,
    _env: &Environment,
    _theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::new(1.0, 1.0))
}

/// The divider's stroke thickness: 1 pt, as `docs/layout-spec.md` fixes it.
const DIVIDER_THICKNESS: f64 = 1.0;

/// Renders a retained divider leaf every flush: draws the separator line 1 pt
/// thick in the `Border` token, oriented by the enclosing stack axis. No
/// accessibility.
pub fn render_divider_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    divider: &Rc<RefCell<Divider>>,
    env: &Environment,
) {
    render_divider_parts(ctx, divider, env);
}

pub fn render_divider_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    _divider: &Rc<RefCell<Divider>>,
    env: &Environment,
) {
    let vertical = matches!(env.get::<StackAxis>(), Some(StackAxis::Horizontal));
    let rect = if vertical {
        kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x0 + DIVIDER_THICKNESS,
            ctx.bounds.y1,
        )
    } else {
        kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            ctx.bounds.y0 + DIVIDER_THICKNESS,
        )
    };
    let border = ctx
        .renderer_mut()
        .read_signal(&color::Border.resolve(env).computed());

    ctx.draw_context(|draw| {
        draw.fill(rect, border);
    });
}
