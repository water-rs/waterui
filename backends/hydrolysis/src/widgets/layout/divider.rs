#[cfg(feature = "menu")]
use crate::num_cast::f64_as_f32;
use crate::renderer::{HydroState, WidgetRenderContext};
#[cfg(feature = "menu")]
use crate::renderer::{menu_look, popup_menu_divider_height};
#[cfg(feature = "menu")]
use crate::widgets::controls::button::MenuRowChrome;
use cherenkov_record::Draw as _;
use nami::SignalExt as _;
use std::cell::RefCell;
use std::rc::Rc;
use waterui_controls::Divider;
use waterui_controls::theme::color;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::resolve::Resolvable as _;
use waterui_layout::stack::Axis as StackAxis;

/// Measures a retained divider leaf: a 1×1 minimum, or the full proposed
/// width and theme divider row height inside a popup menu (`MenuRowChrome`).
/// The divider stretches on its cross axis during placement; its intrinsic
/// size is the line thickness floor.
// impl_widget_behavior dispatches every measure fn with a `&T` borrow,
// including the Copy ones.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn measure_divider_node<E: crate::text::TextEngine>(
    _divider: &Divider,
    proposal: ProposalSize,
    _state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    #[cfg(feature = "menu")]
    if env.get::<MenuRowChrome>().is_some() {
        let metrics = menu_look(theme, env).layout();
        let width = proposal.width.unwrap_or(1.0);
        let height = f64_as_f32(popup_menu_divider_height(metrics));
        return ViewDimensions::new(LayoutSize::new(width, height));
    }
    #[cfg(not(feature = "menu"))]
    let _ = (proposal, env, theme);
    ViewDimensions::new(LayoutSize::new(1.0, 1.0))
}

/// The divider's stroke thickness: 1 pt, as `docs/layout-spec.md` fixes it.
const DIVIDER_THICKNESS: f64 = 1.0;

/// Renders a retained divider leaf every flush: inside a popup menu
/// (`MenuRowChrome`), records the theme's `MenuLook::record_separator` over
/// `MenuLook::separator_rect(ctx.bounds)`; otherwise draws a 1 pt `Border`
/// line oriented by the enclosing stack axis. No accessibility.
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
    #[cfg(feature = "menu")]
    if env.get::<MenuRowChrome>().is_some() {
        let theme = ctx.theme();
        let menu = menu_look(&theme, env);
        let separator = menu.separator_rect(ctx.bounds);
        ctx.draw_context(|draw| {
            menu.record_separator(&mut *draw, separator);
        });
        return;
    }
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
