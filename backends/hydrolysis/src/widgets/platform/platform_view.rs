//! The `PlatformView` leaf, as this backend bridges it.
//!
//! The leaf is placement-only: each flush it records its window-space frame,
//! effective clip and paint order onto the session's [`PlatformViewSink`], and
//! the host turns that into a mounted native child. Nothing is drawn here —
//! the mounted view owns its pixels, its input and its own accessibility
//! subtree, so the leaf also emits no accessibility node (the host grafts the
//! real child onto the virtual tree instead of projecting a duplicate).
//!
//! Reaching this leaf on a runner that installed no sink means the host
//! cannot embed native children — a programmer error, and it panics at node
//! build naming the missing piece.

use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};

use crate::platform_view::{
    PlatformView, PlatformViewPlacement, PlatformViewSink, next_platform_view_id,
};
use crate::renderer::{
    HydroNativeView, HydroState, WidgetRenderContext, graphics_dimensions_from_proposal,
    transformed_rect,
};

/// The retained state of one platform-view leaf: the factory key, the stable
/// placement id and the sink the runner installed.
pub struct PlatformViewRenderState {
    id: u64,
    kind: Box<str>,
    sink: PlatformViewSink,
}

impl PlatformViewRenderState {
    pub fn from_config(view: &PlatformView, env: &Environment) -> Self {
        let sink = env
            .get::<PlatformViewSink>()
            .cloned()
            .unwrap_or_else(|| crate::renderer::unsupported_platform_view());
        Self {
            id: next_platform_view_id(),
            kind: view.kind().into(),
            sink,
        }
    }
}

impl HydroNativeView for Native<PlatformView> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

/// Measures a platform-view leaf: it fills the proposal — the mounted child
/// takes whatever the layout assigned.
pub fn measure_platform_view_node(
    state: &PlatformViewRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let _ = (state, hydro, env, theme);
    graphics_dimensions_from_proposal(proposal)
}

/// Records the leaf's placement every flush: its frame in window hit-test
/// space, clipped to the open paint layers exactly as hit targets are, with a
/// paint order the host stacks embedded children by. The sink republishes the
/// set once the frame's encode completes, so a frame that never re-flushes
/// keeps the previous placements.
pub fn render_platform_view_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<PlatformViewRenderState>>,
    env: &Environment,
) {
    let _ = env;
    let frame = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (clipped, order) = ctx.renderer_mut().platform_view_placement(frame);
    #[allow(clippy::cast_possible_truncation)]
    let clip = (clipped != frame).then(|| {
        [
            clipped.x0 as f32,
            clipped.y0 as f32,
            clipped.width() as f32,
            clipped.height() as f32,
        ]
    });
    let state = state.borrow();
    state.sink.table.borrow_mut().record(PlatformViewPlacement {
        id: state.id,
        kind: state.kind.clone(),
        #[allow(clippy::cast_possible_truncation)]
        x: frame.x0 as f32,
        #[allow(clippy::cast_possible_truncation)]
        y: frame.y0 as f32,
        #[allow(clippy::cast_possible_truncation)]
        width: frame.width() as f32,
        #[allow(clippy::cast_possible_truncation)]
        height: frame.height() as f32,
        clip,
        #[allow(clippy::cast_possible_truncation)]
        order: order as u32,
        visible: !clipped.is_zero_area(),
    });
}
