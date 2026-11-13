//! Animated scalar sampling, animation key registry, and renderer-local
//! motion state.
//!
//! [`AnimationController`] owns the per-frame registry of animation slots,
//! keyed by [`AnimationKey`] (a renderer-chosen owner identity, optionally
//! mixed with a per-instance discriminator). Slots are
//! rebound on every structural rebuild and survive across frames so
//! in-flight animations keep their progress; unbound slots are dropped when
//! the rebuild finishes. The controller samples one slot kind: animated
//! scalars driven by `AnimationTrack<f32>`. A second
//! kind, a wrapper's engine-run motion (water-rs/waterui#2203), is bound and
//! scoped like the first but never sampled: the engine samples it, and the
//! controller only compares the frame clock with the instant its track ends.

#[cfg(any(test, all(target_os = "android", hydrolysis_gpu_painter)))]
use cherenkov_record::{RefreshRange, tree::RATE_FAST};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::renderer::mount::motion::MotionStatus;
use waterui_core::animation::{Animation, AnimationTrack};
use waterui_core::contract_violation;

use crate::time::Instant;

const VALUE_EPSILON: f32 = 0.000_01;

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
    /// Engine-run motions, by the scalar key of the wrapper slot they fill.
    motions: BTreeMap<AnimationKey, Rc<MotionStatus>>,
    active_slots: BTreeSet<AnimationKey>,
    active_motions: BTreeSet<AnimationKey>,
    /// Scopes pushed by the backend while a subtree binds its slots — the
    /// anchored overlay's exit waits on the slots its content owns.
    scope_stack: Vec<u64>,
    scoped_keys: BTreeMap<u64, BTreeSet<AnimationKey>>,
}

/// Stable identity of one animation slot across structural rebuilds.
///
/// Built from a renderer-chosen owner identity (such as a view's pinned
/// allocation), optionally mixed with a per-instance discriminator so
/// several slots owned by one view get distinct keys. Keys of different
/// scopes never collide.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AnimationKey {
    scope: AnimationKeyScope,
    identity: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum AnimationKeyScope {
    RendererLocalScalar,
}

#[derive(Debug)]
struct AnimatedScalarSlot {
    state: Rc<RefCell<AnimatedScalarState>>,
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

/// Whether a repeat-forever animation counts as running work in a
/// quiescence query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerpetualAnimation {
    /// It counts: the query asks whether anything is still animating.
    Counts,
    /// It does not: the query asks whether anything will finish.
    Ignored,
}

impl AnimationKey {
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

    #[cfg(test)]
    const fn test_scalar(identity: usize) -> Self {
        Self {
            scope: AnimationKeyScope::RendererLocalScalar,
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
        self.active_motions.clear();
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
        self.motions
            .retain(|key, _| self.active_motions.contains(key));
    }

    /// Binds the engine-run motion filling `key` during dispatch: until
    /// the rebuild that no longer binds it, settle queries and `key`'s
    /// animation scopes see its track.
    ///
    /// A `key` that is not a scalar-scope key is a contract violation by the
    /// caller: it is reported and binds the motion all the same, since
    /// motions are keyed by the whole key.
    pub fn bind_motion(&mut self, key: AnimationKey, status: Rc<MotionStatus>) {
        if key.scope != AnimationKeyScope::RendererLocalScalar {
            contract_violation!(
                "animation motion key used with mismatched scope; the motion is bound anyway"
            );
        }
        self.active_motions.insert(key);
        self.note_key(key);
        self.motions.insert(key, status);
    }

    /// Whether an engine-run motion's track still runs at `now`.
    #[must_use]
    pub fn motions_active(&self, now: Instant) -> bool {
        self.motions.values().any(|status| status.is_running(now))
    }

    /// Fires the settle re-record of every motion whose track ended by
    /// `now`; `true` when one fired. A comparison with each track's end,
    /// never an evaluation of the track.
    pub fn settle_motions(&self, now: Instant) -> bool {
        self.motions
            .values()
            .fold(false, |fired, status| status.settle(now) | fired)
    }

    /// Binds the scalar slot for `key` and animates it toward `target` with
    /// `animation`, returning a fresh handle.
    ///
    /// The target itself is the per-frame input: re-binding with the same
    /// target does not restart the
    /// in-flight animation, while a new target starts one from the current
    /// sampled value (no snapping). A slot created on first bind starts at
    /// `target` directly.
    ///
    /// A `key` that is not a scalar-scope key is a contract violation by the
    /// caller: it is reported and binds the slot all the same.
    pub fn bind_scalar_target(
        &mut self,
        key: AnimationKey,
        target: f32,
        animation: Animation,
        now: Instant,
    ) -> AnimatedScalarHandle {
        if key.scope != AnimationKeyScope::RendererLocalScalar {
            contract_violation!(
                "animation scalar target key used with mismatched scope; the slot is bound anyway"
            );
        }
        self.active_slots.insert(key);
        self.note_key(key);
        let slot = self.slots.entry(key).or_insert_with(|| AnimatedScalarSlot {
            state: Rc::new(RefCell::new(AnimatedScalarState::new(target, now))),
        });

        let state = Rc::clone(&slot.state);
        let generation = state.borrow_mut().prepare_target_generation();
        let handle = AnimatedScalarHandle { state, generation };
        handle.apply_target(target, Some(animation), now);
        handle
    }

    /// Advances every slot's track to `now` and returns whether any
    /// animation is still running, in which case the renderer must schedule
    /// another redraw frame. Called once per frame with the frame clock.
    pub fn tick(&self, now: Instant) -> bool {
        let mut has_active = false;
        for slot in self.slots.values() {
            if slot.state.borrow_mut().advance(now) {
                has_active = true;
            }
        }
        has_active
    }

    /// Returns whether any slot is still animating, without advancing any
    /// track (used to decide whether the next frame must be scheduled). Every
    /// slot runs to a finish, so none is repeat-forever.
    #[must_use]
    pub fn has_active(&self) -> bool {
        self.slots
            .values()
            .any(|slot| slot.state.borrow().is_active())
    }

    /// The range needed by currently running content, independent of how
    /// often the host happens to sample it: widget scalars move content at
    /// the fast rate. Engine-run motions are
    /// not counted here; the engine reports the rate of every track it
    /// samples.
    #[cfg(any(test, all(target_os = "android", hydrolysis_gpu_painter)))]
    #[must_use]
    pub fn refresh_range(&self) -> Option<RefreshRange> {
        self.has_active().then_some(RATE_FAST)
    }

    /// Marks the start of a subtree whose animation slots belong to `scope`
    /// until the matching [`end_animation_scope`](Self::end_animation_scope).
    ///
    /// A backend that keeps content alive past dismissal — the anchored
    /// overlay's exit — waits on that subtree's running animations through
    /// [`scope_is_active`](Self::scope_is_active) instead of guessing a delay.
    pub fn begin_animation_scope(&mut self, scope: u64) {
        self.scope_stack.push(scope);
    }

    /// Pops the scope opened by [`begin_animation_scope`](Self::begin_animation_scope).
    ///
    /// Ending with no scope open is a contract violation by the caller: it
    /// is reported and nothing is popped.
    pub fn end_animation_scope(&mut self) {
        if self.scope_stack.pop().is_none() {
            contract_violation!("end_animation_scope without a matching begin; no scope is popped");
        }
    }

    /// Attributes `key` to every open scope; called by each `bind_*`.
    fn note_key(&mut self, key: AnimationKey) {
        for &scope in &self.scope_stack {
            self.scoped_keys.entry(scope).or_default().insert(key);
        }
    }

    /// Whether any slot attributed to `scope` is still animating at `now`.
    #[must_use]
    pub fn scope_is_active(&self, scope: u64, now: Instant) -> bool {
        let Some(keys) = self.scoped_keys.get(&scope) else {
            return false;
        };
        keys.iter().any(|key| self.key_is_active(*key, now))
    }

    /// Re-records the owner of every motion attributed to `scope` that runs
    /// at `now` once its track is done: the record that found `scope`
    /// active decides again when its motions end, though nothing samples
    /// them in between.
    pub fn arm_scope_settle(&self, scope: u64, now: Instant) {
        let Some(keys) = self.scoped_keys.get(&scope) else {
            return;
        };
        for status in keys.iter().filter_map(|key| self.motions.get(key)) {
            status.arm_settle(now);
        }
    }

    /// Drops `scope`'s attributions; call when the owning subtree is gone so
    /// its keys stop accumulating here.
    pub fn drop_animation_scope(&mut self, scope: u64) {
        self.scoped_keys.remove(&scope);
    }

    /// Whether `key`'s slot is still animating at `now`, across every scope.
    fn key_is_active(&self, key: AnimationKey, now: Instant) -> bool {
        match key.scope {
            AnimationKeyScope::RendererLocalScalar => {
                self.slots
                    .get(&key)
                    .is_some_and(|slot| slot.state.borrow().is_active())
                    || self
                        .motions
                        .get(&key)
                        .is_some_and(|status| status.is_running(now))
            }
        }
    }
}

impl AnimatedScalarHandle {
    /// Advances the slot's track to `now` and returns the current scalar
    /// value; the renderer samples with the frame clock when drawing.
    #[must_use]
    pub fn sample(&self, now: Instant) -> f32 {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        state.current()
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

    const fn prepare_target_generation(&mut self) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("animation controller generation overflow");
        self.generation
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

const fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= VALUE_EPSILON
}

#[cfg(test)]
mod tests {
    use cherenkov_record::tree::RATE_FAST;
    use core::time::Duration;
    use std::time::Instant;

    use waterui::animation::Animation;

    use super::{AnimationController, AnimationKey};

    const fn scalar_key(identity: usize) -> AnimationKey {
        AnimationKey::test_scalar(identity)
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "end_animation_scope without a matching begin")
    )]
    fn ending_an_unopened_scope_is_reported_and_pops_nothing() {
        let mut controller = AnimationController::default();
        controller.end_animation_scope();
        controller.begin_animation_scope(1);
        controller.end_animation_scope();
    }

    #[test]
    fn scalar_demand_is_fast_until_the_track_settles() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        let animation = Animation::linear(Duration::from_millis(100));
        let handle = controller.bind_scalar_target(scalar_key(1), 0.0, animation.clone(), start);
        assert_eq!(controller.refresh_range(), None);
        handle.apply_target(1.0, Some(animation), start);
        assert_eq!(controller.refresh_range(), Some(RATE_FAST));
        let settled = start + Duration::from_millis(200);
        controller.tick(settled);
        assert_eq!(controller.refresh_range(), None);
        assert_eq!(handle.sample(settled), 1.0);
    }

    #[test]
    fn stale_generation_update_is_ignored() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        let animation = Animation::linear(Duration::from_millis(100));
        controller.begin_rebuild_frame();
        let stale = controller.bind_scalar_target(scalar_key(1), 0.0, animation.clone(), start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);

        controller.begin_rebuild_frame();
        let current = controller.bind_scalar_target(scalar_key(1), 0.0, animation, start);
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
        let handle = controller.bind_scalar_target(scalar_key(1), 0.0, animation.clone(), start);
        controller.finish_rebuild_frame_with_inactive_slot_retention(false);
        handle.apply_target(1.0, Some(animation.clone()), start);

        controller.begin_rebuild_frame();
        controller.finish_rebuild_frame_with_inactive_slot_retention(true);

        controller.begin_rebuild_frame();
        let rebound = controller.bind_scalar_target(scalar_key(1), 1.0, animation, start);
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
}
