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
    PlatformView, PlatformViewPlacement, PlatformViewSink, PlatformViewSource,
    next_platform_view_id,
};
use crate::renderer::{
    HydroNativeView, HydroState, WidgetRenderContext, graphics_dimensions_from_proposal,
};

/// Where a leaf's mounted view comes from — and, for an instance, the
/// object whose lifetime keeps it alive: the invariant lives in the type.
pub enum PlacementSource {
    /// A host-side factory resolves `kind` into a fresh child view.
    Factory(Box<str>),
    /// The host already holds the native view under `id`; `owner` is what
    /// keeps the peer alive — dropping the leaf drops it, which releases
    /// the native view. The Android system-WebView bridge mounts this way.
    #[cfg(hydrolysis_android_system_webview)]
    Instance {
        id: u64,
        owner: Rc<dyn core::any::Any>,
    },
}

impl PlacementSource {
    /// The serialized source carried in the published placement.
    fn wire(&self) -> PlatformViewSource {
        match self {
            Self::Factory(kind) => PlatformViewSource::Factory { kind: kind.clone() },
            #[cfg(hydrolysis_android_system_webview)]
            Self::Instance { id, owner } => {
                // The owner is the point: the leaf carries it so the native
                // peer outlives the placement that mounts it.
                debug_assert!(Rc::strong_count(owner) >= 1);
                PlatformViewSource::Instance { instance: *id }
            }
        }
    }
}

/// The retained state of one platform-view leaf: what the host mounts, the
/// stable placement id and the sink the runner installed.
pub struct PlatformViewRenderState {
    id: u64,
    source: PlacementSource,
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
            source: PlacementSource::Factory(view.kind().into()),
            sink,
        }
    }

    /// A leaf whose placement mounts the native view the host registered
    /// under `instance`. `owner` is the object the leaf keeps alive while
    /// mounted — dropping it releases the native view.
    ///
    /// Only the Android system-WebView bridge creates these today.
    #[cfg(hydrolysis_android_system_webview)]
    pub fn from_instance(instance: u64, owner: Rc<dyn core::any::Any>, env: &Environment) -> Self {
        let sink = env
            .get::<PlatformViewSink>()
            .cloned()
            .unwrap_or_else(|| crate::renderer::unsupported_platform_view());
        Self {
            id: next_platform_view_id(),
            source: PlacementSource::Instance {
                id: instance,
                owner,
            },
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
    // The leaf registers its node-local frame plus the sink table handle;
    // materialization resolves the window rect, clip, visibility and paint
    // rank and writes the record into the table.
    let local_bounds = ctx.bounds;
    let state = state.borrow();
    ctx.renderer_mut().register_platform_view_placement(
        local_bounds,
        Rc::clone(&state.sink.table),
        PlatformViewPlacement {
            id: state.id,
            source: state.source.wire(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            clip: None,
            order: 0,
            visible: false,
        },
    );
}
