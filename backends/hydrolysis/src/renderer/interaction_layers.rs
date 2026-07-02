//! Replayable interaction state layers.
//!
//! Hover, press, and focus visuals are captured once per structural rebuild as
//! retained fragments wrapped in [`DynamicOpacityDraw`]/[`DynamicTransformDraw`]
//! nodes whose scalars are renderer-local animation handles. Pointer
//! interactions then *replay* the retained window frame (re-sampling those
//! handles) instead of re-dispatching the view tree, which keeps hover/press
//! feedback on the cheap window-refresh path.

use super::*;
use waterui_backend_core::widget::{InteractionMotion, MAX_PRESS_WAVES, PressWave, PressWaves};

/// One press ripple wave slot: every press spawns its own wave (Material/mdui
/// semantics), so rapid re-presses overlap while older waves fade out.
#[derive(Debug)]
pub(crate) struct WaveLayer {
    alpha: AnimatedScalarHandle,
    progress: AnimatedScalarHandle,
    /// Origin of this wave's press in WINDOW coordinates (the raw pointer-down
    /// point). Widgets map it into their own local frame at draw time via
    /// `local_interaction_state` and the live hit transform, so it stays
    /// correct under arbitrary nesting and scroll offsets.
    origin: Cell<Option<vello::kurbo::Point>>,
    pressing: Cell<bool>,
    pressed_at: Cell<Option<Instant>>,
    released_at: Cell<Option<Instant>>,
    /// Monotonic press sequence, used to order sampled waves oldest-to-newest
    /// and to pick the recycle victim when every slot is still visible.
    seq: Cell<u64>,
}

impl WaveLayer {
    pub(crate) const fn new(alpha: AnimatedScalarHandle, progress: AnimatedScalarHandle) -> Self {
        Self {
            alpha,
            progress,
            origin: Cell::new(None),
            pressing: Cell::new(false),
            pressed_at: Cell::new(None),
            released_at: Cell::new(None),
            seq: Cell::new(0),
        }
    }

    /// Whether this wave should currently read as pressed: an active press, or
    /// a released press still inside the Material minimum press duration.
    pub(crate) fn visually_pressed(&self, minimum_press_duration: Duration, now: Instant) -> bool {
        if self.pressing.get() {
            return true;
        }
        if self.released_at.get().is_none() {
            return false;
        }
        self.pressed_at
            .get()
            .is_some_and(|pressed_at| now.duration_since(pressed_at) < minimum_press_duration)
    }

    fn visible(&self, now: Instant) -> bool {
        self.pressing.get() || self.alpha.sample(now) > 0.0
    }

    fn copy_state_from(&self, previous: &Self) {
        self.origin.set(previous.origin.get());
        self.pressing.set(previous.pressing.get());
        self.pressed_at.set(previous.pressed_at.get());
        self.released_at.set(previous.released_at.get());
        self.seq.set(previous.seq.get());
    }

    fn clear(&self) {
        self.origin.set(None);
        self.pressing.set(false);
        self.pressed_at.set(None);
        self.released_at.set(None);
    }
}

/// Shared handle bundle for one interactive widget's state layers.
///
/// Cloned into the widget's pointer/hover targets (and therefore into retained
/// subtrees), so input events occurring between structural rebuilds can apply
/// new animation targets directly without re-running `bind_widget_state`.
#[derive(Debug)]
pub(crate) struct InteractionLayerHandles {
    hover_alpha: AnimatedScalarHandle,
    waves: [WaveLayer; MAX_PRESS_WAVES],
    /// Next press sequence number handed to a spawned wave.
    next_wave_seq: Cell<u64>,
    hovering: Cell<bool>,
    /// Widget chrome (not just the state layer) samples interaction state, so
    /// hover/press changes must re-render the widget instead of replaying.
    chrome_state_dependent: Cell<bool>,
    motion: InteractionMotion,
}

impl InteractionLayerHandles {
    pub(crate) fn new(
        hover_alpha: AnimatedScalarHandle,
        waves: [WaveLayer; MAX_PRESS_WAVES],
        motion: InteractionMotion,
    ) -> Self {
        Self {
            hover_alpha,
            waves,
            next_wave_seq: Cell::new(1),
            hovering: Cell::new(false),
            chrome_state_dependent: Cell::new(false),
            motion,
        }
    }

    pub(crate) const fn wave(&self, index: usize) -> &WaveLayer {
        &self.waves[index]
    }

    /// Marks this widget's chrome as sampling interaction state directly, so
    /// hover/press changes escalate to a re-render instead of a replay.
    pub(crate) fn mark_chrome_state_dependent(&self) {
        self.chrome_state_dependent.set(true);
    }

    pub(crate) fn chrome_state_dependent(&self) -> bool {
        self.chrome_state_dependent.get()
    }

    pub(crate) fn hovering(&self) -> bool {
        self.hovering.get()
    }

    pub(crate) fn pressing(&self) -> bool {
        self.waves.iter().any(|wave| wave.pressing.get())
    }

    /// Whether any wave should currently read as pressed: an active press, or
    /// a released press still inside the Material minimum press duration.
    pub(crate) fn visually_pressed(&self, now: Instant) -> bool {
        self.waves
            .iter()
            .any(|wave| wave.visually_pressed(self.motion.minimum_press_duration, now))
    }

    /// Seeds the hover flag for a freshly created handle bundle.
    pub(crate) fn set_initial_hovering(&self, hovering: bool) {
        self.hovering.set(hovering);
    }

    /// Carries interaction state across a structural rebuild from the handle
    /// bundle the previous capture used for the same widget slot.
    pub(crate) fn copy_interaction_state_from(&self, previous: &Self) {
        for (wave, previous_wave) in self.waves.iter().zip(&previous.waves) {
            wave.copy_state_from(previous_wave);
        }
        self.next_wave_seq.set(previous.next_wave_seq.get());
        self.hovering.set(previous.hovering.get());
    }

    /// Drops every wave whose press origin does not satisfy `retain`: a press
    /// must not migrate to a different widget that inherits this slot across a
    /// rebuild.
    pub(crate) fn retain_waves_with_origin(
        &self,
        mut retain: impl FnMut(vello::kurbo::Point) -> bool,
    ) {
        for wave in &self.waves {
            if !wave.origin.get().is_some_and(&mut retain) {
                wave.clear();
            }
        }
    }

    pub(crate) fn set_hovering(&self, hovering: bool, now: Instant) -> bool {
        if self.hovering.replace(hovering) == hovering {
            return false;
        }
        let (target, animation) = if hovering {
            (self.motion.hover_opacity, self.motion.hover_enter.clone())
        } else {
            (0.0, self.motion.hover_exit.clone())
        };
        self.hover_alpha.apply_target(target, Some(animation), now);
        true
    }

    /// Starts a press at a window-space origin by spawning a fresh wave: it
    /// grows from the origin while its layer fades in. Waves still fading from
    /// earlier presses keep fading independently (mdui semantics); when every
    /// slot is still visible the oldest wave is recycled.
    pub(crate) fn begin_press(&self, origin: vello::kurbo::Point, now: Instant) {
        let wave = self.spawn_wave(now);
        let seq = self.next_wave_seq.get();
        self.next_wave_seq
            .set(seq.checked_add(1).expect("press wave sequence overflow"));
        wave.seq.set(seq);
        wave.origin.set(Some(origin));
        wave.pressing.set(true);
        wave.pressed_at.set(Some(now));
        wave.released_at.set(None);
        wave.progress
            .apply_target(0.0, Some(Animation::linear(Duration::ZERO)), now);
        wave.progress
            .apply_target(1.0, Some(self.motion.press_grow.clone()), now);
        wave.alpha.apply_target(
            self.motion.pressed_opacity,
            Some(self.motion.press_fade_in.clone()),
            now,
        );
    }

    /// The slot a new press claims: the first invisible wave, or the oldest
    /// visible one when every slot is still fading.
    fn spawn_wave(&self, now: Instant) -> &WaveLayer {
        self.waves
            .iter()
            .find(|wave| !wave.visible(now))
            .unwrap_or_else(|| {
                self.waves
                    .iter()
                    .min_by_key(|wave| wave.seq.get())
                    .expect("interaction handles hold at least one wave slot")
            })
    }

    /// Ends the active press. Each wave's fade-out is deferred until its
    /// Material minimum press duration has elapsed; [`Self::flush_release`]
    /// applies it from the animation tick.
    pub(crate) fn release(&self, now: Instant) -> bool {
        let mut changed = false;
        for wave in &self.waves {
            if wave.pressing.replace(false) {
                wave.released_at.set(Some(now));
                changed = true;
            }
        }
        if changed {
            self.flush_release(now);
        }
        changed
    }

    /// Applies deferred press fade-outs once each wave's minimum press
    /// duration has elapsed. Returns `true` while a release is still pending,
    /// so the frame pump keeps scheduling animation frames until every
    /// fade-out has started.
    pub(crate) fn flush_release(&self, now: Instant) -> bool {
        let mut pending = false;
        for wave in &self.waves {
            if wave.pressing.get() || wave.released_at.get().is_none() {
                continue;
            }
            let minimum_elapsed = wave.pressed_at.get().is_none_or(|pressed_at| {
                now.duration_since(pressed_at) >= self.motion.minimum_press_duration
            });
            if !minimum_elapsed {
                pending = true;
                continue;
            }
            wave.released_at.set(None);
            wave.pressed_at.set(None);
            wave.alpha
                .apply_target(0.0, Some(self.motion.press_fade_out.clone()), now);
        }
        pending
    }

    /// Drops all press state without animating (slot reuse by an unrelated
    /// widget across a rebuild).
    pub(crate) fn clear_press_state(&self) {
        for wave in &self.waves {
            wave.clear();
        }
    }

    /// Whether a released press is still waiting for its deferred fade-out
    /// (without applying it).
    pub(crate) fn has_pending_release(&self, now: Instant) -> bool {
        self.waves.iter().any(|wave| {
            !wave.pressing.get()
                && wave.released_at.get().is_some()
                && wave.pressed_at.get().is_some_and(|pressed_at| {
                    now.duration_since(pressed_at) < self.motion.minimum_press_duration
                })
        })
    }

    /// Samples the currently visible waves, ordered oldest to newest, with
    /// origins in window coordinates.
    pub(crate) fn sample_waves(&self, now: Instant) -> PressWaves {
        let mut order: [usize; MAX_PRESS_WAVES] = core::array::from_fn(|index| index);
        order.sort_unstable_by_key(|index| self.waves[*index].seq.get());
        let mut sampled = PressWaves::EMPTY;
        for index in order {
            let wave = &self.waves[index];
            if !wave.visible(now) {
                continue;
            }
            sampled.push(PressWave {
                origin: wave.origin.get(),
                progress: wave.progress.sample(now),
                opacity: wave.alpha.sample(now),
            });
        }
        sampled
    }
}

impl HydrolysisRenderer {
    /// Applies any deferred press fade-outs and reports whether more
    /// animation frames are needed for pending releases.
    pub(crate) fn flush_interaction_releases(&mut self, now: Instant) -> bool {
        let mut pending = false;
        for target in &self.hit_test.pointer_targets {
            if let Some(handles) = &target.interaction {
                pending |= handles.flush_release(now);
            }
        }
        pending
    }

    /// Whether any released press is still waiting for its deferred fade-out.
    pub(crate) fn has_pending_interaction_releases(&self, now: Instant) -> bool {
        self.hit_test.pointer_targets.iter().any(|target| {
            target
                .interaction
                .as_ref()
                .is_some_and(|handles| handles.has_pending_release(now))
        })
    }
}
