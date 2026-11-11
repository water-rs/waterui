//! Per-view animation slots: the animated transform/opacity/morph inputs a
//! node re-samples when it records, and the slot lifecycle every per-view
//! slot follows.

use std::cell::Cell;
use std::rc::Rc;

use nami::Signal;

use super::Dirty;
use crate::renderer::signals::SubscribedSnapshot;
use crate::renderer::{Retain, SemanticCore};

impl<E: crate::text::TextEngine> SemanticCore<E> {
    /// Opens a structural rebuild of the animation slots: from here, a slot
    /// survives the matching [`Self::retire_unbound_animation_slots`] only if
    /// it is bound again.
    pub(crate) fn begin_animation_rebuild(&mut self) {
        self.animation_controller.begin_rebuild_frame();
        self.animation_owner_pins.begin_rebuild_frame();
    }

    /// Retires every slot not bound since [`Self::begin_animation_rebuild`],
    /// releasing its owner's address with it.
    pub(crate) fn retire_unbound_animation_slots(&mut self) {
        self.animation_controller
            .finish_rebuild_frame_with_inactive_slot_retention(false);
        self.animation_owner_pins.finish_rebuild_frame();
    }

    /// Resolves slot `slot` of the view retained as `owner`, animating it
    /// along `signal` (see [`Self::owned_scalar_key`]).
    pub fn resolve_owned_scalar<S, T: 'static>(
        &mut self,
        signal: &S,
        owner: &Rc<T>,
        slot: usize,
        refresh: cherenkov_record::RefreshRange,
    ) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let now = self.frame_instant;
        let key = self.owned_scalar_key(owner, slot);
        let (subscription, observed_value) = SubscribedSnapshot::new(signal);
        let handle = self
            .animation_controller
            .bind_scalar(key, observed_value, now, refresh);
        let watcher_handle = handle.clone();
        let signals = self.signals.clone();
        let marked = self.mark_owner_for_animation(key, Dirty::PAINT);
        // A subscription's registration emission echoes the value `bind`
        // already sampled — only a later fire may mark the owner.
        let armed = Rc::new(Cell::new(false));
        let armed_for_watch = Rc::clone(&armed);
        let guard = subscription.activate(move |update| {
            watcher_handle.apply_update_from_context(update, signals.frame_clock());
            if armed_for_watch.get()
                && let Some(cell) = marked.upgrade()
            {
                cell.mark(Dirty::PAINT);
            }
        });
        armed.set(true);
        self.push_guard(Retain::new(guard));
        handle.sample(now)
    }
}
