//! The `dynamic` leaf: `Native<Dynamic>` rendered through a [`HostView`].
//!
//! Mirrors `WuiDynamic`: a host container that owns at most one mounted
//! child. `Dynamic::connect` delivers each `set` as a `Context<AnyView>`;
//! the receiver renders the view through the captured [`Renderer`], drops
//! the previous [`Mounted`] (detaching its view), and mounts the new leaf.
//! The leaf's layout face delegates to the current child and measures
//! empty until one arrives — the layout contract the Swift file's header
//! states. A render miss keeps the previous child.
//!
//! The receiver holds only a `Weak` into the leaf state: the handler's
//! `Rc` owns the receiver for the handler's life, so a strong capture
//! would keep the leaf's platform view alive after the leaf drops —
//! `[weak self]` in the Swift port.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::geometry::{MeasureProposal, Rect};
use cocoa_ui::{Retained, view};
use waterui::component::Dynamic;
use waterui_backend_core::AnyView;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, Renderer};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state.
struct DynamicState {
    /// The render capability each delivered `AnyView` is realized through.
    renderer: Renderer,
    /// The host view — the leaf's platform object.
    host: Retained<HostView>,
    /// The main-thread marker the leaf was rendered under.
    mtm: cocoa_ui::MainThreadMarker,
    /// The mounted child the leaf's layout delegates to.
    child: Option<Mounted>,
    /// Removal transitions in flight — the conditional owns the ghosts of
    /// the content it replaces and holds no slot for them (§4.5).
    ghosts: Vec<crate::transition::RemovalGhost>,
    /// The proposal the parent layout selected when it placed this leaf —
    /// `selectedProposal` in the Swift port — forwarded to the current
    /// child and re-applied to every replacement so a swapped-in child is
    /// never left with a stale or bounds-inferred offer.
    selected: Cell<Option<ProposalSize>>,
}

impl core::fmt::Debug for DynamicState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicState").finish_non_exhaustive()
    }
}

/// `MeasureProposal` (f64 axes) as a layout `ProposalSize` (f32 axes).
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

/// The leaf's layout face: delegates every answer to the current child's
/// `SubView`, measuring empty until a child is mounted.
struct DynamicSubView {
    /// The leaf's state.
    state: Rc<RefCell<DynamicState>>,
}

impl core::fmt::Debug for DynamicSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicSubView").finish_non_exhaustive()
    }
}

impl SubView for DynamicSubView {
    /// `measure(_:)`: the child owns the full measurement packet — its
    /// alignment guides ride along inside stacks.
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.as_ref().map_or_else(
            || ViewDimensions::new(Size::new(0.0, 0.0)),
            |child| child.layout().measure(proposal),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state
            .borrow()
            .child
            .as_ref()
            .map_or(StretchAxis::None, |child| child.layout().stretch_axis())
    }

    fn priority(&self) -> i32 {
        self.state
            .borrow()
            .child
            .as_ref()
            .map_or(0, |child| child.layout().priority())
    }

    /// `rendersNothing`: a mounted child that itself renders nothing.
    fn is_empty(&self) -> bool {
        self.state
            .borrow()
            .child
            .as_ref()
            .is_some_and(|child| child.layout().is_empty())
    }
}

/// `updateChild`: render `view`, detach the previous child, mount the new
/// leaf into the host, then re-forward the negotiated proposal and
/// invalidate up the hierarchy. A render miss keeps the previous child.
///
/// The borrow is released before `layout_if_needed`: that call runs the
/// host's layout handler synchronously, and the handler borrows the same
/// state.
fn update_child(state: &Rc<RefCell<DynamicState>>, view: AnyView) {
    let host = {
        let state_rc = Rc::clone(state);
        let mut state = state.borrow_mut();
        let Some(leaf) = state.renderer.try_render(view) else {
            return;
        };
        // The outgoing child leaves the semantic tree now: `retire` drops
        // its leaf — ending watchers and subscriptions — while a declared
        // transition retains its view as a non-interactive ghost painted
        // beneath the incoming child (§4.5). No slot is held; the layout
        // sees the live child alone.
        if let Some(previous) = state.child.take() {
            let (outgoing, claim) = crate::transition::retire(previous);
            if let Some(claim) = claim {
                view::add_subview_at_bottom(&state.host, &outgoing);
                let weak = Rc::downgrade(&state_rc);
                let ghost_view = outgoing.clone();
                let ghost = crate::transition::start_removal(
                    state.mtm,
                    &outgoing,
                    &claim,
                    None,
                    move || {
                        let Some(state) = weak.upgrade() else { return };
                        let mut state = state.borrow_mut();
                        state.ghosts.retain(|ghost| {
                            Retained::as_ptr(&ghost.view) != Retained::as_ptr(&ghost_view)
                        });
                        view::remove_from_superview(&ghost_view);
                    },
                );
                state.ghosts.push(ghost);
            }
        }
        let mounted = leaf.mount(&state.host);
        if let Some(claim) = mounted.transition_claim() {
            crate::transition::play_insertion(mounted.view(), &claim);
        }
        view::set_translates_autoresizing(mounted.view(), true);
        // The replacement inherits the last negotiated proposal until the
        // parent re-places us.
        if let Some(selected) = state.selected.get() {
            proposal::deliver(mounted.view(), selected);
        }
        state.child = Some(mounted);
        view::invalidate_layout(&state.host);
        crate::measure_memo::invalidate();
        state.host.clone()
    };
    // Force a synchronous layout pass so the new content updates
    // immediately — `layoutIfNeeded` / `layoutSubtreeIfNeeded`.
    host.layout_if_needed();
}

/// Installs the `dynamic` handler on the dispatcher: `Native<Dynamic>`
/// becomes a host whose single child the handler's receiver swaps per
/// delivered `Context<AnyView>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Dynamic>(|dynamic, ctx| {
        let host = HostView::new(ctx.mtm(), Rect::ZERO);
        let state = Rc::new(RefCell::new(DynamicState {
            renderer: ctx.renderer(),
            host: host.clone(),
            mtm: ctx.mtm(),
            child: None,
            ghosts: Vec::new(),
            selected: Cell::new(None),
        }));

        // `layoutSubviews` / `layout`: the child always fills the host.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| {
                let state = state.borrow();
                if let Some(child) = &state.child {
                    view::set_frame(child.view(), view::bounds(view));
                }
            }
        });
        host.set_measure_handler({
            let state = Rc::clone(&state);
            move |_host, proposal| {
                let measured = DynamicSubView {
                    state: Rc::clone(&state),
                }
                .measure(to_proposal(proposal));
                cocoa_ui::Size::new(
                    f64::from(measured.size.width),
                    f64::from(measured.size.height),
                )
            }
        });

        // `WuiPrimaryContentProviding`: the primary-content chain descends
        // into whichever child is mounted now.
        crate::primary_content::forward_current(&host, {
            let state = Rc::clone(&state);
            move |_host| {
                state
                    .borrow()
                    .child
                    .as_ref()
                    .map(|child| view::retain_base(child.view()))
            }
        });

        // `setPlacementProposal`: store the negotiated offer and forward it
        // to the current child.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                state.selected.set(Some(selected));
                if let Some(child) = &state.child {
                    proposal::deliver(child.view(), selected);
                }
            }
        });

        // `setupWatcher`: the leaf connects as the handler's receiver. The
        // pre-connection view, if any, is delivered through the same queue.
        let weak = Rc::downgrade(&state);
        dynamic.connect(move |ctx| {
            if let Some(state) = weak.upgrade() {
                update_child(&state, ctx.into_value());
            }
        });

        let mut leaf = NativeLeaf::new(&*host, DynamicSubView { state });
        leaf.keep(sink_guard);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_proposal_maps_each_axis() {
        let proposal = to_proposal(MeasureProposal {
            width: Some(320.5),
            height: None,
        });
        assert_eq!(proposal.width, Some(320.5));
        assert_eq!(proposal.height, None);

        let proposal = to_proposal(MeasureProposal {
            width: None,
            height: Some(44.0),
        });
        assert_eq!(proposal.width, None);
        assert_eq!(proposal.height, Some(44.0));
    }

    /// A childless leaf reports the empty/defaults the layout contract
    /// states: zero measurement, no stretch, zero priority, not empty.
    #[test]
    fn unmounted_leaf_uses_empty_answers() {
        let child: Option<&dyn SubView> = None;
        let measure = child.map_or_else(
            || ViewDimensions::new(Size::new(0.0, 0.0)),
            |child| child.measure(ProposalSize::new(Some(100.0), None)),
        );
        assert_eq!(measure.size, Size::new(0.0, 0.0));
        assert_eq!(
            child.map_or(StretchAxis::None, SubView::stretch_axis),
            StretchAxis::None
        );
        assert_eq!(child.map_or(0, SubView::priority), 0);
        assert!(!child.is_some_and(SubView::is_empty));
    }
}
