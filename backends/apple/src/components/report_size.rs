//! The `report_size` metadata: `Metadata<ReportSize>` wrapped around a child,
//! reporting the view's committed placed size into a [`Binding<Size>`] after
//! layout placement commits.
//!
//! A transparent `HostView` container — measure, stretch, priority, empty
//! state, response geometry, and placement proposal all answer for the mounted
//! child — that reads its committed `PlacedGeometry` after each layout pass
//! and schedules a post-commit write via [`cocoa_ui::core_animation::on_commit`]
//! when the committed size changed. Writing after commit guarantees watchers
//! (such as `when(size.map(...), ...)`) never trigger a synchronous nested
//! layout pass inside measurement or placement.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::view;
use cocoa_ui::{MainThreadMarker, PlatformView, Rect};
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_layout::ReportSize;

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The wrapper's live state: the mounted child and coalesced post-commit
/// size-reporting state.
struct ReportSizeState {
    child: Mounted,
    report: ReportSize,
    pending: Cell<Option<Size>>,
    scheduled: Cell<bool>,
}

impl core::fmt::Debug for ReportSizeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReportSizeState").finish_non_exhaustive()
    }
}

impl ReportSizeState {
    fn schedule_report(self: &Rc<Self>, mtm: MainThreadMarker, size: Size) {
        if !self.scheduled.get() && nami::Signal::snapshot(self.report.binding()) == size {
            return;
        }
        self.pending.set(Some(size));
        if self.scheduled.replace(true) {
            return;
        }
        let state = Rc::downgrade(self);
        cocoa_ui::core_animation::on_commit(mtm, move || {
            let Some(state) = state.upgrade() else {
                return;
            };
            state.scheduled.set(false);
            if let Some(placed) = state.pending.take() {
                state.report.report(placed);
            }
        });
    }
}

/// The wrapper's layout face: transparent — every answer is the child's.
struct ReportSizeSubView {
    state: Rc<ReportSizeState>,
}

impl core::fmt::Debug for ReportSizeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReportSizeSubView").finish_non_exhaustive()
    }
}

impl SubView for ReportSizeSubView {
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

/// Realizes `Metadata<ReportSize>` as a transparent wrapper that reports its
/// committed placed size after each layout pass commits.
pub fn realize_report_size_metadata(
    metadata: Metadata<ReportSize, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    if let Some(offer) = ctx.offer() {
        offer.pass_on();
    }
    let child = ctx.realize(metadata.content);
    let env = child.handler_env(ctx.env());
    let clip_shape = child.clip_shape_handle();
    let mounted = child.mount(host_view);
    crate::primary_content::forward(&host, mounted.view());
    view::set_translates_autoresizing(mounted.view(), true);

    let state = Rc::new(ReportSizeState {
        child: mounted,
        report: metadata.value,
        pending: Cell::new(None),
        scheduled: Cell::new(false),
    });

    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        ReportSizeSubView {
            state: Rc::clone(&state),
        },
    );

    // `HostView::layout` / `layoutSubviews` commits `geometry` first, then
    // runs this handler: place the child in the full bounds and schedule the
    // committed response size to be written once this transaction commits.
    let geometry = leaf.geometry();
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            view::set_frame(state.child.view(), view::bounds(host));
            if let Some(placed) = geometry.get() {
                state.schedule_report(mtm, *placed.response_rect.size());
            }
        }
    });

    leaf.keep(sink_guard);
    leaf.keep(state);
    leaf.set_handler_env(env);
    leaf.set_clip_shape_handle(clip_shape);
    leaf
}
