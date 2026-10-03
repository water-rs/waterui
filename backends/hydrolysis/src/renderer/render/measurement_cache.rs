//! Unified view-measurement caching.
//!
//! Hydrolysis keeps measurement state in two places with different lifetimes:
//!
//! - **Per-node measurement memos** ([`NodeMeasureEntry`], [`MemoGate`]): one
//!   pair lives inside every container-like retained [`RenderNode`]'s payload
//!   struct, carrying that node's proposal ring for the current frame.
//!   Because the entry is owned by the node it memoizes, it can never answer
//!   for a different node — no address, epoch, or aliasing check is needed,
//!   only the environment identity and frame stamp.
//! - **Per-rebuild view dimensions** ([`MeasurementCaches::view_dimensions`]):
//!   keyed by view pointer identity, environment identity and layout proposal.
//!   Only valid while the view tree that produced the pointers is being
//!   dispatched, so it is cleared at the start of every structural rebuild.
//! - **Per-`Dynamic`-node dimensions** (intrinsic and proposal-dependent):
//!   keyed by the node's stable identity. A connected `Dynamic` owns its
//!   content inside the renderer's retained tree; the `dynamic_nodes`
//!   registry reaches that retained child by identity so a dispatch measure
//!   can re-measure it for the real proposal, and these entries cache the
//!   answers that produces. They persist across rebuilds, are refreshed
//!   whenever the node's content is (re-)dispatched or re-measured, and are
//!   pruned to the identities that are still alive when a rebuild finishes.

use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use waterui_core::layout::{ProposalSize, ViewDimensions};

use crate::renderer::tree::RenderNode;

/// A layout proposal as a hashable cache-key component.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ProposalKey {
    width_bits: Option<u32>,
    height_bits: Option<u32>,
}

impl From<ProposalSize> for ProposalKey {
    fn from(proposal: ProposalSize) -> Self {
        Self {
            width_bits: proposal.width.map(f32::to_bits),
            height_bits: proposal.height.map(f32::to_bits),
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ViewMeasurementKey {
    view_identity: usize,
    env_identity: usize,
    proposal: ProposalKey,
}

/// How many distinct proposals a node's memo answers before its oldest slot
/// is overwritten. A stack's measure and place passes probe a child under a
/// handful of proposals (intrinsic, minimum, maximum, negotiated extent);
/// eight covers that with margin while keeping the per-probe scan trivial.
const NODE_PROPOSAL_SLOTS: usize = 8;

/// One retained node's memoized measurements for a frame: a small proposal
/// ring its `measure` answers.
///
/// The entry lives inside the node struct itself, so it is destroyed with the
/// node and can never answer through a reused heap address. A probe of a
/// re-measured node costs one `RefCell` borrow on the already-in-hand node
/// rather than a keyed lookup into a frame-global table — a stack's repeated
/// probes of a child under different proposals hit the ring without hashing.
///
/// Nodes only reach this entry through their [`MemoGate`]: a node probed
/// once per frame (the common case — there is nothing to reuse) never
/// touches the `RefCell` at all.
#[derive(Default)]
pub(crate) struct NodeMeasureEntry {
    /// Frame the entry was filled in: [`MeasurementCaches::frame`] at write
    /// time. `0` (the default) precedes every frame — the counter starts at
    /// `1`, so a fresh entry never answers.
    frame: u64,
    /// `(env identity, proposal, dimensions)` triples answered this frame,
    /// capped at [`NODE_PROPOSAL_SLOTS`]. A node's probes can alternate
    /// between envs (a stack measuring under a scoped env, `place`
    /// re-measuring under another), so env is a slot key rather than an
    /// entry key — one env switch must not discard the other's answers.
    slots: Vec<(usize, ProposalKey, ViewDimensions)>,
    /// Next slot the ring overwrites once `slots` is full.
    next: usize,
}

/// `Cell`-sized probe gate in front of a node's [`NodeMeasureEntry`].
///
/// The memo can only answer a probe that repeats a proposal already
/// answered this frame, so memoization only pays on nodes where that
/// repeat actually happens — the pattern `Layout` produces on
/// container-like nodes (`measure_stack` probes every child several times,
/// and `place` re-runs it). The gate detects the pattern with a 16-bit
/// fingerprint filter over the proposals seen this frame: a node whose
/// probes are always distinct proposals (the common case — there is
/// nothing to reuse) keeps `memoize` off, so the probe costs a `Cell`
/// read+write and the entry is never borrowed. `memoize` flips on the
/// first repeated proposal fingerprint and stays on across frames while
/// the node is measured under the same environment, so a re-measured node
/// memoizes from the first probe of every following frame.
///
/// The filter may raise `memoize` on a fingerprint collision between
/// distinct proposals — harmless, it only stores answers the ring then
/// serves exactly.
#[derive(Clone, Copy, Default)]
pub(crate) struct MemoGate {
    /// Frame `seen_mask` was recorded in.
    frame: u64,
    /// Bitmask of proposal fingerprints probed this frame: bit `f(p)` is
    /// set once a probe under `p` has run. 16 lanes keep collisions rare.
    seen_mask: u16,
    /// The node has answered a repeat proposal.
    memoize: bool,
}

impl MemoGate {
    /// Registers one `measure` probe of the node under `proposal`,
    /// returning whether the node's [`NodeMeasureEntry`] may hold an
    /// answer — i.e. the node memoizes at all.
    ///
    /// Env is deliberately not part of the gate: the entry keys each slot
    /// by env, so a node probed under alternating envs still flags once a
    /// proposal repeats. A `false` answer already consumed the cheap gate
    /// update: the caller runs the real measurement and does not touch the
    /// entry.
    pub(crate) fn probe(&mut self, frame: u64, proposal: ProposalSize) -> bool {
        let fingerprint = Self::fingerprint(proposal);
        if self.frame != frame {
            self.frame = frame;
            self.seen_mask = fingerprint;
            return self.memoize;
        }
        if self.seen_mask & fingerprint != 0 {
            self.memoize = true;
        }
        self.seen_mask |= fingerprint;
        self.memoize
    }

    /// A 4-bit fingerprint of the proposal's bit pattern, used as the lane
    /// index into [`Self::seen_mask`]. The multiplicative fold spreads the
    /// low-entropy bit patterns of typical proposals (small integers)
    /// across all 16 lanes.
    fn fingerprint(proposal: ProposalSize) -> u16 {
        let w = proposal.width.map(f32::to_bits).unwrap_or(0xA5A5_A5A5);
        let h = proposal.height.map(f32::to_bits).unwrap_or(0x5A5A_5A5A);
        let mixed = w.wrapping_mul(0x9E37_79B1) ^ h.rotate_left(13);
        1 << ((mixed ^ (mixed >> 8) ^ (mixed >> 16) ^ (mixed >> 24)) & 15)
    }
}

impl NodeMeasureEntry {
    /// True when the entry answers for this frame.
    fn current(&self, frame: u64) -> bool {
        self.frame == frame
    }

    /// Memoized dimensions under `(env, proposal)`, if answered this frame.
    pub(crate) fn dims(
        &self,
        env_identity: usize,
        frame: u64,
        proposal: ProposalSize,
    ) -> Option<ViewDimensions> {
        if !self.current(frame) {
            return None;
        }
        let proposal_key = ProposalKey::from(proposal);
        self.slots
            .iter()
            .find(|(env, key, _)| *env == env_identity && *key == proposal_key)
            .map(|(_, _, dimensions)| dimensions.clone())
    }

    /// Marks the entry as answering for `frame`, resetting it if it was
    /// written under a different frame.
    pub(crate) fn ensure_current(&mut self, frame: u64) {
        if !self.current(frame) {
            self.frame = frame;
            self.slots.clear();
            self.next = 0;
        }
    }

    /// Records an answer under `(env_identity, proposal)` in the ring. The
    /// entry must already be current ([`Self::ensure_current`]).
    pub(crate) fn push_dims(
        &mut self,
        env_identity: usize,
        proposal: ProposalSize,
        dimensions: ViewDimensions,
    ) {
        let proposal_key = ProposalKey::from(proposal);
        if let Some((_, _, cached)) = self
            .slots
            .iter_mut()
            .find(|(env, key, _)| *env == env_identity && *key == proposal_key)
        {
            *cached = dimensions;
        } else if self.slots.len() < NODE_PROPOSAL_SLOTS {
            self.slots.push((env_identity, proposal_key, dimensions));
        } else {
            self.slots[self.next] = (env_identity, proposal_key, dimensions);
            self.next = (self.next + 1) % NODE_PROPOSAL_SLOTS;
        }
    }
}

#[derive(Default)]
pub(crate) struct MeasurementCaches {
    view_dimensions: FxHashMap<ViewMeasurementKey, ViewDimensions>,
    dynamic_intrinsic: FxHashMap<usize, ViewDimensions>,
    dynamic_proposal: FxHashMap<(usize, ProposalKey), ViewDimensions>,
    /// Weak handles to the retained child of each connected `Dynamic`, keyed
    /// by the node's identity. A connected `Dynamic`'s content lives in its
    /// `DynamicHostNode`, so a dispatch measure of the `Dynamic` measures
    /// this child for the real proposal instead of guessing from a cache
    /// keyed by a different probe.
    dynamic_nodes: FxHashMap<usize, Weak<RefCell<RenderNode>>>,
    /// Re-entrancy guard: `Dynamic` nodes currently being measured.
    dynamic_measurement_stack: Vec<(usize, ProposalSize)>,
    /// Depth of the transient-measurement scopes currently open; see
    /// [`MeasurementCaches::begin_transient_measurement`].
    transient_depth: u32,
    /// Frame counter stamping node memo entries; bumped by
    /// [`Self::begin_frame`] so no memo ever needs a clear pass.
    frame: u64,
    hits: u32,
    misses: u32,
}

impl MeasurementCaches {
    /// The frame stamp node memos validate against.
    pub(crate) fn frame(&self) -> u64 {
        self.frame
    }

    /// Cached dimensions for a concrete view under a proposal, counting the
    /// lookup in the per-frame hit/miss statistics.
    ///
    /// Answers `None` inside a transient scope, where an address does not
    /// identify a view.
    pub(crate) fn view_dimensions(
        &mut self,
        view_identity: usize,
        env_identity: usize,
        proposal: ProposalSize,
    ) -> Option<ViewDimensions> {
        if self.transient_depth > 0 {
            self.misses += 1;
            return None;
        }
        let key = ViewMeasurementKey {
            view_identity,
            env_identity,
            proposal: proposal.into(),
        };
        let cached = self.view_dimensions.get(&key).cloned();
        if cached.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        cached
    }

    /// Records a view's dimensions, unless a transient scope is open — a view
    /// that dies with the measurement must leave nothing behind under an
    /// address the next one will be handed.
    pub(crate) fn store_view_dimensions(
        &mut self,
        view_identity: usize,
        env_identity: usize,
        proposal: ProposalSize,
        dimensions: ViewDimensions,
    ) {
        if self.transient_depth > 0 {
            return;
        }
        let key = ViewMeasurementKey {
            view_identity,
            env_identity,
            proposal: proposal.into(),
        };
        self.view_dimensions.insert(key, dimensions);
    }

    /// Opens a scope in which measured views are materialized by the
    /// measurement itself rather than owned by the retained tree.
    ///
    /// `view_dimensions` is keyed by a view's heap address, which names one
    /// view only while that view is allocated. A view built to be measured — a
    /// list row pulled out of its collection, a control's label re-erased into
    /// an `AnyView`, a tab's content built for a size — is dropped the moment
    /// the measurement returns, and the allocator hands its address straight to
    /// the next such view *within the same frame*. Clearing the cache per frame
    /// therefore does not make the key sound: the stale entry is read back as
    /// the new view's size, and a list reports its neighbour's row height.
    ///
    /// Inside the scope the cache is neither read nor written, so an address in
    /// it always belongs to the view that is still holding it. The scope covers
    /// the whole subtree because a view materialized here owns its children:
    /// their addresses die with it.
    pub(crate) fn begin_transient_measurement(&mut self) {
        self.transient_depth += 1;
    }

    /// Closes a scope opened by [`Self::begin_transient_measurement`].
    pub(crate) fn end_transient_measurement(&mut self) {
        self.transient_depth = self
            .transient_depth
            .checked_sub(1)
            .expect("hydrolysis transient measurement scope underflow");
    }

    pub(crate) fn dynamic_intrinsic(&self, identity: usize) -> Option<ViewDimensions> {
        self.dynamic_intrinsic.get(&identity).cloned()
    }

    /// Cached dimensions for a `Dynamic` node under a proposal. An
    /// unspecified proposal resolves to the intrinsic entry.
    pub(crate) fn dynamic_dimensions(
        &self,
        identity: usize,
        proposal: ProposalSize,
    ) -> Option<ViewDimensions> {
        if proposal == ProposalSize::UNSPECIFIED {
            self.dynamic_intrinsic(identity)
        } else {
            self.dynamic_proposal
                .get(&(identity, proposal.into()))
                .cloned()
        }
    }

    /// Store a `Dynamic` node's measured dimensions, routed to the intrinsic
    /// or proposal-dependent entry by the proposal.
    pub(crate) fn store_dynamic_dimensions(
        &mut self,
        identity: usize,
        proposal: ProposalSize,
        dimensions: ViewDimensions,
    ) {
        if proposal == ProposalSize::UNSPECIFIED {
            self.dynamic_intrinsic.insert(identity, dimensions);
        } else {
            self.dynamic_proposal
                .insert((identity, proposal.into()), dimensions);
        }
    }

    /// Register the retained child a `Dynamic` was just connected to. The
    /// weak entry dies with the host node; a rebuild registers its new
    /// child over the stale one.
    pub(crate) fn register_dynamic_node(
        &mut self,
        identity: usize,
        child: &Rc<RefCell<RenderNode>>,
    ) {
        self.dynamic_nodes.insert(identity, Rc::downgrade(child));
    }

    /// The retained child a connected `Dynamic` is measured through, or
    /// `None` when the host node was dropped before the caches were pruned.
    pub(crate) fn dynamic_node(&self, identity: usize) -> Option<Rc<RefCell<RenderNode>>> {
        self.dynamic_nodes.get(&identity).and_then(Weak::upgrade)
    }

    /// Marks a `Dynamic` node as being measured, crashing on re-entrant
    /// measurement of the same node (which would recurse forever).
    pub(crate) fn begin_dynamic_measurement(&mut self, identity: usize, proposal: ProposalSize) {
        if let Some((_, active_proposal)) = self
            .dynamic_measurement_stack
            .iter()
            .find(|(active_identity, _)| *active_identity == identity)
        {
            panic!(
                "hydrolysis re-entered Dynamic measurement for node {identity} with proposal {proposal:?} while already measuring {active_proposal:?}"
            );
        }
        self.dynamic_measurement_stack.push((identity, proposal));
    }

    pub(crate) fn finish_dynamic_measurement(&mut self, identity: usize, phase: &str) {
        let popped = self
            .dynamic_measurement_stack
            .pop()
            .expect("hydrolysis dynamic measurement stack underflow");
        assert!(
            popped.0 == identity,
            "hydrolysis dynamic measurement stack corrupted {phase}"
        );
    }

    /// Invalidate the per-frame view-dimension entries; `Dynamic` node entries
    /// survive (their content is owned by the renderer and may not be
    /// re-measurable until the node is re-dispatched).
    ///
    /// Must run at the start of every frame that measures — structural rebuilds
    /// **and** reactive patch frames alike. The view-dimension cache is keyed by
    /// each view's `stable_ptr` (its heap address), which is unique only while
    /// that view is alive: across frames a freed view's address is reused by a
    /// new, different view, so a stale entry under the reused address would
    /// otherwise be returned as that new view's measurement.
    ///
    /// Clearing per frame covers views that live as long as the tree does.
    /// Views a measurement materializes for itself are freed *within* the
    /// frame, and [`Self::begin_transient_measurement`] keeps those out of the
    /// cache entirely.
    pub(crate) fn begin_frame(&mut self) {
        self.view_dimensions.clear();
        // Node memo entries are stamped with this counter and reset lazily
        // on next access instead of a clear pass.
        self.frame = self
            .frame
            .checked_add(1)
            .expect("hydrolysis measurement frame counter overflow");
        self.reset_counters();
    }

    /// Prune `Dynamic` entries whose node no longer exists in the view tree.
    ///
    /// `is_alive` is computed by walking the window's retained tree, which
    /// cannot see into widget-owned subview caches — a `List` keeps each row's
    /// subtree in its own `item_cache`, so dynamics living inside a live row
    /// always look dead to that walk. Dropping the registry entry anyway would
    /// orphan a connected `Dynamic` from the measure path, so `dynamic_nodes`
    /// answers liveness with its own `Weak`: it stays registered exactly as
    /// long as the `DynamicHostNode` that owns it does. The measured-dimension
    /// caches still use `is_alive`, since their entries are keyed by the
    /// `Dynamic`'s `Rc` pointer and must not outlive it.
    pub(crate) fn retain_dynamic_identities(&mut self, is_alive: impl Fn(usize) -> bool) {
        self.dynamic_intrinsic
            .retain(|identity, _| is_alive(*identity));
        self.dynamic_proposal
            .retain(|(identity, _), _| is_alive(*identity));
        self.dynamic_nodes
            .retain(|_, weak| weak.upgrade().is_some());
    }

    pub(crate) fn reset_counters(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// Per-frame (hits, misses) of the view-dimension cache.
    pub(crate) fn stats(&self) -> (u32, u32) {
        (self.hits, self.misses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::layout::Size;

    fn dimensions(width: f32, height: f32) -> ViewDimensions {
        ViewDimensions::new(Size::new(width, height))
    }

    #[test]
    fn view_dimensions_hit_and_miss_are_counted() {
        let mut caches = MeasurementCaches::default();
        let proposal = ProposalSize::new(Some(100.0), None);
        assert!(caches.view_dimensions(1, 2, proposal).is_none());
        caches.store_view_dimensions(1, 2, proposal, dimensions(40.0, 20.0));
        assert_eq!(
            caches.view_dimensions(1, 2, proposal).map(|d| d.size),
            Some(Size::new(40.0, 20.0))
        );
        assert_eq!(caches.stats(), (1, 1));

        // A different environment or proposal is a distinct entry.
        assert!(caches.view_dimensions(1, 3, proposal).is_none());
        assert!(
            caches
                .view_dimensions(1, 2, ProposalSize::UNSPECIFIED)
                .is_none()
        );
        assert_eq!(caches.stats(), (1, 3));
    }

    #[test]
    fn node_entry_answers_only_in_its_frame_and_env() {
        let mut entry = NodeMeasureEntry::default();
        let proposal = ProposalSize::new(Some(100.0), None);
        assert!(entry.dims(2, 1, proposal).is_none());

        entry.ensure_current(1);
        entry.push_dims(2, proposal, dimensions(40.0, 20.0));

        assert_eq!(
            entry.dims(2, 1, proposal).map(|d| d.size),
            Some(Size::new(40.0, 20.0))
        );

        // A different env or frame never answers; alternating envs keep
        // their own slots.
        assert!(entry.dims(3, 1, proposal).is_none());
        assert!(entry.dims(2, 2, proposal).is_none());
        entry.push_dims(3, proposal, dimensions(1.0, 1.0));
        assert_eq!(
            entry.dims(3, 1, proposal).map(|d| d.size),
            Some(Size::new(1.0, 1.0))
        );
        assert_eq!(
            entry.dims(2, 1, proposal).map(|d| d.size),
            Some(Size::new(40.0, 20.0))
        );

        // Re-claiming under the new frame resets and answers again.
        entry.ensure_current(2);
        assert!(entry.dims(2, 2, proposal).is_none());
        entry.push_dims(2, proposal, dimensions(9.0, 9.0));
        assert_eq!(
            entry.dims(2, 2, proposal).map(|d| d.size),
            Some(Size::new(9.0, 9.0))
        );
    }

    #[test]
    fn memo_gate_memoizes_only_repeat_proposals() {
        let mut gate = MemoGate::default();
        let narrow = ProposalSize::new(Some(100.0), None);
        let wide = ProposalSize::new(Some(200.0), Some(50.0));
        // First probe of a frame is never a hit.
        assert!(!gate.probe(1, narrow));
        // A probe under a *different* proposal does not turn it on.
        assert!(!gate.probe(1, wide));
        // A repeated proposal does.
        assert!(gate.probe(1, narrow));
        // The flag sticks across frames.
        assert!(gate.probe(2, narrow));
        assert!(gate.probe(2, wide));
    }

    #[test]
    fn node_entry_remembers_each_proposal() {
        let mut entry = NodeMeasureEntry::default();
        let narrow = ProposalSize::new(Some(100.0), None);
        let wide = ProposalSize::new(Some(200.0), Some(50.0));
        entry.ensure_current(1);
        entry.push_dims(2, narrow, dimensions(40.0, 20.0));
        entry.push_dims(2, wide, dimensions(80.0, 40.0));

        assert_eq!(
            entry.dims(2, 1, narrow).map(|d| d.size),
            Some(Size::new(40.0, 20.0))
        );
        assert_eq!(
            entry.dims(2, 1, wide).map(|d| d.size),
            Some(Size::new(80.0, 40.0))
        );
    }

    #[test]
    fn begin_frame_clears_view_dimensions_but_keeps_dynamic_entries() {
        let mut caches = MeasurementCaches::default();
        caches.store_view_dimensions(1, 2, ProposalSize::UNSPECIFIED, dimensions(1.0, 1.0));
        caches.store_dynamic_dimensions(7, ProposalSize::UNSPECIFIED, dimensions(2.0, 2.0));
        caches.store_dynamic_dimensions(
            7,
            ProposalSize::new(Some(50.0), None),
            dimensions(3.0, 3.0),
        );

        caches.begin_frame();

        assert!(
            caches
                .view_dimensions(1, 2, ProposalSize::UNSPECIFIED)
                .is_none()
        );
        assert_eq!(
            caches.dynamic_intrinsic(7).map(|d| d.size),
            Some(Size::new(2.0, 2.0))
        );
        assert_eq!(
            caches
                .dynamic_dimensions(7, ProposalSize::new(Some(50.0), None))
                .map(|d| d.size),
            Some(Size::new(3.0, 3.0))
        );
    }

    #[test]
    fn unspecified_proposal_routes_to_intrinsic_entry() {
        let mut caches = MeasurementCaches::default();
        caches.store_dynamic_dimensions(5, ProposalSize::UNSPECIFIED, dimensions(8.0, 9.0));
        assert_eq!(
            caches
                .dynamic_dimensions(5, ProposalSize::UNSPECIFIED)
                .map(|d| d.size),
            Some(Size::new(8.0, 9.0))
        );
        assert!(
            caches
                .dynamic_dimensions(5, ProposalSize::new(Some(10.0), Some(10.0)))
                .is_none()
        );
    }

    #[test]
    fn pruning_drops_dead_dynamic_identities() {
        let mut caches = MeasurementCaches::default();
        caches.store_dynamic_dimensions(1, ProposalSize::UNSPECIFIED, dimensions(1.0, 1.0));
        caches.store_dynamic_dimensions(
            2,
            ProposalSize::new(Some(4.0), None),
            dimensions(2.0, 2.0),
        );
        caches.retain_dynamic_identities(|identity| identity == 1);
        assert!(caches.dynamic_intrinsic(1).is_some());
        assert!(
            caches
                .dynamic_dimensions(2, ProposalSize::new(Some(4.0), None))
                .is_none()
        );
    }

    #[test]
    #[should_panic(expected = "re-entered Dynamic measurement")]
    fn reentrant_dynamic_measurement_crashes() {
        let mut caches = MeasurementCaches::default();
        caches.begin_dynamic_measurement(1, ProposalSize::UNSPECIFIED);
        caches.begin_dynamic_measurement(1, ProposalSize::new(Some(1.0), None));
    }

    #[test]
    fn nested_distinct_dynamic_measurements_balance() {
        let mut caches = MeasurementCaches::default();
        caches.begin_dynamic_measurement(1, ProposalSize::UNSPECIFIED);
        caches.begin_dynamic_measurement(2, ProposalSize::UNSPECIFIED);
        caches.finish_dynamic_measurement(2, "inner");
        caches.finish_dynamic_measurement(1, "outer");
    }
}
