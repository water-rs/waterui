//! Deterministic parameter animation: watcher installation, animation
//! events, and per-parameter track state.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use std::sync::{OnceLock, mpsc::Sender};

use filtrate_core::{AnimationTrack, FilterParam, Interpolator, SignalVisitor, WatchGuard};

use crate::effect::EffectRedrawCallback;

pub(super) const PARAM_EPSILON: f32 = 0.000_01;

#[derive(Debug)]
pub(super) struct ParamTrackState {
    pub(super) track: AnimationTrack,
    pub(super) animated_target: Option<f32>,
}

/// Shared animation state that can be updated from watcher callbacks.
#[derive(Debug)]
pub(super) struct SharedAnimationState {
    /// Animation timeline for each parameter index.
    pub(super) tracks: Vec<ParamTrackState>,
    /// Current values for each parameter (either animated or direct).
    pub(super) current_values: Vec<f32>,
    /// Whether any animation is active.
    pub(super) has_active_animation: bool,
}

pub(super) const fn approx_param_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= PARAM_EPSILON
}

pub(super) struct ParamAnimationEvent {
    pub(super) param_index: usize,
    pub(super) target_value: f32,
    pub(super) interpolator: Option<Box<dyn Interpolator>>,
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

pub(super) struct WatcherInstaller<'a> {
    pub(super) sender: Sender<ParamAnimationEvent>,
    pub(super) redraw_callback: Arc<OnceLock<EffectRedrawCallback>>,
    pub(super) events_pending: Arc<core::sync::atomic::AtomicBool>,
    pub(super) guards: &'a mut Vec<WatchGuard>,
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
                .expect("FilterAdapter parameter event receiver dropped while watcher is active");
            events_pending.store(true, core::sync::atomic::Ordering::Release);
            if let Some(callback) = redraw_callback.get() {
                callback();
            }
        }));
        self.guards.push(guard);
    }
}
