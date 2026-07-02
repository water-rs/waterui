//! Reactive inputs: signal watching and animated-value sampling that bind
//! WaterUI signals to frame triggers and the animation controller.

use super::*;

impl HydrolysisRenderer {
    pub(super) fn watch_signal<S>(&mut self, signal: &S)
    where
        S: Signal + Clone + 'static,
    {
        // A reactive *value* change re-flushes the retained tree (re-read, re-layout,
        // re-encode) — the cheap per-frame pump — instead of re-running the whole view
        // `body()`. Structural changes go through `Dynamic`/`when` (a patch), not a
        // plain signal read, so a refresh is sufficient here.
        let Some(identity) = signal.identity() else {
            // Identity-less signal: subscribe fresh each read, retained for one frame.
            let signals = self.signals.clone();
            let guard = signal.watch(move |_| signals.request_refresh());
            self.lifecycle.current_frame_retain.push(Retain::new(guard));
            return;
        };
        // Identity-stable signal: one subscription per signal, reused across frames
        // for as long as the flush keeps reading it (see `SignalWatchRegistry`).
        let key = identity.raw();
        let signal_type = core::any::TypeId::of::<S>();
        if self.lifecycle.signal_watches.mark_seen(key, signal_type) {
            return;
        }
        let signals = self.signals.clone();
        let guard = signal.watch(move |_| signals.request_refresh());
        self.lifecycle.signal_watches.insert(
            key,
            signal_type,
            Box::new(signal.clone()),
            Retain::new(guard),
        );
    }

    pub(crate) fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        self.watch_signal(signal);
        signal.get()
    }

    pub(crate) fn read_resolved_text_styled(
        &mut self,
        text: &Text,
        env: &Environment,
    ) -> StyledStr {
        let resolved = text.resolve(env);
        self.read_signal(&resolved.content)
    }

    pub(crate) fn set_frame_instant(&mut self, at: Instant) {
        self.frame_instant = at;
        self.signals.set_frame_clock(at);
    }

    pub(crate) fn frame_instant(&self) -> Instant {
        self.frame_instant
    }

    pub(crate) fn resolve_toggle_progress<S>(
        &mut self,
        signal: &S,
        default_animation: Animation,
    ) -> f32
    where
        S: Signal<Output = bool> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return if signal.get() { 1.0 } else { 0.0 };
        };
        let now = self.frame_instant;
        let target = if signal.get() { 1.0 } else { 0.0 };
        let key = AnimationKey::scalar(identity);
        let handle = self.animation_controller.bind_scalar_target(
            key,
            target,
            default_animation.clone(),
            now,
        );
        let watcher_handle = handle.clone();
        let signals = self.signals.clone();
        let guard = signal.watch(move |update| {
            let target = if *update.value() { 1.0 } else { 0.0 };
            let animation = update
                .metadata()
                .try_get::<Animation>()
                .unwrap_or_else(|| default_animation.clone());
            watcher_handle.apply_target(target, Some(animation), signals.frame_clock());
            signals.request_refresh();
        });
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        handle.sample(now).clamp(0.0, 1.0)
    }

    pub(crate) fn sample_widget_scalar_target(
        &mut self,
        key: AnimationKey,
        target: f32,
        animation: Animation,
    ) -> f32 {
        let now = self.frame_instant;
        self.animation_controller
            .bind_scalar_target(key, target, animation, now)
            .sample(now)
    }

    pub(crate) fn sample_radio_indicator_state(
        &mut self,
        key: AnimationKey,
        selected: bool,
        motion: &RadioSelectionMotion,
    ) -> RadioIndicatorState {
        self.animation_controller
            .bind_radio_indicator(key, selected, motion, self.frame_instant)
    }

    /// Sample a free-running repeating phase (e.g. an indeterminate progress
    /// indicator). `node_id` is the stable identity of the owning node (its retained
    /// `Rc` address), so the phase slot keys off node identity and survives across
    /// frames and structural changes — unlike a positional `render_depth`, which
    /// shifts when a sibling subtree's node count changes and would reset the phase.
    pub(crate) fn sample_repeating_motion(&mut self, cycle: Duration, node_id: usize) -> Duration {
        let key = AnimationKey::renderer_local_repeating(node_id);
        self.animation_controller
            .bind_repeating_phase(key, cycle, self.frame_instant)
    }

    pub fn advance_animations(&mut self) -> bool {
        let now = self.frame_instant;
        let pending_releases = self.flush_interaction_releases(now);
        self.animation_controller.tick(now)
            || pending_releases
            || self.navigation.slots.iter().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
            })
    }

    pub fn animations_active(&self) -> bool {
        let now = self.frame_instant;
        self.animation_controller.has_active(now)
            || self.has_pending_interaction_releases(now)
            || self.navigation.slots.iter().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
            })
    }
}
