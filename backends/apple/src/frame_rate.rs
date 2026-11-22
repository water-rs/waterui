//! Per-window refresh-rate vote coordination across mounted views and active
//! frame clocks / Metal presenters.

use alloc::collections::BTreeMap;
use alloc::rc::{Rc, Weak};
use core::cell::RefCell;

use waterui_frame_rate::{FrameRate, combine_all};

#[derive(Default)]
struct VotesInner {
    next_id: u64,
    votes: BTreeMap<u64, FrameRate>,
    listeners: Vec<(u64, Rc<dyn Fn()>)>,
}

impl VotesInner {
    const fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    fn combined(&self) -> Option<FrameRate> {
        combine_all(self.votes.values().copied().map(Some))
    }
}

/// Per-window table of active `.frame_rate(...)` subtree votes.
///
/// Installed into each window's [`Environment`](waterui_backend_core::Environment)
/// so active, visible views contribute to presenters and display links on that
/// window alone.
#[derive(Clone, Default)]
pub struct WindowFrameRateVotes(Rc<RefCell<VotesInner>>);

impl core::fmt::Debug for WindowFrameRateVotes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WindowFrameRateVotes")
            .field("combined", &self.combined())
            .finish_non_exhaustive()
    }
}

impl WindowFrameRateVotes {
    /// Combined vote across all mounted and visible `.frame_rate(...)` nodes on
    /// this window, or `None` when none are active.
    #[must_use]
    pub fn combined(&self) -> Option<FrameRate> {
        self.0.borrow().combined()
    }

    /// Registers a vote slot for one mounted `.frame_rate(...)` node; dropping
    /// the returned registration withdraws the slot.
    #[must_use]
    pub fn register(&self) -> VoteRegistration {
        let id = self.0.borrow_mut().alloc_id();
        VoteRegistration {
            table: Rc::downgrade(&self.0),
            id,
        }
    }

    /// Subscribes a presenter or clock to changes in [`Self::combined`].
    /// Dropping the returned handle unsubscribes it.
    #[must_use]
    pub fn subscribe(&self, listener: impl Fn() + 'static) -> VoteListener {
        let mut inner = self.0.borrow_mut();
        let id = inner.alloc_id();
        inner.listeners.push((id, Rc::new(listener)));
        VoteListener {
            table: Rc::downgrade(&self.0),
            id,
        }
    }
}

fn update_vote(table: &Rc<RefCell<VotesInner>>, id: u64, vote: Option<FrameRate>) {
    let listeners = {
        let mut inner = table.borrow_mut();
        let before = inner.combined();
        let changed_entry = match vote {
            Some(rate) => inner.votes.insert(id, rate) != Some(rate),
            None => inner.votes.remove(&id).is_some(),
        };
        if !changed_entry || inner.combined() == before {
            return;
        }
        inner
            .listeners
            .iter()
            .map(|(_, cb)| Rc::clone(cb))
            .collect::<Vec<_>>()
    };
    for cb in listeners {
        cb();
    }
}

/// Owned slot for one mounted `.frame_rate(...)` node.
pub struct VoteRegistration {
    table: Weak<RefCell<VotesInner>>,
    id: u64,
}

impl core::fmt::Debug for VoteRegistration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VoteRegistration")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl VoteRegistration {
    /// Updates or withdraws (`None`) this node's active vote, notifying window
    /// subscribers if the combined window vote changed.
    pub fn set(&self, vote: Option<FrameRate>) {
        if let Some(table) = self.table.upgrade() {
            update_vote(&table, self.id, vote);
        }
    }
}

impl Drop for VoteRegistration {
    fn drop(&mut self) {
        if let Some(table) = self.table.upgrade() {
            update_vote(&table, self.id, None);
        }
    }
}

/// Owned listener handle keeping a presenter/clock subscribed to window vote changes.
pub struct VoteListener {
    table: Weak<RefCell<VotesInner>>,
    id: u64,
}

impl core::fmt::Debug for VoteListener {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VoteListener")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for VoteListener {
    fn drop(&mut self) {
        if let Some(table) = self.table.upgrade() {
            table
                .borrow_mut()
                .listeners
                .retain(|(id, _)| *id != self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;

    #[test]
    fn combines_active_votes_and_withdraws_on_drop_or_hide() {
        let window = WindowFrameRateVotes::default();
        let notifications = Rc::new(Cell::new(0usize));
        let _sub = window.subscribe({
            let notifications = Rc::clone(&notifications);
            move || notifications.set(notifications.get() + 1)
        });

        let a = window.register();
        let b = window.register();
        assert_eq!(window.combined(), None);

        a.set(Some(FrameRate::SLOW));
        assert_eq!(window.combined(), Some(FrameRate::SLOW));
        assert_eq!(notifications.get(), 1);

        // Adding a faster vote raises the combined vote.
        b.set(Some(FrameRate::FAST));
        assert_eq!(window.combined(), Some(FrameRate::FAST));
        assert_eq!(notifications.get(), 2);

        // Hiding/dropping the faster voter drops back to SLOW.
        drop(b);
        assert_eq!(window.combined(), Some(FrameRate::SLOW));
        assert_eq!(notifications.get(), 3);

        a.set(None);
        assert_eq!(window.combined(), None);
        assert_eq!(notifications.get(), 4);
    }

    #[test]
    fn separate_windows_keep_isolated_votes() {
        let w1 = WindowFrameRateVotes::default();
        let w2 = WindowFrameRateVotes::default();
        let r1 = w1.register();
        r1.set(Some(FrameRate::fixed(24.0)));
        assert_eq!(w1.combined(), Some(FrameRate::fixed(24.0)));
        assert_eq!(w2.combined(), None);
    }
}
