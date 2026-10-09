//! Per-view animation slots: the animated transform/opacity/morph inputs a
//! node re-samples when it records, and the slot lifecycle every per-view
//! slot follows.

use std::cell::Cell;
use std::rc::Rc;

use nami::Signal;

use super::Dirty;
use crate::animation::AnimationKey;
use crate::renderer::signals::SubscribedSnapshot;
use crate::renderer::{Retain, SemanticCore};

impl SemanticCore {
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
    ) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let now = self.frame_instant;
        let key = self.owned_scalar_key(owner, slot);
        let (subscription, observed_value) = SubscribedSnapshot::new(signal);
        let handle = self
            .animation_controller
            .bind_scalar(key, observed_value, now);
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

    /// Samples the time-based morph phase of the shape retained as `owner`,
    /// so the timeline survives frames and structural changes around it.
    pub fn sample_morph_progress<T: 'static>(
        &mut self,
        animation: waterui_shape::MorphAnimation,
        owner: &Rc<T>,
    ) -> f32 {
        if animation.duration.is_zero() {
            return 1.0;
        }
        let key = AnimationKey::renderer_local_repeating(self.animation_owner_pins.pin(owner));
        let elapsed = self.animation_controller.bind_timeline_phase(
            key,
            animation.duration,
            animation.repeat,
            self.frame_instant,
        );
        let raw = elapsed.as_secs_f32() / animation.duration.as_secs_f32();
        let cycle = if animation.repeat {
            let base = raw.fract();
            assert!(
                raw.is_finite() && raw >= 0.0,
                "morph animation cycle index must be finite and non-negative"
            );
            let index = crate::num_cast::f32_as_u64(raw.floor());
            if animation.autoreverse && index % 2 == 1 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        animation.easing.ease(cycle).clamp(0.0, 1.0)
    }
}
