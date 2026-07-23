//! Animated scalar sampling, animation key registry, and renderer-local
//! motion state.
//!
//! [`AnimationController`] owns the per-frame registry of animation slots,
//! keyed by [`AnimationKey`] (signal identity, optionally mixed with a
//! per-instance discriminator, or a renderer-local identity). Slots are
//! rebound on every structural rebuild and survive across frames so
//! in-flight animations keep their progress; unbound slots are dropped when
//! the rebuild finishes. The controller covers three slot kinds: animated
//! scalars driven by `AnimationTrack<f32>`, the radio-indicator selection
//! choreography (inner dot scale/opacity plus outer ring color), and
//! free-running repeating/timeline phases for indeterminate indicators.

use core::time::Duration;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::widget::{RadioIndicatorState, RadioSelectionMotion};
use nami::SignalIdentity;
use nami::watcher::Context;
use waterui_core::animation::{Animation, AnimationTrack};

use crate::time::Instant;

const VALUE_EPSILON: f32 = 0.000_01;
const RENDERER_LOCAL_KEY_MASK: u64 = 1 << 63;

const fn mix_identity(identity: usize, discriminator: usize) -> u64 {
    (identity as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(7)
        ^ (discriminator as u64).wrapping_add(0x517C_C1B7_2722_0A95)
}

/// Registry of animation slots keyed by [`AnimationKey`], rebound on every
/// structural rebuild.
///
/// The renderer brackets each rebuild with
/// [`begin_rebuild_frame`](Self::begin_rebuild_frame) and
/// [`finish_rebuild_frame_with_inactive_slot_retention`](Self::finish_rebuild_frame_with_inactive_slot_retention),
/// binds slots during dispatch, and drives all in-flight animations with
/// [`tick`](Self::tick) once per frame.
#[derive(Debug, Default)]
pub struct AnimationController {
    slots: BTreeMap<AnimationKey, AnimatedScalarSlot>,
    radio_indicator_slots: BTreeMap<AnimationKey, AnimatedRadioIndicatorSlot>,
    repeating_slots: BTreeMap<AnimationKey, RepeatingPhaseSlot>,
    active_slots: BTreeSet<AnimationKey>,
    active_radio_indicator_slots: BTreeSet<AnimationKey>,
    active_repeating_slots: BTreeSet<AnimationKey>,
}

/// Stable identity of one animation slot across structural rebuilds.
///
/// Built either from the [`SignalIdentity`] of the reactive value driving
/// the animation (optionally mixed with a per-instance discriminator so
/// several widgets sharing a signal get distinct slots) or from a
/// renderer-local identity for animations the renderer owns itself. Keys of
/// different scopes (scalar, radio indicator, repeating) never collide.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AnimationKey {
    scope: AnimationKeyScope,
    identity: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum AnimationKeyScope {
    Scalar,
    RendererLocalScalar,
    RadioIndicator,
    Repeating,
}

#[derive(Debug)]
struct AnimatedScalarSlot {
    state: Rc<RefCell<AnimatedScalarState>>,
}

#[derive(Debug)]
struct AnimatedRadioIndicatorSlot {
    state: Rc<RefCell<AnimatedRadioIndicatorState>>,
}

/// Cloneable reference to one animated scalar slot, valid for the bind
/// generation it was created in.
///
/// Watcher closures capture a handle at dispatch time and push new targets
/// through it; once the slot is rebound by a later rebuild, targets applied
/// through stale handles are dropped so an outdated closure cannot fight the
/// current frame's state.
#[derive(Clone, Debug)]
pub struct AnimatedScalarHandle {
    key: AnimationKey,
    state: Rc<RefCell<AnimatedScalarState>>,
    generation: u64,
}

#[derive(Debug)]
struct AnimatedScalarState {
    generation: u64,
    track: AnimationTrack<f32>,
    active_target: Option<f32>,
    last_tick: Instant,
}

#[derive(Debug)]
struct AnimatedRadioIndicatorState {
    selected: bool,
    inner_scale: AnimationTrack<f32>,
    inner_opacity: AnimationTrack<f32>,
    outer_color: AnimationTrack<f32>,
    last_tick: Instant,
}

#[derive(Debug)]
struct RepeatingPhaseSlot {
    started_at: Instant,
    cycle: Duration,
    repeat: bool,
}

impl AnimationKey {
    /// Returns the scalar-slot key for the signal whose value the animation
    /// follows.
    #[must_use]
    pub const fn scalar(identity: SignalIdentity) -> Self {
        Self {
            scope: AnimationKeyScope::Scalar,
            identity: identity.raw() as u64,
        }
    }

    /// Returns a scalar-slot key mixing the signal identity with a
    /// per-instance discriminator, so multiple widgets driven by the same
    /// signal (e.g. each option of a picker) get distinct slots.
    #[must_use]
    pub const fn scalar_with_discriminator(identity: SignalIdentity, discriminator: usize) -> Self {
        Self {
            scope: AnimationKeyScope::Scalar,
            identity: mix_identity(identity.raw(), discriminator),
        }
    }

    /// Returns the radio-indicator-slot key for one selectable option,
    /// mixing the selection signal's identity with the option's
    /// discriminator. Only valid with
    /// [`bind_radio_indicator`](AnimationController::bind_radio_indicator).
    #[must_use]
    pub const fn radio_indicator_with_discriminator(
        identity: SignalIdentity,
        discriminator: usize,
    ) -> Self {
        Self {
            scope: AnimationKeyScope::RadioIndicator,
            identity: mix_identity(identity.raw(), discriminator),
        }
    }

    /// Returns a scalar-slot key for an animation the renderer owns itself
    /// (no backing signal), identified by a renderer-chosen identity such as
    /// a node placement.
    #[must_use]
    pub const fn renderer_local_scalar(identity: usize) -> Self {
        Self {
            scope: AnimationKeyScope::RendererLocalScalar,
            identity: identity as u64,
        }
    }

    /// Returns a renderer-owned scalar key scoped by a stable semantic owner
    /// and a per-owner discriminator.
    #[must_use]
    pub const fn renderer_local_scalar_with_discriminator(
        identity: usize,
        discriminator: usize,
    ) -> Self {
        Self {
            scope: AnimationKeyScope::RendererLocalScalar,
            identity: mix_identity(identity, discriminator),
        }
    }

    /// Returns whether this key was built with
    /// [`renderer_local_scalar`](Self::renderer_local_scalar) rather than
    /// from a signal identity.
    #[must_use]
    pub const fn is_renderer_local_scalar(self) -> bool {
        matches!(self.scope, AnimationKeyScope::RendererLocalScalar)
    }

    /// Returns a repeating-phase-slot key for a renderer-owned free-running
    /// timeline (e.g. an indeterminate progress indicator). Only valid with
    /// [`bind_repeating_phase`](AnimationController::bind_repeating_phase) /
    /// [`bind_timeline_phase`](AnimationController::bind_timeline_phase).
    #[must_use]
    pub const fn renderer_local_repeating(identity: usize) -> Self {
        Self {
            scope: AnimationKeyScope::Repeating,
            identity: RENDERER_LOCAL_KEY_MASK | identity as u64,
        }
    }

    #[cfg(test)]
    const fn test_scalar(identity: usize) -> Self {
        Self {
            scope: AnimationKeyScope::Scalar,
            identity: identity as u64,
        }
    }

    #[cfg(test)]
    const fn test_radio_indicator(identity: usize) -> Self {
        Self {
            scope: AnimationKeyScope::RadioIndicator,
            identity: identity as u64,
        }
    }
}

impl AnimationController {
    /// Resets the bound-this-frame bookkeeping; called at the begin of a
    /// structural rebuild before any slot is bound. Slot contents (in-flight
    /// tracks and their progress) are untouched.
    pub fn begin_rebuild_frame(&mut self) {
        self.active_slots.clear();
        self.active_radio_indicator_slots.clear();
        self.active_repeating_slots.clear();
    }

    /// Finishes a structural rebuild by dropping every slot that was not
    /// rebound during it, unless `retain_inactive_slots` is set.
    ///
    /// Retention is used for partial dispatch passes (e.g. replaying a
    /// retained subtree from cache) where unbound slots merely were not
    /// visited and must keep their in-flight state.
    pub fn finish_rebuild_frame_with_inactive_slot_retention(
        &mut self,
        retain_inactive_slots: bool,
    ) {
        if retain_inactive_slots {
            return;
        }
        self.slots.retain(|key, _| self.active_slots.contains(key));
        self.radio_indicator_slots
            .retain(|key, _| self.active_radio_indicator_slots.contains(key));
        self.repeating_slots
            .retain(|key, _| self.active_repeating_slots.contains(key));
    }

    /// Binds the scalar slot for `key` during dispatch, reconciling it with
    /// the value observed from the view tree, and returns a fresh handle.
    ///
    /// If the observed value disagrees with the slot (and with any in-flight
    /// target), the track snaps to it without animating — the watcher that
    /// owns the handle is responsible for animated transitions. Each bind
    /// advances the slot generation, so handles from earlier frames become
    /// inert. Panics if `key` is not a scalar-scope key.
    /// # Panics
    ///
    /// Panics if `key` was created with a scope that does not match this animation binding.
    pub fn bind_scalar(
        &mut self,
        key: AnimationKey,
        observed_value: f32,
        now: Instant,
    ) -> AnimatedScalarHandle {
        assert!(
            matches!(
                key.scope,
                AnimationKeyScope::Scalar | AnimationKeyScope::RendererLocalScalar
            ),
            "animation scalar key used with mismatched scope"
        );
        self.active_slots.insert(key);
        let slot = self.slots.entry(key).or_insert_with(|| AnimatedScalarSlot {
            state: Rc::new(RefCell::new(AnimatedScalarState::new(observed_value, now))),
        });

        let state = Rc::clone(&slot.state);
        let generation = state.borrow_mut().prepare_generation(observed_value);
        AnimatedScalarHandle {
            key,
            state,
            generation,
        }
    }

    /// Binds the scalar slot for `key` and animates it toward `target` with
    /// `animation`, returning a fresh handle.
    ///
    /// Unlike [`bind_scalar`](Self::bind_scalar), the target itself is the
    /// per-frame input: re-binding with the same target does not restart the
    /// in-flight animation, while a new target starts one from the current
    /// sampled value (no snapping). A slot created on first bind starts at
    /// `target` directly. Panics if `key` is not a scalar-scope key.
    /// # Panics
    ///
    /// Panics if `key` was created with a scope that does not match this animation binding.
    pub fn bind_scalar_target(
        &mut self,
        key: AnimationKey,
        target: f32,
        animation: Animation,
        now: Instant,
    ) -> AnimatedScalarHandle {
        assert!(
            matches!(
                key.scope,
                AnimationKeyScope::Scalar | AnimationKeyScope::RendererLocalScalar
            ),
            "animation scalar target key used with mismatched scope"
        );
        self.active_slots.insert(key);
        let slot = self.slots.entry(key).or_insert_with(|| AnimatedScalarSlot {
            state: Rc::new(RefCell::new(AnimatedScalarState::new(target, now))),
        });

        let state = Rc::clone(&slot.state);
        let generation = state.borrow_mut().prepare_target_generation();
        let handle = AnimatedScalarHandle {
            key,
            state,
            generation,
        };
        handle.apply_target(target, Some(animation), now);
        handle
    }

    /// Binds the radio-indicator slot for `key` and returns the sampled
    /// indicator state for this frame.
    ///
    /// A selection change starts the choreography described by `motion`:
    /// selecting grows the inner dot from zero while fading dot opacity and
    /// outer ring color in; deselecting fades them out without shrinking the
    /// dot. Panics if `key` is not a radio-indicator-scope key.
    /// # Panics
    ///
    /// Panics if `key` was created with a scope that does not match this animation binding.
    pub fn bind_radio_indicator(
        &mut self,
        key: AnimationKey,
        selected: bool,
        motion: &RadioSelectionMotion,
        now: Instant,
    ) -> RadioIndicatorState {
        assert!(
            key.scope == AnimationKeyScope::RadioIndicator,
            "animation radio indicator key used with mismatched scope"
        );
        self.active_radio_indicator_slots.insert(key);
        let slot =
            self.radio_indicator_slots
                .entry(key)
                .or_insert_with(|| AnimatedRadioIndicatorSlot {
                    state: Rc::new(RefCell::new(AnimatedRadioIndicatorState::new(
                        selected, now,
                    ))),
                });

        slot.state.borrow_mut().prepare(selected, motion, now)
    }

    /// Advances every slot's track to `now` and returns whether any
    /// animation is still running, in which case the renderer must schedule
    /// another redraw frame. Called once per frame with the frame clock.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut has_active = false;
        for slot in self.slots.values() {
            if slot.state.borrow_mut().advance(now) {
                has_active = true;
            }
        }
        for slot in self.radio_indicator_slots.values() {
            if slot.state.borrow_mut().advance(now) {
                has_active = true;
            }
        }
        has_active
            || self.repeating_slots.values().any(|slot| {
                slot.repeat || now.saturating_duration_since(slot.started_at) < slot.cycle
            })
    }

    /// Returns whether any slot is still animating at `now`, without
    /// advancing any track (used to decide whether the next frame must be
    /// scheduled).
    #[must_use]
    pub fn has_active(&self, now: Instant) -> bool {
        self.slots
            .values()
            .any(|slot| slot.state.borrow().is_active())
            || self
                .radio_indicator_slots
                .values()
                .any(|slot| slot.state.borrow().is_active())
            || self.repeating_slots.values().any(|slot| {
                slot.repeat || now.saturating_duration_since(slot.started_at) < slot.cycle
            })
    }

    /// Returns the keys of all scalar slots whose animation is still in
    /// flight, letting the renderer patch exactly the nodes that depend on
    /// them.
    #[must_use]
    pub fn active_scalar_keys(&self) -> BTreeSet<AnimationKey> {
        self.slots
            .iter()
            .filter_map(|(key, slot)| slot.state.borrow().is_active().then_some(*key))
            .collect()
    }

    /// Returns whether any radio-indicator choreography is still in flight.
    #[must_use]
    pub fn has_active_radio_indicator(&self) -> bool {
        self.radio_indicator_slots
            .values()
            .any(|slot| slot.state.borrow().is_active())
    }

    /// Returns whether any repeating/timeline phase slot still requires
    /// frames at `now`: repeating slots run forever, one-shot timeline slots
    /// only until their cycle completes.
    #[must_use]
    pub fn has_active_repeating(&self, now: Instant) -> bool {
        self.repeating_slots
            .values()
            .any(|slot| slot.repeat || now.saturating_duration_since(slot.started_at) < slot.cycle)
    }

    /// Binds the repeating slot for `key` and returns the phase within the
    /// current cycle (`elapsed % cycle`), for free-running looping motion
    /// such as indeterminate progress indicators.
    pub fn bind_repeating_phase(
        &mut self,
        key: AnimationKey,
        cycle: Duration,
        now: Instant,
    ) -> Duration {
        let elapsed = self.bind_timeline_phase(key, cycle, true, now);
        Duration::from_secs_f64(elapsed.as_secs_f64() % cycle.as_secs_f64())
    }

    /// Binds the timeline slot for `key` and returns the elapsed time since
    /// the slot's start, capped at `cycle` when `repeat` is false.
    ///
    /// The slot's start instant persists across rebuilds; changing `cycle` or
    /// `repeat` restarts the timeline at `now`. Panics if `cycle` is zero or
    /// `key` is not a repeating-scope key.
    /// # Panics
    ///
    /// Panics if `key` was created with a scope that does not match this animation binding.
    pub fn bind_timeline_phase(
        &mut self,
        key: AnimationKey,
        cycle: Duration,
        repeat: bool,
        now: Instant,
    ) -> Duration {
        assert!(
            !cycle.is_zero(),
            "animation repeating phase cycle must be non-zero"
        );
        assert!(
            key.scope == AnimationKeyScope::Repeating,
            "animation repeating key used with mismatched scope"
        );
        self.active_repeating_slots.insert(key);
        let slot = self
            .repeating_slots
            .entry(key)
            .or_insert(RepeatingPhaseSlot {
                started_at: now,
                cycle,
                repeat,
            });
        if slot.cycle != cycle || slot.repeat != repeat {
            slot.started_at = now;
            slot.cycle = cycle;
            slot.repeat = repeat;
        }
        let elapsed = now.saturating_duration_since(slot.started_at);
        if repeat { elapsed } else { elapsed.min(cycle) }
    }
}

impl AnimatedScalarHandle {
    /// Returns the key of the slot this handle is bound to.
    #[must_use]
    pub const fn key(&self) -> AnimationKey {
        self.key
    }

    /// Returns whether the slot's animation is still in flight.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.borrow().is_active()
    }

    /// Advances the slot's track to `now` and returns the current scalar
    /// value; the renderer samples with the frame clock when drawing.
    #[must_use]
    pub fn sample(&self, now: Instant) -> f32 {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        state.current()
    }

    /// Applies a signal watcher update: the new value becomes the target and
    /// the `Animation` carried in the update's metadata (if any) drives the
    /// transition.
    pub fn apply_update_from_context(&self, update: Context<f32>, now: Instant) {
        let metadata = update.metadata().try_get::<Animation>();
        self.apply_target(update.into_value(), metadata, now);
    }

    /// Steers the slot toward `target`, animating with `animation` or
    /// snapping immediately when it is `None`.
    ///
    /// Dropped when the slot was rebound by a later rebuild (stale handle
    /// generation), and ignored when the slot is already at — or already
    /// animating toward — the same target, so repeated identical updates do
    /// not restart the animation.
    pub fn apply_target(&self, target: f32, animation: Option<Animation>, now: Instant) {
        let mut state = self.state.borrow_mut();
        if state.generation != self.generation {
            return;
        }
        state.apply_target(target, animation, now);
    }
}

impl AnimatedScalarState {
    const fn new(initial: f32, now: Instant) -> Self {
        Self {
            generation: 1,
            track: AnimationTrack::new(initial),
            active_target: None,
            last_tick: now,
        }
    }

    fn prepare_generation(&mut self, observed_value: f32) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("animation controller generation overflow");
        self.reconcile_observed(observed_value);
        self.generation
    }

    const fn prepare_target_generation(&mut self) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("animation controller generation overflow");
        self.generation
    }

    fn reconcile_observed(&mut self, observed_value: f32) {
        if let Some(target) = self.active_target {
            if !approx_eq(target, observed_value) {
                self.track.set_target(observed_value, None);
                self.active_target = None;
            }
            return;
        }

        if !approx_eq(self.current(), observed_value) {
            self.track.set_target(observed_value, None);
        }
    }

    fn apply_target(&mut self, target: f32, animation: Option<Animation>, now: Instant) {
        let _ = self.advance(now);
        if approx_eq(self.current(), target) {
            self.track.set_target(target, None);
            self.active_target = None;
            return;
        }
        if self
            .active_target
            .is_some_and(|active| approx_eq(active, target))
        {
            return;
        }

        if let Some(animation) = animation {
            self.track.set_target(target, Some(animation));
            self.active_target = Some(target);
        } else {
            self.track.set_target(target, None);
            self.active_target = None;
        }
    }

    fn advance(&mut self, now: Instant) -> bool {
        let delta = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        let active = self.track.advance(delta);
        if !active {
            self.active_target = None;
        }
        active
    }

    const fn is_active(&self) -> bool {
        self.track.is_active()
    }

    fn current(&self) -> f32 {
        self.track.value()
    }
}

impl AnimatedRadioIndicatorState {
    const fn new(selected: bool, now: Instant) -> Self {
        let selected_progress = if selected { 1.0 } else { 0.0 };
        Self {
            selected,
            inner_scale: AnimationTrack::new(1.0),
            inner_opacity: AnimationTrack::new(selected_progress),
            outer_color: AnimationTrack::new(selected_progress),
            last_tick: now,
        }
    }

    fn prepare(
        &mut self,
        selected: bool,
        motion: &RadioSelectionMotion,
        now: Instant,
    ) -> RadioIndicatorState {
        let _ = self.advance(now);
        if self.selected != selected {
            if selected {
                self.inner_scale.set_target(0.0, None);
                self.inner_scale
                    .set_target(1.0, Some(motion.inner_grow.clone()));
            } else {
                self.inner_scale.set_target(1.0, None);
            }
            let selected_progress = if selected { 1.0 } else { 0.0 };
            self.inner_opacity
                .set_target(selected_progress, Some(motion.inner_opacity.clone()));
            self.outer_color
                .set_target(selected_progress, Some(motion.outer_color.clone()));
            self.selected = selected;
        }
        self.sample()
    }

    fn advance(&mut self, now: Instant) -> bool {
        let delta = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        let scale_active = self.inner_scale.advance(delta);
        let opacity_active = self.inner_opacity.advance(delta);
        let color_active = self.outer_color.advance(delta);
        scale_active || opacity_active || color_active
    }

    const fn is_active(&self) -> bool {
        self.inner_scale.is_active()
            || self.inner_opacity.is_active()
            || self.outer_color.is_active()
    }

    fn sample(&self) -> RadioIndicatorState {
        RadioIndicatorState {
            selected: self.selected,
            outer_selected_progress: self.outer_color.value().clamp(0.0, 1.0),
            inner_scale: self.inner_scale.value().clamp(0.0, 1.0),
            inner_opacity: self.inner_opacity.value().clamp(0.0, 1.0),
        }
    }
}

const fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= VALUE_EPSILON
}

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::time::Instant;

    use crate::widget::RadioSelectionMotion;
    use waterui::animation::Animation;

    use super::{AnimationController, AnimationKey};

    const fn scalar_key(identity: usize) -> AnimationKey {
        AnimationKey::test_scalar(identity)
    }

    const fn radio_key(identity: usize) -> AnimationKey {
        AnimationKey::test_radio_indicator(identity)
    }

    #[test]
    fn scalar_animation_advances_and_stops() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        controller.begin_rebuild_frame();
        let handle = controller.bind_scalar(scalar_key(1), 0.0, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        handle.apply_target(
            1.0,
            Some(Animation::ease_in_out(Duration::from_millis(120))),
            start,
        );

        let mid_value = handle.sample(start + Duration::from_millis(60));
        assert!(mid_value > 0.0 && mid_value < 1.0);
        assert!(controller.tick(start + Duration::from_millis(60)));

        let final_value = handle.sample(start + Duration::from_millis(200));
        assert!((final_value - 1.0).abs() < 0.0001);
        assert!(!controller.tick(start + Duration::from_millis(200)));
    }

    #[test]
    fn stale_generation_update_is_ignored() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        controller.begin_rebuild_frame();
        let stale = controller.bind_scalar(scalar_key(1), 0.0, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        controller.begin_rebuild_frame();
        let current = controller.bind_scalar(scalar_key(1), 0.0, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        stale.apply_target(1.0, None, start);
        assert!((current.sample(start) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn scalar_target_binding_animates_without_snapping() {
        let mut controller = AnimationController::default();
        controller.begin_rebuild_frame();
        let first = controller.bind_scalar_target(
            scalar_key(1),
            0.0,
            Animation::ease_in_out(Duration::from_millis(1)),
            Instant::now(),
        );
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        assert!((first.sample(Instant::now()) - 0.0).abs() < 0.0001);

        let start = Instant::now();
        controller.begin_rebuild_frame();
        let second = controller.bind_scalar_target(
            scalar_key(1),
            1.0,
            Animation::ease_in_out(Duration::from_millis(120)),
            start,
        );
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        let mid = second.sample(start + Duration::from_millis(60));
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    fn repeated_same_scalar_target_does_not_restart_animation() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        controller.begin_rebuild_frame();
        let first = controller.bind_scalar_target(
            scalar_key(1),
            0.0,
            Animation::ease_in_out(Duration::from_millis(120)),
            start,
        );
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        assert!((first.sample(start) - 0.0).abs() < 0.0001);

        controller.begin_rebuild_frame();
        let second = controller.bind_scalar_target(
            scalar_key(1),
            1.0,
            Animation::ease_in_out(Duration::from_millis(120)),
            start,
        );
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        assert!(second.sample(start + Duration::from_millis(60)) > 0.0);

        controller.begin_rebuild_frame();
        let third = controller.bind_scalar_target(
            scalar_key(1),
            1.0,
            Animation::ease_in_out(Duration::from_millis(120)),
            start + Duration::from_millis(60),
        );
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        let final_value = third.sample(start + Duration::from_millis(200));
        assert!((final_value - 1.0).abs() < 0.0001);
        assert!(!controller.tick(start + Duration::from_millis(200)));
    }

    #[test]
    fn retained_inactive_scalar_slot_keeps_in_flight_target() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        let animation = Animation::ease_in_out(Duration::from_millis(120));

        controller.begin_rebuild_frame();
        let handle = controller.bind_scalar(scalar_key(1), 0.0, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        handle.apply_target(1.0, Some(animation), start);

        controller.begin_rebuild_frame();
        controller.finish_rebuild_frame_with_inactive_slot_retention(true);

        controller.begin_rebuild_frame();
        let rebound = controller.bind_scalar(scalar_key(1), 1.0, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        let mid = rebound.sample(start + Duration::from_millis(60));
        assert!(
            mid > 0.0 && mid < 1.0,
            "retained cache replay must not make an in-flight scalar snap to its observed target"
        );
    }

    #[test]
    fn scalar_target_keys_survive_reordered_render_pass() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        let animation = Animation::ease_in_out(Duration::from_millis(120));

        controller.begin_rebuild_frame();
        let first_a = controller.bind_scalar_target(scalar_key(1), 0.0, animation.clone(), start);
        let first_b = controller.bind_scalar_target(scalar_key(2), 0.0, animation.clone(), start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        assert!((first_a.sample(start) - 0.0).abs() < 0.0001);
        assert!((first_b.sample(start) - 0.0).abs() < 0.0001);

        controller.begin_rebuild_frame();
        let animating_a =
            controller.bind_scalar_target(scalar_key(1), 1.0, animation.clone(), start);
        let steady_b = controller.bind_scalar_target(scalar_key(2), 0.0, animation.clone(), start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        assert!(animating_a.sample(start + Duration::from_millis(60)) > 0.0);
        assert!((steady_b.sample(start + Duration::from_millis(60)) - 0.0).abs() < 0.0001);

        let reordered_at = start + Duration::from_millis(60);
        controller.begin_rebuild_frame();
        let reordered_b =
            controller.bind_scalar_target(scalar_key(2), 0.0, animation.clone(), reordered_at);
        let reordered_a =
            controller.bind_scalar_target(scalar_key(1), 1.0, animation, reordered_at);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert!(
            (reordered_b.sample(reordered_at) - 0.0).abs() < 0.0001,
            "stable B must not inherit A's in-flight animation when render order changes"
        );
        let reordered_a_value = reordered_a.sample(reordered_at);
        assert!(
            reordered_a_value > 0.0 && reordered_a_value < 1.0,
            "stable A must keep its in-flight animation when render order changes"
        );
    }

    #[test]
    fn radio_indicator_selection_grows_dot_and_fades_color() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        let motion = RadioSelectionMotion {
            inner_grow: Animation::linear(Duration::from_millis(300)),
            inner_opacity: Animation::linear(Duration::from_millis(50)),
            outer_color: Animation::linear(Duration::from_millis(50)),
        };

        controller.begin_rebuild_frame();
        let initial = controller.bind_radio_indicator(radio_key(1), false, &motion, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert!(!initial.selected);
        assert_eq!(initial.inner_scale, 1.0);
        assert_eq!(initial.inner_opacity, 0.0);
        assert_eq!(initial.outer_selected_progress, 0.0);

        controller.begin_rebuild_frame();
        let changed = controller.bind_radio_indicator(radio_key(1), true, &motion, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert!(changed.selected);
        assert_eq!(changed.inner_scale, 0.0);
        assert_eq!(changed.inner_opacity, 0.0);
        assert_eq!(changed.outer_selected_progress, 0.0);

        let mid = start + Duration::from_millis(25);
        controller.begin_rebuild_frame();
        let mid_state = controller.bind_radio_indicator(radio_key(1), true, &motion, mid);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert!(mid_state.inner_scale > 0.0 && mid_state.inner_scale < 1.0);
        assert!(mid_state.inner_opacity > 0.0 && mid_state.inner_opacity < 1.0);
        assert!(mid_state.outer_selected_progress > 0.0 && mid_state.outer_selected_progress < 1.0);
    }

    #[test]
    fn radio_indicator_deselection_fades_dot_without_shrinking() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        let motion = RadioSelectionMotion {
            inner_grow: Animation::linear(Duration::from_millis(300)),
            inner_opacity: Animation::linear(Duration::from_millis(50)),
            outer_color: Animation::linear(Duration::from_millis(50)),
        };

        controller.begin_rebuild_frame();
        let initial = controller.bind_radio_indicator(radio_key(1), true, &motion, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert!(initial.selected);
        assert_eq!(initial.inner_scale, 1.0);
        assert_eq!(initial.inner_opacity, 1.0);

        controller.begin_rebuild_frame();
        let changed = controller.bind_radio_indicator(radio_key(1), false, &motion, start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert!(!changed.selected);
        assert_eq!(changed.inner_scale, 1.0);
        assert_eq!(changed.inner_opacity, 1.0);

        let mid = start + Duration::from_millis(25);
        controller.begin_rebuild_frame();
        let mid_state = controller.bind_radio_indicator(radio_key(1), false, &motion, mid);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        assert_eq!(mid_state.inner_scale, 1.0);
        assert!(mid_state.inner_opacity > 0.0 && mid_state.inner_opacity < 1.0);
    }
}
