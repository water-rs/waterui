//! The `sizeThatFits` memo the Apple backend carries, ported verbatim in
//! shape: proposals are the only inputs the layout spec allows a measure to
//! read, so a child measured twice in one pass with the same proposal must
//! answer identically — and must not pay the JNI round-trip for it.
//!
//! The epoch counter is the invalidation channel: anything that can change
//! what a measure would answer — content changes, child membership, layout
//! invalidation — bumps it, and every memo recomputes on the next call.
//! Where the Apple backend keeps that counter in a process-static, this
//! port threads it: [`MeasureEpoch`] is a shared cell the runtime's
//! [`crate::jvm::Platform`] owns and every memo clones by `Rc`. Entries are
//! per-leaf and live in the leaf's `SubView` wrapper, so a dropped leaf frees
//! its own cache with no registry sweep.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::collections::HashMap;

use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

/// The measure epoch, shared explicitly: the [`crate::jvm::Platform`] owns
/// one, and every [`MeasureMemo`] holds a clone. Main-thread state — all
/// measure and invalidation traffic is main-looper work — so a `Cell`
/// inside an `Rc` is the whole story; no atomic needed.
#[derive(Debug, Clone)]
pub struct MeasureEpoch(Rc<Cell<u64>>);

impl MeasureEpoch {
    /// A fresh epoch starting at 1, so a never-stamped entry (`0`) can
    /// never read as current.
    pub fn new() -> Self {
        Self(Rc::new(Cell::new(1)))
    }

    /// Marks every memoized measure stale — the counterpart of Apple's
    /// `MeasureMemo.invalidate`.
    pub fn bump(&self) {
        self.0.set(self.0.get() + 1);
    }

    /// The current epoch, read once per memoized call.
    fn current(&self) -> u64 {
        self.0.get()
    }
}

impl Default for MeasureEpoch {
    fn default() -> Self {
        Self::new()
    }
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
pub struct MeasureMemo {
    entries: RefCell<HashMap<u64, (u64, ViewDimensions)>>,
}

impl MeasureMemo {
    /// The cap; a larger map would thrash on scroll-driven proposal streams.
    const CAP: usize = 128;

    fn lookup(&self, proposal: ProposalSize, generation: u64) -> Option<ViewDimensions> {
        self.entries
            .borrow()
            .get(&key(proposal))
            .and_then(|(stamped, answer)| (*stamped == generation).then(|| answer.clone()))
    }

    fn store(&self, proposal: ProposalSize, generation: u64, answer: ViewDimensions) {
        let mut entries = self.entries.borrow_mut();
        if entries.len() >= Self::CAP {
            entries.clear();
        }
        entries.insert(key(proposal), (generation, answer));
    }
}

/// A `SubView` that memoizes `measure` — wrapped around every leaf's layout
/// face by [`crate::contract::NativeLeaf::new`], so no handler opts out.
pub struct MemoizingSubView {
    inner: Box<dyn SubView>,
    memo: MeasureMemo,
    epoch: MeasureEpoch,
}

impl MemoizingSubView {
    /// Wraps `inner` around `epoch` — constructed inside `NativeLeaf::new`,
    /// never by hand. The epoch is the runtime's: cloned from the
    /// [`crate::jvm::Platform`] so `invalidate_measures` reaches this memo.
    pub fn new(inner: Box<dyn SubView>, epoch: MeasureEpoch) -> Self {
        Self {
            inner,
            memo: MeasureMemo::default(),
            epoch,
        }
    }
}

impl SubView for MemoizingSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let generation = self.epoch.current();
        if let Some(answer) = self.memo.lookup(proposal, generation) {
            return answer;
        }
        let answer = self.inner.measure(proposal);
        self.memo.store(proposal, generation, answer.clone());
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
    use waterui_core::layout::Size;

    /// A `SubView` that counts its measure calls through a shared cell.
    struct CountingSubView {
        calls: Rc<Cell<u32>>,
        answer: ViewDimensions,
    }

    impl SubView for CountingSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            self.calls.set(self.calls.get() + 1);
            self.answer.clone()
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    /// A memoized leaf answering `width`×`height`, plus its call counter
    /// and the epoch it reads.
    fn counting(width: f32, height: f32) -> (Rc<Cell<u32>>, MeasureEpoch, MemoizingSubView) {
        let calls = Rc::new(Cell::new(0u32));
        let epoch = MeasureEpoch::new();
        let view = MemoizingSubView::new(
            Box::new(CountingSubView {
                calls: Rc::clone(&calls),
                answer: ViewDimensions::new(Size::new(width, height)),
            }),
            epoch.clone(),
        );
        (calls, epoch, view)
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
        let (calls, _epoch, view) = counting(10.0, 20.0);
        let proposal = full(100.0, 50.0);
        let first = view.measure(proposal);
        let second = view.measure(proposal);
        assert_eq!(first, second);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn distinct_proposals_remeasure() {
        let (calls, _epoch, view) = counting(10.0, 20.0);
        let _ = view.measure(full(100.0, 50.0));
        let _ = view.measure(full(40.0, 50.0));
        let _ = view.measure(ProposalSize::UNSPECIFIED);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn none_and_zero_never_collide() {
        let (calls, _epoch, view) = counting(10.0, 20.0);
        let _ = view.measure(full(0.0, 50.0));
        let _ = view.measure(ProposalSize {
            width: None,
            height: Some(50.0),
        });
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn invalidation_forces_remeasure() {
        let (calls, epoch, view) = counting(10.0, 20.0);
        let proposal = full(100.0, 50.0);
        let _ = view.measure(proposal);
        epoch.bump();
        let _ = view.measure(proposal);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn epochs_are_per_instance() {
        // Two runtimes never share an epoch: a bump on one must not stale
        // the other's memos.
        let (calls, _epoch_a, view) = counting(10.0, 20.0);
        let proposal = full(100.0, 50.0);
        let _ = view.measure(proposal);
        MeasureEpoch::new().bump();
        let _ = view.measure(proposal);
        assert_eq!(calls.get(), 1);
    }
}
