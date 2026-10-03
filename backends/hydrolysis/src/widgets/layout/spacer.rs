use crate::renderer::{HydroNativeView, HydroState, WidgetRenderContext};
use std::cell::RefCell;
use std::rc::Rc;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_layout::spacer::Spacer;
use waterui_layout::stack::Axis;

/// The spacer leaf contract (layout-spec §5/§6): a spacer answers its
/// `min_length` on the enclosing stack's main axis and zero on the cross
/// axis, whatever the proposal — the stack injects `Axis` into the
/// environment it measures its children under and keeps that answer as the
/// flexible child's floor under compression. Outside a stack a spacer
/// claims nothing.
fn spacer_intrinsic(spacer: &Spacer, env: &Environment) -> LayoutSize {
    let min_length = spacer.min_length();
    match env.get::<Axis>() {
        Some(Axis::Horizontal) => LayoutSize::new(min_length, 0.0),
        Some(Axis::Vertical) => LayoutSize::new(0.0, min_length),
        _ => LayoutSize::zero(),
    }
}

impl HydroNativeView for Native<()> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<Spacer> {
    fn intrinsic(
        _state: &mut HydroState,
        view: &Self,
        env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        spacer_intrinsic(view.as_inner(), env)
    }
}

/// Measures a retained empty (`()`) leaf: zero intrinsic, matching the dispatch path.
pub(crate) fn measure_empty_node(
    _empty: &(),
    _proposal: ProposalSize,
    _state: &mut HydroState,
    _env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
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

/// Measures a retained spacer leaf: its minimum length on the enclosing
/// stack's main axis whatever the proposal (the answer is the floor the
/// stack keeps under compression, expansion happens at placement),
/// matching the dispatch path.
pub(crate) fn measure_spacer_node(
    spacer: &Spacer,
    _proposal: ProposalSize,
    _state: &mut HydroState,
    env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    ViewDimensions::new(spacer_intrinsic(spacer, env))
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
