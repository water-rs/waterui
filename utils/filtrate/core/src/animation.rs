//! `f32`-only animation track driven by an external [`Interpolator`].
//!
//! `AnimationTrack` is an internal helper used by the GPU runtime to smooth
//! parameter transitions between two values when animation metadata is
//! provided by a reactive watcher. It deliberately knows nothing about
//! easing curves, springs, or specific reactive systems — those are the
//! interpolator's responsibility.

extern crate alloc;

use alloc::boxed::Box;
use core::time::Duration;

use crate::Interpolator;

/// Active animation segment carried inside an [`AnimationTrack`].
struct ActiveSegment {
    interpolator: Box<dyn Interpolator>,
    elapsed: Duration,
    from: f32,
    to: f32,
}

/// Single-channel `f32` animation timeline.
///
/// The track stores the current sampled value plus an optional active
/// segment. Set a new target via [`AnimationTrack::set_target`]; advance it
/// each frame via [`AnimationTrack::advance`].
pub struct AnimationTrack {
    current: f32,
    active: Option<ActiveSegment>,
}

impl AnimationTrack {
    /// Create a track seeded with an initial value and no active segment.
    #[must_use]
    pub const fn new(initial: f32) -> Self {
        Self {
            current: initial,
            active: None,
        }
    }

    /// Current sampled value.
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.current
    }

    /// Replace the target value. With `Some(interpolator)` of nonzero
    /// duration the track animates smoothly; otherwise it snaps immediately.
    pub fn set_target(&mut self, target: f32, interpolator: Option<Box<dyn Interpolator>>) {
        match interpolator {
            Some(interpolator) if !interpolator.duration().is_zero() => {
                let from = self.current;
                self.active = Some(ActiveSegment {
                    interpolator,
                    elapsed: Duration::ZERO,
                    from,
                    to: target,
                });
            }
            _ => {
                self.current = target;
                self.active = None;
            }
        }
    }

    /// Advance by a frame delta. Returns `true` while a segment remains
    /// active after advancement.
    pub fn advance(&mut self, delta: Duration) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };

        active.elapsed = active.elapsed.saturating_add(delta);
        self.current = active
            .interpolator
            .interpolate(active.from, active.to, active.elapsed);

        if active.interpolator.is_complete(active.elapsed) {
            self.current = active.to;
            self.active = None;
            false
        } else {
            true
        }
    }

    /// Whether this track has an active segment.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

impl core::fmt::Debug for AnimationTrack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimationTrack")
            .field("current", &self.current)
            .field("is_active", &self.is_active())
            .finish_non_exhaustive()
    }
}
