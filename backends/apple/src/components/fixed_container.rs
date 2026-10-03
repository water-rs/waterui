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

use cocoa_ui::geometry::MeasureProposal;
use cocoa_ui::{PlatformView, Rect, Retained, view};
use waterui::layout::container::FixedContainer;
use waterui_core::layout::{
    Layout, ProposalSize, StretchAxis, SubView, ViewDimensions, measure_layout,
    with_memoized_children,
};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HitTest, HostView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView};

/// The leaf's live state: the layout engine, the mounted children, and the
/// proposal a Rust parent last selected for the container.
struct FixedState {
    /// The `FixedContainer`'s layout object.
    layout: Box<dyn Layout>,
    /// The rendered children, in layout order.
    children: Vec<Mounted>,
    /// The proposal the parent layout selected when it placed this
    /// container — `selectedProposal` in the Swift port. `None` while no
    /// Rust parent has placed us: the container builds its own bounded offer
    /// from the rect it fills.
    selected: Cell<Option<ProposalSize>>,
    /// The host view, for layout invalidation inside watchers.
    host: Retained<HostView>,
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

/// `performLayout`: place every child inside the safe area, extend the ones
/// that manage their own safe area through the edges they touch, snap to
/// pixels.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn perform_layout(state: &Rc<RefCell<FixedState>>) {
    let (host, safe_rect, placements) = {
        let state = state.borrow();
        if state.children.is_empty() {
            return;
        }
        let host = state.host.clone();
        let host_view: &PlatformView = &state.host;
        let safe_rect = crate::native_layout::safe_area_rect(host_view);
        // Placed by a Rust parent: the proposal it selected. Natively hosted:
        // the boundary offer for the rect this container fills.
        let proposal = state.selected.get().unwrap_or_else(|| {
            ProposalSize::new(
                Some(safe_rect.size.width as f32),
                Some(safe_rect.size.height as f32),
            )
        });
        let placements = with_memoized_children(&children_of(&state), |children| {
            state.layout.place(
                waterui_core::layout::Rect::new(
                    waterui_core::layout::Point::new(
                        safe_rect.origin.x as f32,
                        safe_rect.origin.y as f32,
                    ),
                    waterui_core::layout::Size::new(
                        safe_rect.size.width as f32,
                        safe_rect.size.height as f32,
                    ),
                ),
                proposal,
                children,
            )
        });
        (host, safe_rect, placements)
    };

    let state = state.borrow();
    assert_eq!(
        placements.len(),
        state.children.len(),
        "fixed container layout returned {} placements for {} children",
        placements.len(),
        state.children.len()
    );
    let bounds: Rect = view::bounds(&host);
    let scale = host.display_scale().unwrap_or(1.0);
    for (index, (child, placement)) in state.children.iter().zip(placements.iter()).enumerate() {
        let mut frame = Rect::new(
            f64::from(placement.frame.x()),
            f64::from(placement.frame.y()),
            f64::from(placement.frame.width()),
            f64::from(placement.frame.height()),
        );
        assert!(
            frame.is_valid_for_layout(),
            "fixed container received an invalid layout rect for child {index}: {frame:?}"
        );
        if crate::native_layout::manages_safe_area(child.view()) {
            frame = frame.extended_through(safe_rect, bounds);
        }
        // The negotiated proposal lands before the frame: a container child
        // that lays out on the frame change already holds its selected
        // proposal, and a proposal change alone still marks it for relayout.
        proposal::deliver(child.view(), placement.proposal);
        view::set_frame(child.view(), frame.pixel_snapped(scale));
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

/// A `MeasureProposal` (f64, `None`-free axes) as a layout `ProposalSize`
/// (f32, `Option` axes).
fn to_proposal(proposal: MeasureProposal) -> ProposalSize {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; kit geometry is f64"
    )]
    ProposalSize::new(
        proposal.width.map(|width| width as f32),
        proposal.height.map(|height| height as f32),
    )
}

/// Installs the `fixed_container` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
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
            selected: Cell::new(None),
            host: host.clone(),
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
            move |_host| perform_layout(&state)
        });

        // The proposal a Rust parent selected invalidates placement even
        // when the frame does not move — equal bounds under a different
        // offer can produce a different child layout.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            let host = host.clone();
            move |selected| {
                let state = state.borrow();
                if state.selected.get() != Some(selected) {
                    state.selected.set(Some(selected));
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
        leaf.keep(sink_guard);
        leaf.keep(layout_guards);
        leaf.keep(state);
        leaf
    });
}
