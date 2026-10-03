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
//! jobject, which a `Global` pins for its life — and holds an `Rc` sink. The
//! whole map is main-thread state: delivery and registration both happen on
//! the main looper, so a thread-local `HashMap` is the whole story.

use alloc::rc::Rc;
use core::cell::RefCell;
use std::collections::HashMap;

use waterui_core::layout::ProposalSize;

use crate::contract::PlatformView;

thread_local! {
    /// view key → the leaf's selected-proposal sink.
    static CHANNELS: RefCell<HashMap<usize, Rc<dyn Fn(ProposalSize)>>> = RefCell::new(HashMap::new());
}

/// The view's registry key: the raw global reference, which a `Global` pins
/// until the reference drops — the mirror of an Apple view's address.
fn key(view: &PlatformView) -> usize {
    view.as_raw() as usize
}

/// Delivers `proposal` to the leaf owning `view`, if it registered a sink.
/// Delivery is the parent's half of rule L-2: it runs immediately before the
/// frame write so the leaf's own layout pass sees the selected constraint.
pub(crate) fn deliver(view: &PlatformView, proposal: ProposalSize) {
    CHANNELS.with(|channels| {
        if let Some(sink) = channels.borrow().get(&key(view)) {
            sink(proposal);
        }
    });
}

/// Delivers to the leaf owning `key` — the low-level form for a caller that
/// already holds the registry key.
#[allow(dead_code, reason = "key-level delivery arrives with later ports")]
pub(crate) fn deliver_key(key: usize, proposal: ProposalSize) {
    CHANNELS.with(|channels| {
        if let Some(sink) = channels.borrow().get(&key) {
            sink(proposal);
        }
    });
}

/// The guard a leaf keeps to withdraw its sink on drop.
#[derive(Debug)]
pub(crate) struct SinkGuard {
    key: usize,
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        CHANNELS.with(|channels| {
            channels.borrow_mut().remove(&self.key);
        });
    }
}

/// Registers `sink` as the selected-proposal callback for `view`, answering
/// the guard that unregisters it. A leaf calls this for views it lays out
/// itself — containers and every leaf that draws its own content.
pub(crate) fn register_sink(
    view: &PlatformView,
    sink: impl Fn(ProposalSize) + 'static,
) -> SinkGuard {
    let key = key(view);
    CHANNELS.with(|channels| {
        channels.borrow_mut().insert(key, Rc::new(sink));
    });
    SinkGuard { key }
}
