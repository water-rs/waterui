//! Unified view-measurement caching.
//!
//! Hydrolysis keeps two kinds of measurement state with different lifetimes:
//!
//! - **Per-rebuild view dimensions** ([`MeasurementCaches::view_dimensions`]):
//!   keyed by view pointer identity, environment identity and layout proposal.
//!   Only valid while the view tree that produced the pointers is being
//!   dispatched, so it is cleared at the start of every structural rebuild.
//! - **Per-`Dynamic`-node dimensions** (intrinsic and proposal-dependent):
//!   keyed by the node's stable identity. A connected `Dynamic` owns its
//!   content inside the renderer, so layout passes that cannot re-measure the
//!   content read these entries instead. They persist across rebuilds, are
//!   refreshed whenever the node's content is (re-)dispatched, and are pruned
//!   to the identities that are still alive when a rebuild finishes.

use rustc_hash::FxHashMap;
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

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ViewMeasurementKey {
    view_identity: usize,
    env_identity: usize,
    proposal: ProposalKey,
}

#[derive(Default)]
pub(crate) struct MeasurementCaches {
    view_dimensions: FxHashMap<ViewMeasurementKey, ViewDimensions>,
    dynamic_intrinsic: FxHashMap<usize, ViewDimensions>,
    dynamic_proposal: FxHashMap<(usize, ProposalKey), ViewDimensions>,
    /// Re-entrancy guard: `Dynamic` nodes currently being measured.
    dynamic_measurement_stack: Vec<(usize, ProposalSize)>,
    hits: u32,
    misses: u32,
}

impl MeasurementCaches {
    /// Cached dimensions for a concrete view under a proposal, counting the
    /// lookup in the per-frame hit/miss statistics.
    pub(crate) fn view_dimensions(
        &mut self,
        view_identity: usize,
        env_identity: usize,
        proposal: ProposalSize,
    ) -> Option<ViewDimensions> {
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

    pub(crate) fn store_view_dimensions(
        &mut self,
        view_identity: usize,
        env_identity: usize,
        proposal: ProposalSize,
        dimensions: ViewDimensions,
    ) {
        let key = ViewMeasurementKey {
            view_identity,
            env_identity,
            proposal: proposal.into(),
        };
        self.view_dimensions.insert(key, dimensions);
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

    /// Invalidate the per-rebuild view-dimension entries; `Dynamic` node
    /// entries survive (their content is owned by the renderer and may not be
    /// re-measurable until the node is re-dispatched).
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.view_dimensions.clear();
        self.reset_counters();
    }

    /// Prune `Dynamic` entries whose node no longer exists in the view tree.
    pub(crate) fn retain_dynamic_identities(&mut self, is_alive: impl Fn(usize) -> bool) {
        self.dynamic_intrinsic
            .retain(|identity, _| is_alive(*identity));
        self.dynamic_proposal
            .retain(|(identity, _), _| is_alive(*identity));
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
    fn rebuild_clears_view_dimensions_but_keeps_dynamic_entries() {
        let mut caches = MeasurementCaches::default();
        caches.store_view_dimensions(1, 2, ProposalSize::UNSPECIFIED, dimensions(1.0, 1.0));
        caches.store_dynamic_dimensions(7, ProposalSize::UNSPECIFIED, dimensions(2.0, 2.0));
        caches.store_dynamic_dimensions(
            7,
            ProposalSize::new(Some(50.0), None),
            dimensions(3.0, 3.0),
        );

        caches.begin_rebuild_frame();

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
