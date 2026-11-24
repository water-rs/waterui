//! The `dynamic_range` metadata: `Metadata<DynamicRangeMetadata>` wrapped
//! around content whose backing layer tree gets a `preferredDynamicRange` choice.
//!
//! A transparent `HostView` container — measure, stretch, priority and the
//! placement proposal all answer for the mounted child — that tags itself and
//! its layer tree with the mode at construction, updates when the reactive
//! signal changes, and re-applies it on every layout pass so later-attached
//! sublayers inherit it while a layer already carrying its own tag keeps its
//! local mode.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::dynamic_range::{self, DynamicRange};
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui::Signal;
use waterui::metadata::secure::{DynamicRange as WaterDynamicRange, DynamicRangeMetadata};
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_core::{Metadata, contract_violation};

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::proposal;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child and current dynamic-range mode.
struct DynamicRangeState {
    /// The mounted content.
    child: Mounted,
    /// Current resolved mode re-applied on layout passes and reactive updates.
    mode: Cell<DynamicRange>,
}

impl core::fmt::Debug for DynamicRangeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicRangeState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct DynamicRangeSubView {
    /// The leaf's state.
    state: Rc<DynamicRangeState>,
}

impl core::fmt::Debug for DynamicRangeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicRangeSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for DynamicRangeSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state.child.layout().resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

fn to_cocoa(range: WaterDynamicRange) -> DynamicRange {
    match range {
        WaterDynamicRange::Standard => DynamicRange::Standard,
        WaterDynamicRange::High => DynamicRange::High,
        other => {
            contract_violation!("unknown DynamicRange variant: {other:?}; falling back to High");
            DynamicRange::High
        }
    }
}

/// Realizes `Metadata<DynamicRangeMetadata>` as a transparent wrapper that
/// tags its backing layer tree with the requested dynamic range and updates
/// in place when the reactive range changes.
pub fn realize_dynamic_range_metadata(
    metadata: Metadata<DynamicRangeMetadata, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    let mounted = ctx.realize(metadata.content).mount(host_view);
    crate::primary_content::forward(&host, mounted.view());
    view::set_translates_autoresizing(mounted.view(), true);

    let signal = metadata.value.range();
    let initial_mode = to_cocoa(signal.snapshot());
    dynamic_range::apply_to_view(initial_mode, host_view);

    let state = Rc::new(DynamicRangeState {
        child: mounted,
        mode: Cell::new(initial_mode),
    });

    // Each layout pass stretches the content over the bounds and re-applies
    // the mode so newly attached sublayers inherit the override.
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            let bounds = view::bounds(host);
            view::set_frame(state.child.view(), bounds);
            dynamic_range::apply_to_view(state.mode.get(), host);
        }
    });

    // `setPlacementProposal`: the proposal selected for this wrapper is the
    // proposal its content was negotiated with.
    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        DynamicRangeSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.watch(signal, {
        let state = Rc::clone(&state);
        let host = host.clone();
        move |wctx| {
            let next_mode = to_cocoa(*wctx.value());
            state.mode.set(next_mode);
            dynamic_range::apply_to_view(next_mode, &host);
        }
    });
    leaf.keep(state);
    leaf
}
