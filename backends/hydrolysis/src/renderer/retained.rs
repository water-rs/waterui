//! Animated-scalar resolution and morph-progress sampling for the retained render
//! tree. These re-sample animated transform/opacity/morph signals every flush so
//! the node tree's transform/opacity/morph nodes stay live without re-dispatching.

use super::*;

pub(crate) fn affine_near(left: vello::kurbo::Affine, right: vello::kurbo::Affine) -> bool {
    left.as_coeffs()
        .iter()
        .zip(right.as_coeffs())
        .all(|(left, right)| (*left - right).abs() <= 0.001)
}

pub(crate) fn rect_near(left: vello::kurbo::Rect, right: vello::kurbo::Rect) -> bool {
    (left.x0 - right.x0).abs() <= 0.001
        && (left.y0 - right.y0).abs() <= 0.001
        && (left.x1 - right.x1).abs() <= 0.001
        && (left.y1 - right.y1).abs() <= 0.001
}

impl HydrolysisRenderer {
    #[cfg(test)]
    pub(crate) fn scene_is_empty(&self) -> bool {
        !scene_has_content(&self.scene)
    }

    pub(super) fn resolve_animated_scalar_with_discriminator<S>(
        &mut self,
        signal: &S,
        discriminator: usize,
    ) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return signal.get();
        };
        let now = self.frame_instant;
        let key = AnimationKey::scalar_with_discriminator(identity, discriminator);
        let handle = self
            .animation_controller
            .bind_scalar(key, signal.get(), now);
        let watcher_handle = handle.clone();
        let signals = self.signals.clone();
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, signals.frame_clock());
            signals.request_redraw();
        });
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        handle.sample(now)
    }

    /// Sample a time-based shape-morph phase. `node_id` is the stable identity of
    /// the owning morph node (its retained `Rc` address), so the timeline slot keys
    /// off node identity and survives across frames and structural changes — unlike a
    /// positional `render_depth`, which shifts when a sibling subtree's node count
    /// changes and would restart the morph mid-animation.
    pub(crate) fn sample_morph_progress(
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
            let index = raw.floor() as u64;
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
