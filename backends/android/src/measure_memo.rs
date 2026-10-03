//! The `sizeThatFits` memo the Apple backend carries, ported verbatim in
//! shape: proposals are the only inputs the layout spec allows a measure to
//! read, so a child measured twice in one pass with the same proposal must
//! answer identically — and must not pay the JNI round-trip for it.
//!
//! The generation counter is the invalidation channel: anything that can
//! change what a measure would answer — content changes, child membership,
//! layout invalidation — bumps it, and every memo recomputes on the next
//! call. Entries are per-leaf and live in the leaf's `SubView` wrapper, so a
//! dropped leaf frees its own cache with no registry sweep.

use alloc::boxed::Box;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;

use waterui_core::layout::{
    ProposalSize, Size, StretchAxis, SubView, ViewDimensions,
};

/// The measure epoch. Bump on every change a cached measure could be stale
/// for; a fresh pass sees a new epoch and recomputes.
static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Invalidates every memoized measure — the counterpart of Apple's
/// `MeasureMemo.invalidate`.
pub(crate) fn invalidate() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Packs a proposal into the memo key. `None` encodes as zero and `Some(v)`
/// as `v`'s bits plus one, so `None` and `Some(0.0)` never collide.
fn key(proposal: ProposalSize) -> u64 {
    let width = proposal.width.map_or(0, |w| w.to_bits().wrapping_add(1));
    let height = proposal.height.map_or(0, |h| h.to_bits().wrapping_add(1));
    (u64::from(width) << 32) | u64::from(height)
}

/// The per-leaf memo: proposal bits → `(generation, answer)`. Bounded — past
/// the cap the map resets rather than growing on adversarial proposals.
#[derive(Debug, Default)]
pub(crate) struct MeasureMemo {
    entries: RefCell<HashMap<u64, (u64, ViewDimensions)>>,
}

impl MeasureMemo {
    /// The cap; a larger map would thrash on scroll-driven proposal streams.
    const CAP: usize = 128;

    fn lookup(&self, proposal: ProposalSize) -> Option<ViewDimensions> {
        self.entries
            .borrow()
            .get(&key(proposal))
            .and_then(|(generation, answer)| {
                (*generation == GENERATION.load(Ordering::Relaxed)).then(|| answer.clone())
            })
    }

    fn store(&self, proposal: ProposalSize, answer: ViewDimensions) {
        let mut entries = self.entries.borrow_mut();
        if entries.len() >= Self::CAP {
            entries.clear();
        }
        entries.insert(
            key(proposal),
            (GENERATION.load(Ordering::Relaxed), answer),
        );
    }
}

/// A `SubView` that memoizes `measure` — wrapped around every leaf's layout
/// face by [`crate::contract::NativeLeaf::new`], so no handler opts out.
#[derive(Debug)]
pub(crate) struct MemoizingSubView {
    inner: Box<dyn SubView>,
    memo: MeasureMemo,
}

impl MemoizingSubView {
    /// Wraps `inner` — constructed inside `NativeLeaf::new`, never by hand.
    pub(crate) fn new(inner: Box<dyn SubView>) -> Self {
        Self {
            inner,
            memo: MeasureMemo::default(),
        }
    }
}

impl SubView for MemoizingSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        if let Some(answer) = self.memo.lookup(proposal) {
            return answer;
        }
        let answer = self.inner.measure(proposal);
        self.memo.store(proposal, answer);
        answer
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
    use super::*;
    use alloc::rc::Rc;
    use core::cell::Cell;

    /// A `SubView` that counts its measure calls through a shared cell.
    struct CountingSubView {
        calls: Rc<Cell<u32>>,
        answer: ViewDimensions,
    }

    impl SubView for CountingSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            self.calls.set(self.calls.get() + 1);
            self.answer
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
    }

    /// A memoized leaf answering `width`×`height`, plus its call counter.
    fn counting(width: f32, height: f32) -> (Rc<Cell<u32>>, MemoizingSubView) {
        let calls = Rc::new(Cell::new(0u32));
        let view = MemoizingSubView::new(Box::new(CountingSubView {
            calls: Rc::clone(&calls),
            answer: ViewDimensions::new(Size::new(width, height)),
        }));
        (calls, view)
    }

    /// A full proposal: both axes offered.
    fn full(width: f32, height: f32) -> ProposalSize {
        ProposalSize {
            width: Some(width),
            height: Some(height),
        }
    }

    #[test]
    fn same_proposal_measures_once() {
        let (calls, view) = counting(10.0, 20.0);
        let proposal = full(100.0, 50.0);
        let first = view.measure(proposal);
        let second = view.measure(proposal);
        assert_eq!(first, second);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn distinct_proposals_remeasure() {
        let (calls, view) = counting(10.0, 20.0);
        view.measure(full(100.0, 50.0));
        view.measure(full(40.0, 50.0));
        view.measure(ProposalSize::UNSPECIFIED);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn none_and_zero_never_collide() {
        let (calls, view) = counting(10.0, 20.0);
        view.measure(full(0.0, 50.0));
        view.measure(ProposalSize {
            width: None,
            height: Some(50.0),
        });
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn invalidation_forces_remeasure() {
        let (calls, view) = counting(10.0, 20.0);
        let proposal = full(100.0, 50.0);
        view.measure(proposal);
        invalidate();
        view.measure(proposal);
        assert_eq!(calls.get(), 2);
    }
}
