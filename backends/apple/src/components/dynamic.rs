//! The `dynamic` leaf: `Native<Dynamic>` rendered through a [`HostView`].
//!
//! Mirrors `WuiDynamic`: a host container that owns at most one mounted
//! child. `Dynamic::connect` delivers each `set` as a `Context<AnyView>`;
//! the receiver renders the view through the captured [`Renderer`], hands
//! the previous [`LiveChild`] to `LiveChild::remove` — which retires a
//! claimed instance into a ghost beneath the live content — and mounts
//! the new leaf. The leaf's layout face delegates to the current child
//! and measures empty until one arrives — the layout contract the Swift
//! file's header states. A render miss keeps the previous child.
//!
//! A `.transition(...)` placed directly on the conditional is adopted:
//! [`render_adopting`] renders the `Dynamic` leaf unclaimed and stores
//! the claim in [`DynamicState::adopted`], so every child the host swaps
//! gets it as its `adopted` claim — a child's own declaration still wins
//! (`effective()`'s rule, §5.1).
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
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_core::metadata::Retain;

use crate::contract::{NativeLeaf, RenderContext, Renderer};
use crate::dispatch::Dispatcher;
use crate::proposal::{self, Placement};
use crate::transition::TransitionClaim;
use crate::transition::ghost::Ghost;
use crate::transition::live::{LiveChild, Removal};

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
    /// The mounted child the leaf's layout delegates to.
    child: Option<LiveChild>,
    /// The claim a `.transition(...)` directly over the conditional
    /// adopted — offered to every child the host swaps; a child's own
    /// declaration wins.
    adopted: Option<TransitionClaim>,
    /// Removal transitions in flight — the conditional owns the ghosts of
    /// the content it replaces and holds no slot for them (§4.5).
    ghosts: Vec<Ghost>,
    /// `false` until the first child is mounted: the initial child plays
    /// no insertion, every later one does.
    built: bool,
    /// The placement the parent layout gave this leaf — its proposal is
    /// `selectedProposal` in the Swift port — forwarded to the current
    /// child and re-applied to every replacement so a swapped-in child is
    /// never left with a stale or bounds-inferred offer, nor a stale pixel
    /// alignment.
    placement: Cell<Placement>,
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

/// Peels the conditional shapes a `.transition(...)` declaration adopts
/// through (§5.1): `Dynamic` itself, `Native<Dynamic>`, and the
/// `Metadata<Retain>` a `watch` puts around either — the same nodes
/// Hydrolysis's `adopt_by_dynamic` reaches. Anything else comes back
/// unchanged as `Err`. The `Retain` guard travels with the answer: it
/// owns the watcher keeping the conditional live, so it must outlive
/// the render.
pub fn expand_to_dynamic(view: AnyView) -> Result<(Dynamic, Option<Retain>), AnyView> {
    let (content, retained) = match view.downcast::<Metadata<Retain>>() {
        Ok(metadata) => (metadata.content, Some(metadata.value)),
        Err(view) => (view, None),
    };
    let dynamic = match content.downcast::<Dynamic>() {
        Ok(dynamic) => Ok(*dynamic),
        Err(content) => content
            .downcast::<waterui_backend_core::Native<Dynamic>>()
            .map(|native| native.into_inner()),
    };
    match (dynamic, retained) {
        (Ok(dynamic), retained) => Ok((dynamic, retained)),
        (Err(content), Some(retained)) => Err(AnyView::new(Metadata {
            content,
            value: retained,
        })),
        (Err(content), None) => Err(content),
    }
}

/// Renders `dynamic` with `claim` adopted (§5.1): the leaf itself stays
/// unclaimed — the conditional's own mount never transitions — and every
/// child it swaps in or out carries the claim as `adopted`. `retained`
/// is the `Metadata<Retain>` guard `expand_to_dynamic` peeled, kept
/// alive with the leaf.
pub fn render_adopting(
    dynamic: Dynamic,
    retained: Option<Retain>,
    claim: TransitionClaim,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mut leaf = render(dynamic, ctx, Some(claim));
    if let Some(retained) = retained {
        leaf.keep(retained);
    }
    leaf
}

/// `updateChild`: render `view`, retire the previous child, mount the
/// new leaf into the host, then re-forward the negotiated proposal and
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
        // The outgoing child leaves the semantic tree now: a claimed
        // instance retires into a ghost painted beneath the incoming
        // child — a conditional holds no slot (§4.5) — and an unclaimed
        // one detaches with the drop.
        if let Some(previous) = state.child.take() {
            let weak = Rc::downgrade(&state_rc);
            let ghost_view = view::retain_base(previous.view());
            if let Some(ghost) = previous.remove(Removal {
                env: state.renderer.context().env(),
                slot: None,
                on_complete: Box::new(move || {
                    let Some(state) = weak.upgrade() else {
                        return;
                    };
                    state.borrow_mut().ghosts.retain(|ghost| {
                        std::ptr::from_ref(ghost.host()) != Retained::as_ptr(&ghost_view)
                    });
                }),
            }) {
                view::add_subview_at_bottom(&state.host, ghost.host());
                state.ghosts.push(ghost);
            }
        }
        // The replacement plays its insertion only once the conditional
        // has built — the initial child is a mount, not a structural
        // insertion.
        let child = if state.built {
            LiveChild::mount_inserted(
                leaf,
                &state.host,
                state.adopted.as_ref(),
                state.renderer.context().env(),
            )
        } else {
            state.built = true;
            LiveChild::mount(leaf, &state.host, state.adopted.as_ref())
        };
        view::set_translates_autoresizing(child.view(), true);
        // The replacement inherits the last placement until the parent
        // re-places us.
        let placement = state.placement.get();
        if placement != Placement::UNPLACED {
            proposal::deliver(child.view(), placement);
        }
        state.child = Some(child);
        view::invalidate_layout(&state.host);
        crate::measure_memo::invalidate();
        state.host.clone()
    };
    // Force a synchronous layout pass so the new content updates
    // immediately — `layoutIfNeeded` / `layoutSubtreeIfNeeded`.
    host.layout_if_needed();
}

/// Builds the `dynamic` leaf with `adopted` as the claim every child
/// inherits — `None` for an ordinary conditional.
fn render(
    dynamic: Dynamic,
    ctx: &RenderContext<'_>,
    adopted: Option<TransitionClaim>,
) -> NativeLeaf {
    let host = HostView::new(ctx.mtm(), Rect::ZERO);
    let state = Rc::new(RefCell::new(DynamicState {
        renderer: ctx.renderer(),
        host: host.clone(),
        child: None,
        adopted,
        ghosts: Vec::new(),
        built: false,
        placement: Cell::new(Placement::UNPLACED),
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
        move |placement| {
            let state = state.borrow();
            state.placement.set(placement);
            if let Some(child) = &state.child {
                proposal::deliver(child.view(), placement);
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
}

/// Installs the `dynamic` handler on the dispatcher: `Native<Dynamic>`
/// becomes a host whose single child the handler's receiver swaps per
/// delivered `Context<AnyView>`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Dynamic>(|dynamic, ctx| render(dynamic, ctx, None));
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
