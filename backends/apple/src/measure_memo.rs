//! Persistent measure memoization at the leaf boundary.
//!
//! Core's `with_memoized_children` dedups child measures inside a single
//! layout call, but nothing dedups across calls: a nested stack negotiates
//! each leaf at a handful of distinct proposals per level, and an unmemoized
//! leaf re-runs its whole compute — probing every child — on every consult.
//! Call counts then multiply by the probe fan-out at each level, which is
//! exponential over nested containers (a depth-8 eager stack drives hundreds
//! of millions of leaf measures and never reaches first paint).
//!
//! Every `NativeLeaf` wraps its `SubView` in a [`MemoizingSubView`], so each
//! consult at a repeated proposal hits the leaf's cache instead of
//! re-descending its subtree — containers, delegates, metadata wrappers and
//! fallback leaves alike. Entries are keyed on the proposal's bits and stamped
//! with the global generation; any measure-relevant mutation anywhere calls
//! [`invalidate`], which bumps the generation and stales every cache at
//! once. The policy is deliberately conservative — a leaf that forgets to
//! invalidate is the failure mode, so every port calls `invalidate` from the
//! same places it would request a relayout, plus content mutations that
//! resize without one (text rebuilds, container resyncs, lazy viewport
//! measurements).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

/// The negotiation epoch: bumped by every measure-affecting mutation.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Marks every memoized measure stale tree-wide. Call it wherever a leaf's
/// measure-relevant inputs change — content rebuilds, child resyncs, bound
/// value updates that resize.
#[cfg(any(
    test,
    feature = "badge",
    feature = "container",
    feature = "dynamic",
    feature = "fixed_container",
    feature = "gpu_surface",
    feature = "list",
    feature = "menu",
    feature = "picker",
    feature = "plain",
    feature = "scroll",
    feature = "table",
    feature = "text",
    feature = "text_field"
))]
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// `ProposalSize` bits packed into a key — `None` is `0`, so `Some(-0.0)`
/// never aliases it and NaN proposals key consistently.
fn key(proposal: ProposalSize) -> u64 {
    let w = proposal.width.map_or(0_u64, |v| u64::from(v.to_bits()) + 1);
    let h = proposal
        .height
        .map_or(0_u64, |v| u64::from(v.to_bits()) + 1);
    (w << 32) | h
}

/// A leaf's `(proposal -> dimensions)` cache for the current epoch.
///
/// Entries outliving a bound flush lazily: at 128 entries the map clears,
/// which bounds memory and amortizes the re-miss into the next epoch.
#[derive(Default)]
struct MeasureMemo {
    inner: RefCell<HashMap<u64, (u64, ViewDimensions)>>,
}

impl MeasureMemo {
    /// Answers `compute()` cached on `proposal` for the current epoch.
    fn measure(
        &self,
        proposal: ProposalSize,
        compute: impl FnOnce() -> ViewDimensions,
    ) -> ViewDimensions {
        let generation = GENERATION.load(Ordering::Relaxed);
        let key = key(proposal);
        if let Some((stamped, dimensions)) = self.inner.borrow().get(&key)
            && *stamped == generation
        {
            return dimensions.clone();
        }
        let dimensions = compute();
        let mut map = self.inner.borrow_mut();
        if map.len() >= 128 {
            map.clear();
        }
        map.insert(key, (generation, dimensions.clone()));
        dimensions
    }
}

/// Wraps any leaf `SubView` in a [`MeasureMemo`]: repeated probes at the same
/// proposal hit the cache instead of re-running the leaf's compute. Wrapping
/// at the `NativeLeaf` boundary memoizes delegates (env/metadata wrappers,
/// `AnyView`, fallback leaves) that have no memo of their own.
pub struct MemoizingSubView {
    inner: Box<dyn SubView>,
    memo: MeasureMemo,
}

impl MemoizingSubView {
    pub fn new(inner: Box<dyn SubView>) -> Self {
        Self {
            inner,
            memo: MeasureMemo {
                inner: RefCell::new(HashMap::new()),
            },
        }
    }
}

impl fmt::Debug for MemoizingSubView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoizingSubView").finish_non_exhaustive()
    }
}

impl SubView for MemoizingSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.memo.measure(proposal, || self.inner.measure(proposal))
    }
    fn stretch_axis(&self) -> StretchAxis {
        self.inner.stretch_axis()
    }
    fn priority(&self) -> i32 {
        self.inner.priority()
    }
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use waterui_core::layout::{ProposalSize, Size, ViewDimensions};

    use super::{MeasureMemo, invalidate, key};

    fn dimensions(width: f32) -> ViewDimensions {
        ViewDimensions::new(Size::new(width, 0.0))
    }

    #[test]
    fn distinct_proposals_key_distinctly() {
        let free = ProposalSize::new(None, None);
        let zero = ProposalSize::new(Some(0.0), Some(0.0));
        let negative_zero = ProposalSize::new(Some(-0.0), Some(0.0));
        // `None` is `0`, so a real bound must never alias it — even the
        // all-zero-bit `Some(-0.0)`.
        assert_ne!(key(free), key(zero));
        assert_ne!(key(zero), key(negative_zero));
        assert_ne!(
            key(ProposalSize::new(Some(1.0), None)),
            key(ProposalSize::new(None, Some(1.0)))
        );
        // NaN proposals key consistently — same bits, same key.
        assert_eq!(
            key(ProposalSize::new(Some(f32::NAN), None)),
            key(ProposalSize::new(Some(f32::NAN), None))
        );
    }

    #[test]
    fn a_repeated_proposal_computes_once() {
        let memo = MeasureMemo::default();
        let calls = AtomicUsize::new(0);
        let compute = || {
            calls.fetch_add(1, AtomicOrdering::Relaxed);
            dimensions(7.0)
        };
        let proposal = ProposalSize::new(Some(100.0), None);
        memo.measure(proposal, compute);
        memo.measure(proposal, compute);
        memo.measure(ProposalSize::new(Some(50.0), None), compute);
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 2);
    }

    #[test]
    fn invalidation_recomputes_every_leaf() {
        let memo = MeasureMemo::default();
        let calls = AtomicUsize::new(0);
        let compute = || {
            calls.fetch_add(1, AtomicOrdering::Relaxed);
            dimensions(7.0)
        };
        let proposal = ProposalSize::new(Some(100.0), None);
        memo.measure(proposal, compute);
        invalidate();
        memo.measure(proposal, compute);
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 2);
    }

    #[test]
    fn the_cache_stays_bounded() {
        let memo = MeasureMemo::default();
        let mut value = 0.0_f32;
        for _ in 0..300 {
            value += 1.0;
            memo.measure(ProposalSize::new(Some(value), Some(value)), || {
                dimensions(value)
            });
        }
        // The 128-entry bound cleared the map at least once rather than
        // growing without limit.
        assert!(memo.inner.borrow().len() <= 128);
    }
}
