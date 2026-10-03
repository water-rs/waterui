//! `WuiProposalAware` — the delivery channel that hands a placed leaf the
//! proposal its parent negotiated for it.
//!
//! The layout spec forbids size negotiation during `layoutSubviews`
//! (`docs/layout-spec.md` rule L-2): the leaf's own `SubView` is measured
//! with the *offered* proposal, but once the parent places it, the leaf must
//! see the *selected* proposal to lay its own content out under the same
//! constraint the parent assumed. The parent's `layout.place` answers
//! `SubviewPlacement { frame, proposal }`; the container delivers `proposal`
//! through this registry before writing the frame.
//!
//! Each entry is keyed by the view's global-reference identity — the raw
//! jobject, which a `Global` pins for its life — and holds an `Rc` sink.
//! Where the Apple backend keeps the map in a `thread_local!`, this port
//! threads it: [`Proposals`] is a shared map the runtime's
//! [`crate::jvm::Platform`] owns and every [`SinkGuard`] clones by `Rc`, so
//! a leaf can unregister itself without reaching for a static. Delivery and
//! registration both run on the main looper, so `RefCell` is the whole
//! synchronization story.

use alloc::rc::Rc;
use core::cell::RefCell;
use std::collections::HashMap;

use waterui_core::layout::ProposalSize;

use crate::contract::PlatformView;

/// A leaf's selected-proposal sink.
pub type ProposalSink = Rc<dyn Fn(ProposalSize)>;

/// The view's registry key: the raw global reference, which a `Global` pins
/// until the reference drops — the mirror of an Apple view's address.
fn key(view: &PlatformView) -> usize {
    view.as_raw() as usize
}

/// The proposal channel map, owned by the runtime's `Platform` and shared
/// by `Rc` with each `SinkGuard`. view key → the leaf's selected-proposal
/// sink.
#[derive(Clone, Default)]
pub struct Proposals(Rc<RefCell<HashMap<usize, ProposalSink>>>);

impl core::fmt::Debug for Proposals {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Proposals")
            .field("channels", &self.0.borrow().len())
            .finish()
    }
}

impl Proposals {
    /// An empty channel map — built by `Platform::new`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Delivers `proposal` to the leaf owning `view`, if it registered a
    /// sink. Delivery is the parent's half of rule L-2: it runs immediately
    /// before the frame write so the leaf's own layout pass sees the
    /// selected constraint.
    pub fn deliver(&self, view: &PlatformView, proposal: ProposalSize) {
        if let Some(sink) = self.0.borrow().get(&key(view)) {
            sink(proposal);
        }
    }

    /// Delivers to the leaf owning `key` — the low-level form for a caller
    /// that already holds the registry key.
    #[allow(dead_code, reason = "key-level delivery arrives with later ports")]
    pub fn deliver_key(&self, key: usize, proposal: ProposalSize) {
        if let Some(sink) = self.0.borrow().get(&key) {
            sink(proposal);
        }
    }

    /// Registers `sink` as the selected-proposal callback for `view`,
    /// answering the guard that unregisters it. A leaf calls this for views
    /// it lays out itself — containers and every leaf that draws its own
    /// content.
    pub fn register_sink(
        &self,
        view: &PlatformView,
        sink: impl Fn(ProposalSize) + 'static,
    ) -> SinkGuard {
        let key = key(view);
        self.0.borrow_mut().insert(key, Rc::new(sink));
        SinkGuard {
            key,
            proposals: self.clone(),
        }
    }
}

/// The guard a leaf keeps to withdraw its sink on drop.
pub struct SinkGuard {
    key: usize,
    proposals: Proposals,
}

impl core::fmt::Debug for SinkGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SinkGuard")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        self.proposals.0.borrow_mut().remove(&self.key);
    }
}
