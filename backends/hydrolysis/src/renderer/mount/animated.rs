//! Animated-scalar resolution and morph-progress sampling: the animated
//! transform/opacity/morph inputs a node re-samples when it records.

use std::cell::Cell;
use std::rc::Rc;

use nami::Signal;

use super::Dirty;
use crate::animation::AnimationKey;
use crate::renderer::signals::SubscribedSnapshot;
use crate::renderer::{Retain, SemanticCore};

impl SemanticCore {
    pub fn resolve_animated_scalar_with_discriminator<S>(
        &mut self,
        signal: &S,
        discriminator: usize,
    ) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return self.read_signal(signal);
        };
        let now = self.frame_instant;
        let key = AnimationKey::scalar_with_discriminator(identity, discriminator);
        let (subscription, observed_value) = SubscribedSnapshot::new(signal);
        let handle = self
            .animation_controller
            .bind_scalar(key, observed_value, now);
        let watcher_handle = handle.clone();
        let signals = self.signals.clone();
        let owner = self.mark_owner_for_animation(key, Dirty::PAINT);
        // A subscription's registration emission echoes the value `bind`
        // already sampled — only a later fire may mark the owner.
        let armed = Rc::new(Cell::new(false));
        let armed_for_watch = Rc::clone(&armed);
        let guard = subscription.activate(move |update| {
            watcher_handle.apply_update_from_context(update, signals.frame_clock());
            if armed_for_watch.get()
                && let Some(cell) = owner.upgrade()
            {
                cell.mark(Dirty::PAINT);
            }
        });
        armed.set(true);
        self.push_guard(Retain::new(guard));
        handle.sample(now)
    }

    /// Sample a time-based shape-morph phase. `node_id` is the stable identity of
    /// the owning morph node (its retained `Rc` address), so the timeline slot keys
    /// off node identity and survives across frames and structural changes — unlike a
    /// positional `render_depth`, which shifts when a sibling subtree's node count
    /// changes and would restart the morph mid-animation.
    pub fn sample_morph_progress(
        &mut self,
        animation: waterui_shape::MorphAnimation,
        node_id: usize,
    ) -> f32 {
        if animation.duration.is_zero() {
            return 1.0;
        }
        let key = AnimationKey::renderer_local_repeating(node_id);
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
