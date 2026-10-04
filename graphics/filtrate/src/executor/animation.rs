//! Deterministic parameter animation: watcher installation, animation
//! events, and per-parameter track state.
//!
//! [`ParamAnimator`] is the executor's reactive-parameter driver: watchers
//! feed change events into a channel, each render drains the channel into
//! per-parameter [`AnimationTrack`]s, and the sampled values fill the
//! passes' uniform blocks for that frame.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{
    OnceLock,
    mpsc::{self, Receiver, Sender},
};

use filtrate_core::{
    AnimatedTarget, AnimationTrack, FilterParam, Interpolator, SignalVisitor, WatchGuard,
};

use crate::effect::EffectRedrawCallback;

pub const PARAM_EPSILON: f32 = 0.000_01;

#[derive(Debug)]
pub struct ParamTrackState {
    pub track: AnimationTrack,
    pub animated_target: Option<f32>,
}

/// Shared animation state that can be updated from watcher callbacks.
#[derive(Debug)]
pub struct SharedAnimationState {
    /// Animation timeline for each parameter index.
    pub tracks: Vec<ParamTrackState>,
    /// Current values for each parameter (either animated or direct).
    pub current_values: Vec<f32>,
    /// Whether any animation is active.
    pub has_active_animation: bool,
}

pub const fn approx_param_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= PARAM_EPSILON
}

pub struct ParamAnimationEvent {
    pub param_index: usize,
    pub target_value: f32,
    pub interpolator: Option<Box<dyn Interpolator>>,
}

impl core::fmt::Debug for ParamAnimationEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParamAnimationEvent")
            .field("param_index", &self.param_index)
            .field("target_value", &self.target_value)
            .field("animated", &self.interpolator.is_some())
            .finish()
    }
}

// ============================================================================
// The event channel's sending half and the watcher installer.
// ============================================================================

/// The sending half of a [`ParamAnimator`]'s event channel.
///
/// It is `Send + Sync` and owns nothing tied to a reactive frontend, so an
/// effect hands it to watcher callbacks while the frontend keeps the
/// (commonly `!Send`) subscription guards on its own side.
#[derive(Clone)]
pub struct ParamSender {
    sender: Sender<ParamAnimationEvent>,
    /// Host wake callback shared with every parameter watcher.
    redraw_callback: Arc<OnceLock<EffectRedrawCallback>>,
    /// Set by watcher callbacks the instant an event is queued, cleared when
    /// events are consumed — lets `redraw_hint` see changes that arrived
    /// between frames without draining the channel.
    events_pending: Arc<AtomicBool>,
}

impl ParamSender {
    /// Queues a new target for parameter `param_index` and wakes the host.
    ///
    /// A watcher may outlive its animator: an effect hands the guard to its
    /// caller, and the host drops the effect on its own schedule. A change
    /// reported after the animator is gone has nothing left to drive, so it
    /// is discarded and the host is not woken.
    fn send(&self, param_index: usize, target: AnimatedTarget) {
        let event = ParamAnimationEvent {
            param_index,
            target_value: target.value,
            interpolator: target.interpolator,
        };
        if self.sender.send(event).is_err() {
            return;
        }
        self.events_pending.store(true, Ordering::Release);
        if let Some(callback) = self.redraw_callback.get() {
            callback();
        }
    }

    /// Subscribes `param` so every change it reports reaches parameter
    /// `param_index`. The returned guard keeps the subscription alive.
    pub fn watch<P: FilterParam + ?Sized>(&self, param_index: usize, param: &P) -> WatchGuard {
        let sender = self.clone();
        param.watch_animated(Box::new(move |target| sender.send(param_index, target)))
    }
}

/// Installs one watcher per visited parameter, collecting their guards.
pub struct WatcherInstaller {
    sender: ParamSender,
    guards: Vec<WatchGuard>,
}

impl SignalVisitor for WatcherInstaller {
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P) {
        self.guards.push(self.sender.watch(param_index, param));
    }
}

/// The reactive-parameter driver: owns the event channel the watchers feed
/// and one [`AnimationTrack`] per parameter, and turns them into the
/// per-frame sampled values a shader uniform is written from.
///
/// It holds no watcher subscription: [`ParamAnimator::new`] hands the guards
/// back to the owner, which drops them before the animator. That keeps the
/// animator `Send` while a reactive frontend's guards are not.
pub struct ParamAnimator {
    /// Current parameter targets delivered by reactive watcher events.
    target_params: Vec<f32>,
    /// True when target parameters changed since the last successful render.
    target_params_dirty: bool,
    /// Animation state owned by the render thread.
    state: SharedAnimationState,
    /// Parameter-change events, each carrying optional animation metadata.
    events: Receiver<ParamAnimationEvent>,
    /// The sending half handed to watcher callbacks.
    sender: ParamSender,
}

impl core::fmt::Debug for ParamAnimator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParamAnimator")
            .field("target_params", &self.target_params)
            .field("has_active_animation", &self.state.has_active_animation)
            .finish_non_exhaustive()
    }
}

impl ParamAnimator {
    /// Creates an animator seeded with the parameters' initial values and
    /// installs one watcher per parameter through `install`.
    ///
    /// `install` receives the [`WatcherInstaller`] and is expected to visit
    /// every parameter (`visit_signals`/`visit_params`) with indices matching
    /// `initial_targets`. The returned guards keep those watchers alive; the
    /// owner drops them before the animator, whose channel they feed.
    pub fn new(
        initial_targets: Vec<f32>,
        install: impl FnOnce(&mut WatcherInstaller),
    ) -> (Self, Vec<WatchGuard>) {
        let (sender, events) = mpsc::channel();
        let sender = ParamSender {
            sender,
            redraw_callback: Arc::new(OnceLock::new()),
            events_pending: Arc::new(AtomicBool::new(false)),
        };
        let mut installer = WatcherInstaller {
            sender: sender.clone(),
            guards: Vec::with_capacity(initial_targets.len()),
        };
        install(&mut installer);

        let state = SharedAnimationState {
            tracks: initial_targets
                .iter()
                .copied()
                .map(|value| ParamTrackState {
                    track: AnimationTrack::new(value),
                    animated_target: None,
                })
                .collect(),
            current_values: initial_targets.clone(),
            has_active_animation: false,
        };
        let animator = Self {
            target_params: initial_targets,
            target_params_dirty: true,
            state,
            events,
            sender,
        };
        (animator, installer.guards)
    }

    /// Appends a parameter seeded with `initial` and returns its index.
    pub fn push_param(&mut self, initial: f32) -> usize {
        let index = self.target_params.len();
        self.target_params.push(initial);
        self.state.current_values.push(initial);
        self.state.tracks.push(ParamTrackState {
            track: AnimationTrack::new(initial),
            animated_target: None,
        });
        self.target_params_dirty = true;
        index
    }

    /// The number of parameters this animator drives.
    pub const fn param_count(&self) -> usize {
        self.target_params.len()
    }

    /// The sending half of the event channel, for watchers installed after
    /// construction.
    pub const fn sender(&self) -> &ParamSender {
        &self.sender
    }

    /// Installs the host wake callback. Must run exactly once, before setup.
    pub fn install_redraw_callback(&self, callback: EffectRedrawCallback) {
        assert!(
            self.sender.redraw_callback.set(callback).is_ok(),
            "filtrate redraw callback must be installed exactly once before setup"
        );
    }

    /// Fills in a no-op wake callback when the host never installed one, so
    /// watcher callbacks have something to call.
    pub fn ensure_redraw_callback(&self) {
        let _ = self
            .sender
            .redraw_callback
            .get_or_init(|| Arc::new(|| {}) as EffectRedrawCallback);
    }

    /// The installed wake callback, if any — used when chaining adapters.
    pub fn redraw_callback(&self) -> Option<EffectRedrawCallback> {
        self.sender.redraw_callback.get().cloned()
    }

    /// Snaps every current value to its target and clears animation state.
    /// Runs at setup completion so the first frame renders final values.
    pub fn apply_targets_to_current(&mut self) {
        let param_count = self.target_params.len();
        for i in 0..param_count {
            let target = self.target_params[i];
            self.state.current_values[i] = target;
            self.state.tracks[i].track.set_target(target, None);
            self.state.tracks[i].animated_target = None;
        }
        self.target_params_dirty = false;
    }

    fn consume_events(&mut self) {
        self.sender.events_pending.store(false, Ordering::Release);
        while let Ok(event) = self.events.try_recv() {
            assert!(
                event.param_index < self.state.current_values.len(),
                "filtrate watcher produced out-of-range parameter index {}",
                event.param_index
            );
            self.target_params[event.param_index] = event.target_value;
            self.target_params_dirty = true;
            let entry = &mut self.state.tracks[event.param_index];
            entry
                .track
                .set_target(event.target_value, event.interpolator);
            entry.animated_target = entry.track.is_active().then_some(event.target_value);
        }
        self.state.has_active_animation = self
            .state
            .tracks
            .iter()
            .any(|entry| entry.track.is_active());
    }

    /// Update interpolated parameters in-place; returns whether another frame is needed.
    pub fn update(&mut self, delta: Duration) -> bool {
        let param_count = self.target_params.len();
        self.consume_events();
        let mut needs_redraw = false;

        for i in 0..param_count {
            let target = self.target_params[i];
            let entry = &mut self.state.tracks[i];

            if let Some(animated_target) = entry.animated_target {
                // Underlying target changed without a new animation event:
                // fail fast to direct target sync so state stays coherent.
                if !approx_param_eq(animated_target, target) {
                    entry.track.set_target(target, None);
                    entry.animated_target = None;
                }
            }

            if entry.animated_target.is_none()
                && !approx_param_eq(self.state.current_values[i], target)
            {
                entry.track.set_target(target, None);
            }

            let active = entry.track.advance(delta);
            self.state.current_values[i] = entry.track.value();

            if active {
                needs_redraw = true;
            } else {
                entry.animated_target = None;
            }
        }

        self.state.has_active_animation = needs_redraw;
        needs_redraw
    }

    /// The per-frame sampled values, indexed as visited.
    pub fn current_values(&self) -> &[f32] {
        &self.state.current_values
    }

    /// Every parameter's largest magnitude until its running animation
    /// completes, after applying the events received so far: the bound
    /// [`SpatialFilter::footprint_of`](filtrate_core::SpatialFilter::footprint_of)
    /// is evaluated at to cover a whole animation.
    pub fn magnitude_bounds(&mut self) -> Vec<f32> {
        self.consume_events();
        self.state
            .tracks
            .iter()
            .zip(&self.target_params)
            .map(|(entry, target)| entry.track.magnitude_bound().max(target.abs()))
            .collect()
    }

    /// Marks the just-consumed targets as rendered.
    pub const fn mark_rendered(&mut self) {
        self.target_params_dirty = false;
    }

    /// Whether a parameter change or an active animation wants another frame.
    pub fn redraw_hint(&self) -> bool {
        self.target_params_dirty
            || self.state.has_active_animation
            || self.sender.events_pending.load(Ordering::Acquire)
    }
}
