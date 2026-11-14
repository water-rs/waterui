//! The `fixed_container` leaf: `Native<FixedContainer>` rendered through a
//! [`HostView`].
//!
//! Mirrors `WuiFixedContainer`: a fixed array of children laid out inside
//! the host's safe area by the Rust layout engine, child frames
//! pixel-snapped, children that manage their own safe area extended through
//! it to the bounds edge they touch. The leaf itself answers that it manages
//! its safe area — the platform rule a fixed container holds — through the
//! `cocoaUiManagesSafeArea` selector the fallback's `wuiHandlesSafeArea`
//! reads.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use cocoa_ui::response::ReportsResponse;
use cocoa_ui::{PlatformView, Rect, Retained, view};
use waterui::layout::container::FixedContainer;
use waterui_backend_core::{AnyView, View};
use waterui_core::layout::{
    Layout, LayoutError, PlacedGeometry, ProposalSize, Rect as LayoutRect, ResolvedExtent,
    StretchAxis, SubView, ViewDimensions, measure_layout, place_layout, resolve_layout_extent,
};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal::{self, CommittedGeometry, Placement, to_proposal};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HitTest, HostView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView};

/// The leaf's live state: the layout engine, the mounted children, and the
/// placement a Rust parent last gave the container.
struct FixedState {
    /// The `FixedContainer`'s layout object.
    layout: Box<dyn Layout>,
    /// The rendered children, in layout order.
    children: Vec<Mounted>,
    /// How the parent layout placed this container: the proposal it
    /// selected — `None` while no Rust parent has placed us, and the
    /// container builds its own bounded offer from the rect it fills — and
    /// the offset its pixel alignment left.
    placement: Cell<Placement>,
    /// The host view, for layout invalidation inside watchers.
    host: Retained<HostView>,
    /// The leaf's committed geometry, which each layout pass commits from
    /// its `place_layout` answer — the leaf's own once it is rendered.
    geometry: CommittedGeometry,
}

impl core::fmt::Debug for FixedState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FixedState").finish_non_exhaustive()
    }
}

/// The children as the layout engine sees them.
fn children_of(state: &FixedState) -> Vec<&dyn SubView> {
    state.children.iter().map(Mounted::layout).collect()
}

/// `containerMeasure` / `placements`: the measurement the leaf reports.
fn measure(state: &FixedState, proposal: ProposalSize) -> ViewDimensions {
    measure_layout(&*state.layout, proposal, &children_of(state))
}

/// `performLayout`: place every child inside the boundary the regions it
/// does not ignore leave, then run each child's extension rule — an ignorer
/// reaches the deepest boundary its declaration releases, a scroll surface
/// or a background-slot fill reaches the window edge on the edges it
/// touches, a safe-area manager reaches the bounds it touches — and snap to
/// pixels.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn perform_layout(state: &Rc<RefCell<FixedState>>) {
    let host = {
        let state = state.borrow();
        if state.children.is_empty() {
            return;
        }
        state.host.clone()
    };
    let host_view: &PlatformView = &host;
    // The region context is computed once for the pass and handed to
    // every child's extension rule — the ancestor walk runs at the
    // boundary, not per child.
    let context = crate::native_layout::LayoutContext::of(host_view);
    let safe_rect = context.safe_rect();
    let (placements, moved) = {
        let state = state.borrow();
        // Placed by a Rust parent: the proposal it selected. Natively hosted:
        // the boundary offer for the rect this container fills.
        let proposal = state.placement.get().proposal.unwrap_or_else(|| {
            ProposalSize::new(
                Some(safe_rect.size.width as f32),
                Some(safe_rect.size.height as f32),
            )
        });
        let placements = place_layout(
            &*state.layout,
            proposal::layout_rect(safe_rect),
            proposal,
            &children_of(&state),
        )
        .expect("a fixed container's layout resolves its response");
        // The container's response and its children's placements come from
        // one answer; the leaf retains that geometry with the placement.
        // Where the parent anchors the response elsewhere in the
        // allocation, the children move with it.
        let allocation = proposal::allocation(&host);
        let moved = state.geometry.commit(
            host.response_geometry(),
            PlacedGeometry {
                proposal,
                answer: placements.answer,
                allocation,
                response_rect: placements
                    .response_rect
                    .translated(-allocation.x(), -allocation.y()),
            },
        );
        (placements, moved)
    };

    let state = state.borrow();
    for (index, (child, placement)) in state
        .children
        .iter()
        .zip(placements.children.iter())
        .enumerate()
    {
        let placed_frame = placement.frame.translated(moved.x, moved.y);
        let frame = Rect::new(
            f64::from(placed_frame.x()),
            f64::from(placed_frame.y()),
            f64::from(placed_frame.width()),
            f64::from(placed_frame.height()),
        );
        if !frame.is_valid_for_layout() {
            waterui::contract_violation!(
                "fixed container received an invalid layout rect for child {index}: {frame:?}; \
                 the child is not placed"
            );
            continue;
        }
        let written = context.extend_child(
            child.view(),
            frame,
            state.host.background_slot() == Some(index),
        );
        proposal::place_painted(
            child.view(),
            written.frame,
            written.painted_past,
            Some(placement.proposal),
            state.placement.get().offset,
        );
    }
}

/// The container's layout face: measures through the layout engine, claims
/// the axis the layout answers for the live child axes, never empty.
struct FixedSubView {
    /// The leaf's state.
    state: Rc<RefCell<FixedState>>,
}

impl core::fmt::Debug for FixedSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FixedSubView").finish_non_exhaustive()
    }
}

impl SubView for FixedSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        let state = self.state.borrow();
        resolve_layout_extent(&*state.layout, bounds, proposal, &children_of(&state))
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure(&self.state.borrow(), proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        let state = self.state.borrow();
        let axes: Vec<StretchAxis> = state
            .children
            .iter()
            .map(|child| child.layout().stretch_axis())
            .collect();
        state.layout.stretch_axis(&axes)
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `fixed_container` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    // A `.background()` desugars to
    // `FixedContainer::new(BackgroundLayout, (background, content))` —
    // claimed as a plain view before `body()` hides the layout inside
    // `DirectionalLayout`, so the host can mark slot 0 as the background
    // the §7.1 fill rule extends through touched-edge regions.
    dispatcher.register_view::<FixedContainer>(|container, ctx| {
        let background = (container.as_parts().0 as &dyn core::any::Any)
            .is::<waterui::layout::BackgroundLayout>();
        let leaf = ctx.render(AnyView::new(container.body(ctx.env())));
        if background {
            let host = leaf.view().downcast_ref::<HostView>().expect(
                "a `BackgroundLayout` container must mount a `HostView` — the fill rule marks the background slot on it",
            );
            host.set_background_slot(Some(0));
        }
        leaf
    });

    dispatcher.register_native::<FixedContainer>(|container, ctx| {
        let mtm = ctx.mtm();
        let (layout, contents) = container.into_inner();
        let host = HostView::new(mtm, Rect::ZERO);

        let mut children = Vec::with_capacity(contents.len());
        for content in contents {
            let mounted = ctx.render(content).mount(&host);
            view::set_translates_autoresizing(mounted.view(), true);
            children.push(mounted);
        }

        let state = Rc::new(RefCell::new(FixedState {
            layout,
            children,
            placement: Cell::new(Placement::UNPLACED),
            host: host.clone(),
            geometry: CommittedGeometry::default(),
        }));

        host.set_hit_test_handler(|_host, _point| HitTest::PassIfSelf);
        // A fixed container owns its children's safe-area question: they lay
        // out inside the host's safe area and the ones managing it extend
        // through to the bounds they touch.
        host.set_manages_safe_area(true);
        host.set_primary_content_handler({
            let state = Rc::clone(&state);
            move |_host| {
                state
                    .borrow()
                    .children
                    .first()
                    .map(|child| view::retain_base(child.view()))
            }
        });
        // The scroll-surface search descends through this container's
        // children, in stacking order.
        host.set_scroll_surface_handler({
            let state = Rc::clone(&state);
            move |_host| {
                state
                    .borrow()
                    .children
                    .iter()
                    .map(|child| view::retain_base(child.view()))
                    .collect()
            }
        });

        let measure_state = Rc::clone(&state);
        host.set_measure_handler(move |_host, proposal| {
            let state = measure_state.borrow();
            let measured = measure(&state, to_proposal(proposal));
            cocoa_ui::Size::new(
                f64::from(measured.size.width),
                f64::from(measured.size.height),
            )
        });

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |_host| {
                perform_layout(&state);
            }
        });

        // The placement a Rust parent gave invalidates layout even when the
        // frame does not move — equal bounds under a different offer can
        // produce a different child layout, and under a different offset a
        // different pixel alignment of the children.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            let host = host.clone();
            move |placement| {
                let state = state.borrow();
                if state.placement.get() != placement {
                    state.placement.set(placement);
                    host.set_needs_layout();
                }
            }
        });

        // The layout's own invalidation signal — a reactive constraint (a
        // frame's computed bound, padding's computed inset) re-marks the host
        // and its ancestors for layout when it publishes a new value.
        let layout_guards = state.borrow().layout.watch_invalidation(Rc::new({
            let host = host.clone();
            move || {
                crate::invalidation::invalidate_layout_hierarchy(&host);
            }
        }));

        let mut leaf = NativeLeaf::new(
            &*host,
            FixedSubView {
                state: Rc::clone(&state),
            },
        );
        state.borrow_mut().geometry = leaf.drive_geometry();
        leaf.keep(sink_guard);
        leaf.keep(layout_guards);
        leaf.keep(state);
        leaf
    });
}
