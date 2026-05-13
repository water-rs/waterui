use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;

use nami::watcher::Context;
use waterui::animation::{Animation, AnimationTrack};

use crate::time::Instant;

const VALUE_EPSILON: f32 = 0.000_01;

#[derive(Debug, Default)]
pub struct AnimationController {
    slots: Vec<AnimatedScalarSlot>,
    repeating_slots: Vec<RepeatingPhaseSlot>,
    cursor: usize,
    repeating_cursor: usize,
}

#[derive(Debug)]
struct AnimatedScalarSlot {
    state: Rc<RefCell<AnimatedScalarState>>,
}

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

#[derive(Debug)]
struct RepeatingPhaseSlot {
    started_at: Instant,
    cycle: Duration,
}

impl AnimationController {
    pub fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
        self.repeating_cursor = 0;
    }

    pub fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
        self.repeating_slots.truncate(self.repeating_cursor);
    }

    pub fn bind_scalar(&mut self, observed_value: f32, now: Instant) -> AnimatedScalarHandle {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("animation controller scalar cursor overflow");

        if index == self.slots.len() {
            self.slots.push(AnimatedScalarSlot {
                state: Rc::new(RefCell::new(AnimatedScalarState::new(observed_value, now))),
            });
        }

        let state = Rc::clone(&self.slots[index].state);
        let generation = state.borrow_mut().prepare_generation(observed_value);
        AnimatedScalarHandle { state, generation }
    }

    pub fn bind_scalar_target(
        &mut self,
        target: f32,
        animation: Animation,
        now: Instant,
    ) -> AnimatedScalarHandle {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("animation controller scalar cursor overflow");

        if index == self.slots.len() {
            self.slots.push(AnimatedScalarSlot {
                state: Rc::new(RefCell::new(AnimatedScalarState::new(target, now))),
            });
        }

        let state = Rc::clone(&self.slots[index].state);
        let generation = state.borrow_mut().prepare_target_generation();
        let handle = AnimatedScalarHandle { state, generation };
        handle.apply_target(target, Some(animation), now);
        handle
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let mut has_active = false;
        for slot in &self.slots {
            if slot.state.borrow_mut().advance(now) {
                has_active = true;
            }
        }
        has_active || !self.repeating_slots.is_empty()
    }

    pub fn bind_repeating_phase(&mut self, cycle: Duration, now: Instant) -> Duration {
        assert!(
            !cycle.is_zero(),
            "animation repeating phase cycle must be non-zero"
        );
        let index = self.repeating_cursor;
        self.repeating_cursor = self
            .repeating_cursor
            .checked_add(1)
            .expect("animation controller repeating cursor overflow");
        if index == self.repeating_slots.len() {
            self.repeating_slots.push(RepeatingPhaseSlot {
                started_at: now,
                cycle,
            });
        }
        let slot = &mut self.repeating_slots[index];
        if slot.cycle != cycle {
            slot.started_at = now;
            slot.cycle = cycle;
        }
        let elapsed = now.saturating_duration_since(slot.started_at);
        Duration::from_secs_f64(elapsed.as_secs_f64() % cycle.as_secs_f64())
    }
}

impl AnimatedScalarHandle {
    pub fn sample(&self, now: Instant) -> f32 {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        state.current()
    }

    pub fn apply_update_from_context(&self, update: Context<f32>, now: Instant) {
        let metadata = update.metadata().try_get::<Animation>();
        self.apply_target(update.into_value(), metadata, now);
    }

    pub fn apply_target(&self, target: f32, animation: Option<Animation>, now: Instant) {
        let mut state = self.state.borrow_mut();
        if state.generation != self.generation {
            return;
        }
        state.apply_target(target, animation, now);
    }
}

impl AnimatedScalarState {
    fn new(initial: f32, now: Instant) -> Self {
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

    fn prepare_target_generation(&mut self) -> u64 {
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

        match animation {
            Some(animation) => {
                self.track.set_target(target, Some(animation));
                self.active_target = Some(target);
            }
            None => {
                self.track.set_target(target, None);
                self.active_target = None;
            }
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

    fn current(&self) -> f32 {
        self.track.value()
    }
}

const fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= VALUE_EPSILON
}

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::time::Instant;

    use waterui::animation::Animation;

    use super::AnimationController;

    #[test]
    fn scalar_animation_advances_and_stops() {
        let mut controller = AnimationController::default();
        let start = Instant::now();
        controller.begin_rebuild_frame();
        let handle = controller.bind_scalar(0.0, start);
        controller.finish_rebuild_frame();

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
        let stale = controller.bind_scalar(0.0, start);
        controller.finish_rebuild_frame();

        controller.begin_rebuild_frame();
        let current = controller.bind_scalar(0.0, start);
        controller.finish_rebuild_frame();

        stale.apply_target(1.0, None, start);
        assert!((current.sample(start) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn scalar_target_binding_animates_without_snapping() {
        let mut controller = AnimationController::default();
        controller.begin_rebuild_frame();
        let first = controller.bind_scalar_target(
            0.0,
            Animation::ease_in_out(Duration::from_millis(1)),
            Instant::now(),
        );
        controller.finish_rebuild_frame();
        assert!((first.sample(Instant::now()) - 0.0).abs() < 0.0001);

        let start = Instant::now();
        controller.begin_rebuild_frame();
        let second = controller.bind_scalar_target(
            1.0,
            Animation::ease_in_out(Duration::from_millis(120)),
            start,
        );
        controller.finish_rebuild_frame();

        let mid = second.sample(start + Duration::from_millis(60));
        assert!(mid > 0.0 && mid < 1.0);
    }
}
