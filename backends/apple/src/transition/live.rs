//! What every structural site stores for a live child (§1.4): the
//! mounted leaf — the transition wrapper when the instance carried a
//! claim — and the claim's live state.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, view};
use waterui_backend_core::Environment;
use waterui_core::layout::{Size, SubView};
use waterui_core::transition::{GhostSlot, TransitionGeometry, TransitionPhase};

use crate::contract::{Mounted, NativeLeaf};
use crate::transition::TransitionClaim;
use crate::transition::ghost::Ghost;
use crate::transition::host::TransitionHost;
use crate::transition::property::InsertionRun;
use crate::transition::retire::{self, RetiredVisual};

/// A live child: its mounted leaf and, when the instance carried a
/// claim, the transition state the site removes through.
pub struct LiveChild {
    /// The wrapper leaf when claimed, the content leaf otherwise.
    mounted: Mounted,
    /// The claim's live state — `None` for an unclaimed instance.
    transition: Option<LiveTransition>,
}

/// A claimed instance's live state.
pub struct LiveTransition {
    /// The wrapper the content mounted inside.
    host: Rc<TransitionHost>,
    /// The owned claim — the leaf's own, or the adopted one when the leaf
    /// declared none (the child's own claim wins, `effective()`'s rule).
    claim: TransitionClaim,
    /// The in-flight insertion run — dropped at retirement so a removal
    /// starts a fresh clock at progress 0.
    insertion: Rc<RefCell<Option<InsertionRun>>>,
}

impl core::fmt::Debug for LiveChild {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveChild").finish_non_exhaustive()
    }
}

/// The inputs a structural removal needs.
pub struct Removal<'a> {
    /// The site's environment — the blur and pixel realizations read
    /// their GPU services from it (stages 8 and 9).
    pub env: &'a Environment,
    /// The frozen layout slot — collection sites only (§4).
    pub slot: Option<GhostSlot>,
    /// The site's slot release; captures only a `Weak` to site state.
    pub on_complete: Box<dyn FnOnce()>,
}

impl LiveChild {
    /// Initial build: wraps a claimed leaf and plays nothing.
    pub fn mount(
        leaf: NativeLeaf,
        parent: &PlatformView,
        adopted: Option<&TransitionClaim>,
    ) -> Self {
        Self::wrap(leaf, parent, adopted)
    }

    /// Structural insertion: wraps and starts the insertion run (§3.2).
    pub fn mount_inserted(
        leaf: NativeLeaf,
        parent: &PlatformView,
        adopted: Option<&TransitionClaim>,
        env: &Environment,
    ) -> Self {
        let mut child = Self::wrap(leaf, parent, adopted);
        if let Some(transition) = child.transition.as_mut() {
            transition.insertion = InsertionRun::start(&transition.host, &transition.claim, env);
        }
        child
    }

    /// The child's placed view — the transition host when claimed.
    pub fn view(&self) -> &PlatformView {
        self.mounted.view()
    }

    /// The child's layout face.
    pub fn layout(&self) -> &dyn SubView {
        self.mounted.layout()
    }

    /// The claim the instance carries — `None` for an unclaimed one.
    pub const fn claim(&self) -> Option<&TransitionClaim> {
        match &self.transition {
            Some(transition) => Some(&transition.claim),
            None => None,
        }
    }

    /// Wraps a claimed, non-empty leaf in a [`TransitionHost`] and mounts
    /// the wrapper; anything else mounts plain. The leaf's own claim
    /// wins over `adopted`; an empty instance is never wrapped, matching
    /// Hydrolysis's `!is_empty()` filter.
    fn wrap(
        mut leaf: NativeLeaf,
        parent: &PlatformView,
        adopted: Option<&TransitionClaim>,
    ) -> Self {
        let claim = leaf
            .take_transition()
            .or_else(|| adopted.cloned())
            .filter(|_| !leaf.layout().is_empty());
        let Some(claim) = claim else {
            return Self {
                mounted: leaf.mount(parent),
                transition: None,
            };
        };
        let mtm =
            cocoa_ui::MainThreadMarker::new().expect("a transition host mounts on the main thread");
        let host = Rc::new(TransitionHost::new(mtm, leaf));
        let mounted = host.leaf().mount(parent);
        view::set_translates_autoresizing(mounted.view(), true);
        Self::claimed(mounted, host, claim)
    }

    /// A claimed `Self` whose insertion slot is still empty.
    fn claimed(mounted: Mounted, host: Rc<TransitionHost>, claim: TransitionClaim) -> Self {
        Self {
            mounted,
            transition: Some(LiveTransition {
                host,
                claim,
                insertion: Rc::new(RefCell::new(None)),
            }),
        }
    }

    /// Structural removal (§1.5). An unclaimed instance — or a claimed
    /// one whose transition is already complete — detaches now and
    /// answers `None`; otherwise the host retires and the ghost starts.
    pub fn remove(self, removal: Removal<'_>) -> Option<Ghost> {
        let Removal {
            env,
            slot,
            on_complete,
        } = removal;
        let Self {
            mounted,
            transition,
        } = self;
        let LiveTransition {
            host,
            claim,
            insertion,
        } = transition?;
        // 1. A live insertion's run ends: the removal starts a fresh
        //    clock at progress 0.
        insertion.borrow_mut().take();
        // 2. The geometry freezes at the host's current bounds.
        let size = view::bounds(&host.host).size;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the layout contract is f32; kit geometry is f64"
        )]
        let geometry = TransitionGeometry::new(
            Size::new(size.width as f32, size.height as f32),
            claim.direction,
        );
        // 3. Frozen frames (§2.2): each armed surface hands its last
        //    presented frame and its presentation layer while it is
        //    still live — before any semantic teardown.
        #[cfg(feature = "gpu_surface")]
        let frozen = retire::collect_frozen_surfaces(&host.content_view);
        // 4. Resign focus inside the outgoing subtree — the only
        //    per-view walk.
        retire::resign_focus_within(&host.content_view);
        // 5. From here the subtree is a ghost.
        host.host.retire();
        // 6. Semantic end: the leaf cascade drops every watcher and
        //    subscription while the retired host keeps the views.
        let wrapper = mounted.into_retired_leaf();
        let content = host
            .content
            .borrow_mut()
            .take()
            .expect("a live transition host holds its content")
            .into_retired_leaf();
        drop(content);
        drop(wrapper);
        // 7. The live guards cleared the capturable slots in step 6 —
        //    each armed surface's frozen stand-in takes the slot, so an
        //    enclosing live filter keeps resolving the ghost's surfaces.
        #[cfg(feature = "gpu_surface")]
        let frozen = retire::install_frozen_surfaces(env, frozen);
        // 8. Build the visual and start the run.
        #[cfg(not(feature = "gpu_surface"))]
        let frozen = ();
        let visual = RetiredVisual::new(
            host.host.clone(),
            host.content_view.clone(),
            frozen,
            geometry,
        );
        let state = claim
            .spec
            .start(TransitionPhase::Removal, claim.reduce_motion);
        if state.captures_content() {
            crate::transition::transition_violation!(
                "waterui-apple: pixel transitions land in stage 9 — a `captures_content` \
                 removal has no realization yet; use a property transition (opacity, scale, \
                 move, rotation) or an asymmetric transition"
            );
        }
        // A zero-duration removal has nothing to sample, and a host with no
        // window never ticks — the insertion run's rule: both complete now,
        // and dropping `visual` performs the detachment.
        if state.is_complete() || !view::has_window(&host.host) {
            return None;
        }
        let mtm = cocoa_ui::MainThreadMarker::new().expect("a ghost starts on the main thread");
        Some(Ghost::start(mtm, visual, state, env, slot, on_complete))
    }
}
