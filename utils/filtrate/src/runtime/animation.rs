//! Deterministic parameter animation: watcher installation, animation
//! events, and per-parameter track state.
//!
//! [`ParamAnimator`] is the whole reactive-parameter driver shared by the
//! single-input [`FilterAdapter`](super::FilterAdapter) and the multi-input
//! [`MultiInputFilter`](crate::multi_input::MultiInputFilter): watchers feed
//! change events into a channel, each render drains the channel into
//! per-parameter [`AnimationTrack`]s, and the sampled values become the
//! shader's parameter block for that frame.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::time::Duration;
use std::sync::{
    OnceLock,
    mpsc::{self, Receiver, Sender},
};

use filtrate_core::{AnimationTrack, FilterParam, Interpolator, SignalVisitor, WatchGuard};

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
// Stage and signal visitors used by the planner / animation watcher install.
// ============================================================================

pub struct WatcherInstaller<'a> {
    pub sender: Sender<ParamAnimationEvent>,
    pub redraw_callback: Arc<OnceLock<EffectRedrawCallback>>,
    pub events_pending: Arc<core::sync::atomic::AtomicBool>,
    pub guards: &'a mut Vec<WatchGuard>,
}

impl SignalVisitor for WatcherInstaller<'_> {
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P) {
        let sender = self.sender.clone();
        let redraw_callback = self.redraw_callback.clone();
        let events_pending = self.events_pending.clone();
        let guard = param.watch_animated(Box::new(move |target| {
            sender
                .send(ParamAnimationEvent {
                    param_index,
                    target_value: target.value,
                    interpolator: target.interpolator,
                })
                .expect("filtrate parameter event receiver dropped while watcher is active");
            events_pending.store(true, core::sync::atomic::Ordering::Release);
            if let Some(callback) = redraw_callback.get() {
                callback();
            }
        }));
        self.guards.push(guard);
    }
}

/// Collects the initial snapshot of every visited parameter, indexed exactly
/// as the watchers will later report changes.
pub struct SnapshotCollector {
    pub values: Vec<f32>,
}

impl SignalVisitor for SnapshotCollector {
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P) {
        assert!(
            param_index < self.values.len(),
            "filtrate parameter visitor produced out-of-range index {param_index}"
        );
        self.values[param_index] = param.snapshot();
    }
}

/// The reactive-parameter driver: owns the watcher subscriptions, the event
/// channel, and one [`AnimationTrack`] per parameter, and turns them into the
/// per-frame sampled values a shader uniform is written from.
pub struct ParamAnimator {
    /// Current parameter targets delivered by reactive watcher events.
    target_params: Vec<f32>,
    /// True when target parameters changed since the last successful render.
    target_params_dirty: bool,
    /// Animation state owned by the render thread.
    state: SharedAnimationState,
    /// Parameter watcher guards dropped before their event channel and callback.
    _watcher_guards: Vec<WatchGuard>,
    /// Parameter-change events, each carrying optional animation metadata.
    events: Receiver<ParamAnimationEvent>,
    /// Set by watcher callbacks the instant an event is queued, cleared when
    /// events are consumed — lets `redraw_hint` see changes that arrived
    /// between frames without draining the channel.
    events_pending: Arc<core::sync::atomic::AtomicBool>,
    /// Host wake callback shared with every parameter watcher.
    redraw_callback: Arc<OnceLock<EffectRedrawCallback>>,
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
    /// `initial_targets`.
    pub fn new(initial_targets: Vec<f32>, install: impl FnOnce(&mut WatcherInstaller<'_>)) -> Self {
        let (sender, events) = mpsc::channel();
        let redraw_callback = Arc::new(OnceLock::new());
        let events_pending = Arc::new(core::sync::atomic::AtomicBool::new(false));
        let mut watcher_guards = Vec::with_capacity(initial_targets.len());
        install(&mut WatcherInstaller {
            sender,
            redraw_callback: redraw_callback.clone(),
            events_pending: events_pending.clone(),
            guards: &mut watcher_guards,
        });

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
        Self {
            target_params: initial_targets,
            target_params_dirty: true,
            state,
            _watcher_guards: watcher_guards,
            events,
            events_pending,
            redraw_callback,
        }
    }

    /// Installs the host wake callback. Must run exactly once, before setup.
    pub fn install_redraw_callback(&self, callback: EffectRedrawCallback) {
        assert!(
            self.redraw_callback.set(callback).is_ok(),
            "filtrate redraw callback must be installed exactly once before setup"
        );
    }

    /// Fills in a no-op wake callback when the host never installed one, so
    /// watcher callbacks have something to call.
    pub fn ensure_redraw_callback(&self) {
        let _ = self
            .redraw_callback
            .get_or_init(|| Arc::new(|| {}) as EffectRedrawCallback);
    }

    /// The installed wake callback, if any — used when chaining adapters.
    pub fn redraw_callback(&self) -> Option<EffectRedrawCallback> {
        self.redraw_callback.get().cloned()
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
        self.events_pending
            .store(false, core::sync::atomic::Ordering::Release);
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

    /// Marks the just-consumed targets as rendered.
    pub const fn mark_rendered(&mut self) {
        self.target_params_dirty = false;
    }

    /// Whether a parameter change or an active animation wants another frame.
    pub fn redraw_hint(&self) -> bool {
        self.target_params_dirty
            || self.state.has_active_animation
            || self
                .events_pending
                .load(core::sync::atomic::Ordering::Acquire)
    }
}
