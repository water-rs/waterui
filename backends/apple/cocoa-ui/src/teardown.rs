//! Owner-driven teardown for handler-holding views.
//!
//! A view that stores `Rc` handlers keeps whatever state those handlers
//! capture; that state is usually the owner's mounted subtree. If the view
//! outlives its owner — a superview or window can retain it — the slots
//! keep the released owner's state alive forever, and a callback the
//! platform still delivers reaches freed-by-contract state. The owner
//! therefore clears the slots at its own release boundary, which
//! [`HandlerTeardown`] performs: the guard is kept like any other owned
//! resource and clears the slots once, in the owner's normal drop order.

use std::fmt;

use objc2::rc::Retained;

/// A view whose installed handler slots clear as one teardown.
///
/// Clearing is slot-local and idempotent: every `set_*_handler` answers
/// `None` afterwards, so a callback the platform delivers to a view that
/// outlives its owner does nothing by construction instead of reaching
/// state the owner already released.
pub trait HandlerSlots {
    /// Drops every installed handler, releasing the state they capture.
    fn clear_handlers(&self);
}

/// Holds a [`HandlerSlots`] view and clears its handler slots on drop.
///
/// The owner keeps the guard like any other retained resource: it drops
/// in the owner's ordinary drop order, so the handler slots die exactly
/// where the owner's other teardown does rather than inside a per-type
/// special case.
pub struct HandlerTeardown<V: HandlerSlots> {
    view: Retained<V>,
}

impl<V: HandlerSlots> fmt::Debug for HandlerTeardown<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandlerTeardown").finish_non_exhaustive()
    }
}

impl<V: HandlerSlots> HandlerTeardown<V> {
    /// Holds `view`; its handler slots are cleared when the guard drops.
    #[must_use]
    pub const fn new(view: Retained<V>) -> Self {
        Self { view }
    }
}

impl<V: HandlerSlots> Drop for HandlerTeardown<V> {
    fn drop(&mut self) {
        self.view.clear_handlers();
    }
}
