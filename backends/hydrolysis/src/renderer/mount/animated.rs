//! Per-view animation slots: the engine-run motions a wrapper hands its
//! layer, and the slot lifecycle every per-view slot follows.

use std::rc::Rc;

use cherenkov_record::Animatable;
use nami::Signal;

use super::Dirty;
use super::cell::NodeCell;
use super::motion::Motion;
use crate::renderer::SemanticCore;

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

    /// A wrapper property's engine-run motion following `signal` through
    /// `map`, on this window's frame clock and placement clock
    /// (water-rs/waterui#2203), re-recording `owner` when a record waits on
    /// it. With `gate`, the motion decides whether `owner`'s record can
    /// show, and a change or a track's end that flips that re-records it.
    pub(crate) fn wrapper_motion<S, T>(
        &self,
        signal: &S,
        map: fn(f32) -> T,
        owner: &Rc<NodeCell<E>>,
        gate: bool,
    ) -> Motion<T>
    where
        S: Signal<Output = f32> + 'static,
        T: Animatable + PartialEq,
    {
        let owner = Rc::downgrade(owner);
        let record = Box::new(move || {
            if let Some(cell) = owner.upgrade() {
                cell.mark(Dirty::PAINT);
            }
        });
        Motion::new(
            signal,
            map,
            self.signals.clone(),
            Rc::clone(&self.placement_clock),
            record,
            gate,
        )
    }

    /// Binds slot `slot` of the view retained as `owner` to `motion` for
    /// this rebuild: the settle queries, the subtree's animation scope and
    /// the one settle re-record see it, though nothing samples it.
    pub(crate) fn bind_motion<T, O: 'static>(
        &mut self,
        motion: &Motion<T>,
        owner: &Rc<O>,
        slot: usize,
    ) where
        T: Animatable + PartialEq,
    {
        let key = self.owned_scalar_key(owner, slot);
        self.animation_controller
            .bind_motion(key, Rc::clone(motion.status()));
    }
}
