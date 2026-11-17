//! Measurement memoization.
//!
//! Every container-like retained [`RenderNode`] carries its own measurement
//! memo ([`NodeMeasureEntry`], [`MemoGate`]) holding that node's proposal
//! ring for the current frame. Because the entry is owned by the node it
//! memoizes, it can never answer for a different node — no address, epoch, or
//! aliasing check is needed, only the environment identity and frame stamp.
//! [`MeasurementCaches`] holds the frame stamp the memos validate against and
//! counts their hits and misses.
//!
//! [`RenderNode`]: crate::renderer::tree::RenderNode

use waterui_core::layout::{ProposalSize, ViewDimensions};

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
pub struct NodeMeasureEntry {
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
pub struct MemoGate {
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
        let w = proposal.width.map_or(0xA5A5_A5A5, f32::to_bits);
        let h = proposal.height.map_or(0x5A5A_5A5A, f32::to_bits);
        let mixed = w.wrapping_mul(0x9E37_79B1) ^ h.rotate_left(13);
        1 << ((mixed ^ (mixed >> 8) ^ (mixed >> 16) ^ (mixed >> 24)) & 15)
    }
}

impl NodeMeasureEntry {
    /// True when the entry answers for this frame.
    const fn current(&self, frame: u64) -> bool {
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

/// The frame stamp node memos validate against, and the per-frame count of
/// their hits and misses.
#[derive(Default)]
pub struct MeasurementCaches {
    /// Frame counter stamping node memo entries; bumped by
    /// [`Self::begin_frame`] so no memo ever needs a clear pass.
    frame: u64,
    hits: u32,
    misses: u32,
}

impl MeasurementCaches {
    /// The frame stamp node memos validate against.
    pub(crate) const fn frame(&self) -> u64 {
        self.frame
    }

    /// Counts a measure a node memo answered.
    pub(crate) const fn record_hit(&mut self) {
        self.hits += 1;
    }

    /// Counts a measure a node memo did not answer.
    pub(crate) const fn record_miss(&mut self) {
        self.misses += 1;
    }

    /// Starts a frame: every node memo filled before it goes stale.
    ///
    /// Must run at the start of every frame that measures — structural rebuilds
    /// **and** reactive patch frames alike.
    pub(crate) const fn begin_frame(&mut self) {
        // Node memo entries are stamped with this counter and reset lazily
        // on next access instead of a clear pass.
        self.frame = self
            .frame
            .checked_add(1)
            .expect("hydrolysis measurement frame counter overflow");
        self.reset_counters();
    }

    /// Opens a `BuiltSubview` layout-dependency pass, invalidating the memos
    /// filled outside it.
    ///
    /// A host can measure a sub-view earlier in the same flush — a button or
    /// menu's `measure_built` on its label, a lazy row's `patch_and_measure`
    /// — filling node memos before `place`
    /// lays it out. Inside the dependency pass those answers would come back
    /// from the cache without running the reads beneath them, so the pass's
    /// dependency sweep would prune subscriptions the cached layout still
    /// needs. Bumping the memo frame stamps earlier answers stale; memos
    /// filled *during* the pass still serve its repeated probes.
    pub(crate) const fn begin_dependency_pass(&mut self) {
        self.frame = self
            .frame
            .checked_add(1)
            .expect("hydrolysis measurement frame counter overflow");
    }

    pub(crate) const fn reset_counters(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// Per-frame (hits, misses) of the node memos.
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    pub(crate) const fn stats(&self) -> (u32, u32) {
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
}
