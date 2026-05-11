//! Reactive parameter abstraction for filter inputs.
//!
//! `FilterParam` lets a filter accept either static `f32` values or richer
//! reactive sources (signals from a UI framework, video pipeline timecodes,
//! etc.) without knowing about any specific reactive system. The runtime
//! samples each parameter every frame via [`FilterParam::snapshot`] and
//! optionally subscribes to value changes via [`FilterParam::watch_animated`].
//!
//! # Roles
//!
//! - The filter library itself is reactive-system agnostic.
//! - Reactive frontends provide [`FilterParam`] implementations for their
//!   own signal types (or wrappers around them).
//! - The runtime in `filtrate` consumes `FilterParam` snapshots and (when
//!   provided) animation interpolators to produce smooth GPU updates.

extern crate alloc;

use alloc::boxed::Box;
use core::any::Any;
use core::time::Duration;

/// A scalar `f32` parameter that can be sampled at render time and
/// optionally observed for animated changes.
///
/// `f32` itself implements this trait as a static value. Reactive systems
/// provide their own `FilterParam` implementations on signal wrappers.
pub trait FilterParam: 'static {
    /// Sample the current parameter value.
    fn snapshot(&self) -> f32;

    /// Install a callback invoked when the underlying value changes. The
    /// default implementation returns `None`, indicating the value is not
    /// observable; the runtime then falls back to per-frame snapshots.
    ///
    /// Returning `Some(WatchGuard)` keeps the subscription alive until the
    /// guard is dropped. The runtime drops the guard when the filter is
    /// torn down.
    fn watch_animated(&self, callback: AnimatedCallback) -> Option<WatchGuard> {
        let _ = callback;
        None
    }
}

impl FilterParam for f32 {
    #[inline]
    fn snapshot(&self) -> f32 {
        *self
    }
}

/// Boxed callback type passed to [`FilterParam::watch_animated`]. The
/// callback is invoked on every observed change, so it is `Fn` rather than
/// `FnMut` — implementations needing mutable state should rely on interior
/// mutability (`Mutex`, `Cell`, …).
pub type AnimatedCallback = Box<dyn Fn(AnimatedTarget) + Send + Sync>;

/// Carries a new target value plus optional animation interpolator from a
/// watcher callback into the runtime.
pub struct AnimatedTarget {
    /// The new target parameter value.
    pub value: f32,
    /// Optional interpolator. If `Some`, the runtime smooths from the
    /// previous value to `value` along this curve. If `None`, the value
    /// snaps immediately on the next render.
    pub interpolator: Option<Box<dyn Interpolator>>,
}

impl core::fmt::Debug for AnimatedTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimatedTarget")
            .field("value", &self.value)
            .field("animated", &self.interpolator.is_some())
            .finish()
    }
}

/// Interpolator drives a smooth transition between two `f32` values over
/// time.
///
/// Reactive frontends (e.g. `WaterUI`) wrap their own animation primitives in
/// this trait to feed `filtrate`'s runtime without coupling either side.
pub trait Interpolator: Send + 'static {
    /// Total duration of the animation.
    fn duration(&self) -> Duration;

    /// Sample the interpolated value at `elapsed` time since the animation
    /// started, given the start (`from`) and end (`to`) values.
    fn interpolate(&self, from: f32, to: f32, elapsed: Duration) -> f32;

    /// Whether the animation is finished at `elapsed` time.
    fn is_complete(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration()
    }
}

/// Opaque guard returned by [`FilterParam::watch_animated`] that keeps a
/// subscription alive. Dropping the guard cancels the subscription.
///
/// The guard is not required to be `Send`: filter pipelines run on a single
/// scheduler thread today, and reactive frontends commonly use non-`Send`
/// subscription handles.
pub struct WatchGuard(#[expect(dead_code, reason = "lifetime-only owner")] Box<dyn Any>);

impl WatchGuard {
    /// Wrap any owned value as a watch guard. The runtime drops this when
    /// it tears down the filter, which transitively drops the subscription.
    #[must_use]
    pub fn new<T: Any + 'static>(inner: T) -> Self {
        Self(Box::new(inner))
    }
}

impl core::fmt::Debug for WatchGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WatchGuard").finish_non_exhaustive()
    }
}
