use crate::renderer::{HydroNativeView, HydroState, WidgetRenderContext};
use std::cell::RefCell;
use std::rc::Rc;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_layout::spacer::Spacer;

impl HydroNativeView for Native<()> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<Spacer> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

/// Measures a retained empty (`()`) leaf: zero intrinsic, matching the dispatch path.
pub(crate) fn measure_empty_node(
    _empty: &(),
    _proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::zero())
}

/// Renders a retained empty (`()`) leaf every flush: a no-op (it draws nothing and
/// has no accessibility), mirroring the dispatch path.
pub(crate) fn render_empty_node(
    ctx: &mut WidgetRenderContext<'_>,
    empty: &Rc<RefCell<()>>,
    env: &Environment,
) {
    render_empty_parts(ctx, empty, env);
}

pub(crate) fn render_empty_parts(
    _ctx: &mut WidgetRenderContext<'_>,
    _empty: &Rc<RefCell<()>>,
    _env: &Environment,
) {
}

/// Measures a retained spacer leaf: zero intrinsic (it expands during placement,
/// never from its intrinsic size), matching the dispatch path.
pub(crate) fn measure_spacer_node(
    _spacer: &Spacer,
    _proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::zero())
}

/// Renders a retained spacer leaf every flush: a no-op (it draws nothing and has
/// no accessibility), mirroring the dispatch path.
pub(crate) fn render_spacer_node(
    ctx: &mut WidgetRenderContext<'_>,
    spacer: &Rc<RefCell<Spacer>>,
    env: &Environment,
) {
    render_spacer_parts(ctx, spacer, env);
}

pub(crate) fn render_spacer_parts(
    _ctx: &mut WidgetRenderContext<'_>,
    _spacer: &Rc<RefCell<Spacer>>,
    _env: &Environment,
) {
}
