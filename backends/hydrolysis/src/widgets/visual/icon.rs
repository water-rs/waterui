use std::cell::RefCell;
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::prelude::theme_color::{MutedForeground, Surface};
use waterui::shape::RoundedRectangle;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_icon::SystemIcon;
use waterui_layout::stack::zstack;

use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RetainedSubview, WidgetRenderContext,
    measure_view_dimensions_with_proposal, measure_view_intrinsic, normalize_view_for_render,
};

/// Side length of the unsupported-`SystemIcon` placeholder marker, in points.
/// Matches the Material standard icon size so the marker occupies the slot a real
/// icon would.
const ICON_SIZE: f32 = 24.0;
/// Outline thickness of the placeholder marker.
const ICON_BORDER: f32 = 2.0;
/// Corner radius of the placeholder marker.
const ICON_RADIUS: f32 = 6.0;

/// `SystemIcon` resolves names against an OS-provided icon catalog (SF Symbols).
/// Hydrolysis draws its own pixels and has no such catalog, so it cannot render the
/// real symbol. Rather than crashing, it renders a neutral "missing glyph" marker
/// (an outlined rounded square in theme tokens) that surfaces the asymmetry without
/// faking the icon — portable code should depend on a packaged icon crate
/// (`waterui-icons-material-icon`, `waterui-icons-lucide`, `waterui-icons-fontawesome7`)
/// instead. The marker carries the requested symbol name as its accessibility label.
fn system_icon_placeholder(icon: &SystemIcon, env: &Environment) -> AnyView {
    let inner = ICON_SIZE - ICON_BORDER * 2.0;
    normalize_view_for_render(
        AnyView::new(
            zstack((
                ().size(ICON_SIZE, ICON_SIZE)
                    .background(MutedForeground)
                    .clip(RoundedRectangle::new(ICON_RADIUS)),
                ().size(inner, inner)
                    .background(Surface)
                    .clip(RoundedRectangle::new(ICON_RADIUS - ICON_BORDER)),
            ))
            .a11y_role(AccessibilityRole::Image)
            .a11y_label(icon.name.as_str().to_owned()),
        ),
        env,
    )
}

/// The retained render state of a system icon: the placeholder marker is a
/// move-only `AnyView` (built once from the static icon name), so the persistent
/// `Widget` node holds it as a [`RetainedSubview`] built once and re-flushed each
/// frame. The marker's own dispatch drives accessibility (icon a11y is
/// render-driven).
pub(crate) struct IconRenderState {
    placeholder: RetainedSubview,
}

impl IconRenderState {
    pub(crate) fn from_icon(icon: &SystemIcon, env: &Environment) -> Self {
        Self {
            placeholder: RetainedSubview::new(system_icon_placeholder(icon, env)),
        }
    }

    /// Eagerly build the placeholder sub-view (the measure path has only `&mut
    /// HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.placeholder.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<SystemIcon> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_view_intrinsic(&system_icon_placeholder(view.as_inner(), env), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        measure_view_dimensions_with_proposal(
            &system_icon_placeholder(view.as_inner(), env),
            proposal,
            state,
            env,
        )
    }
}

/// Measures a retained icon leaf from its [`IconRenderState`]: the icon sizes
/// itself to the placeholder marker, mirroring the dispatch path's `dimensions`.
pub(crate) fn measure_icon_node(
    state: &IconRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(state.placeholder.measure_built(hydro, env))
}

/// Renders a retained icon leaf every flush: flushes the placeholder marker
/// sub-view, whose own dispatch drives accessibility (icon a11y is render-driven).
pub(crate) fn render_icon_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<IconRenderState>>,
    env: &Environment,
) {
    render_icon_parts(ctx, state, env);
}

pub(crate) fn render_icon_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<IconRenderState>>,
    env: &Environment,
) {
    let bounds = ctx.bounds;
    let render_ctx = ctx.render_context();
    let mut state = state.borrow_mut();
    state
        .placeholder
        .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
}
