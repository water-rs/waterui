//! The shared presentation-time anchor every presenter maps
//! `CACurrentMediaTime` through — one explicit
//! `(media_seconds, cherenkov::Instant)` pair, cached in the render
//! environment so the same target timestamp maps to the same `Instant`
//! nanoseconds across every surface under the GPU runtime (#1683, and the
//! key #1725's one-engine-per-context rendering will join on).
//!
//! No global clock and no per-frame `Instant::now()`: a per-call anchor
//! would add callback jitter, and independent per-presenter anchors would
//! map the same media timestamp to different instants.

use std::rc::Rc;
use std::time::Duration;

use waterui_graphics::backend::cherenkov::{FrameTime, Instant};

/// The anchor all presenters under one GPU runtime share.
#[derive(Debug)]
pub struct PresentationTime {
    /// The `CACurrentMediaTime` sample the anchor was taken at.
    anchor_media: f64,
    /// The `Instant` sampled in the same statement — the two clocks' one
    /// measured correspondence.
    anchor_instant: Instant,
}

impl PresentationTime {
    /// Anchors the mapping at install time — the two samples are taken back
    /// to back so the anchor pair names one real instant on both clocks.
    #[must_use]
    pub fn new() -> Self {
        let anchor_media = objc2_quartz_core::CACurrentMediaTime();
        let anchor_instant = Instant::now();
        Self {
            anchor_media,
            anchor_instant,
        }
    }

    /// Maps a display link's `targetPresentationTimestamp` onto the shared
    /// `Instant` axis as the frame's reported presentation time.
    #[must_use]
    pub fn map(&self, media_time: f64) -> FrameTime {
        FrameTime::presented_at(self.instant_at(media_time))
    }

    /// Maps a `CACurrentMediaTime` timestamp onto the shared `Instant` axis.
    fn instant_at(&self, media_time: f64) -> Instant {
        let delta = media_time - self.anchor_media;
        let duration = Duration::from_secs_f64(delta.abs());
        // A timestamp behind the anchor is legal (a queued update's target
        // can predate install). One the monotonic `Instant` axis cannot
        // represent is a broken invariant — fail fast rather than mint an
        // invented frame time.
        if delta >= 0.0 {
            self.anchor_instant.checked_add(duration)
        } else {
            self.anchor_instant.checked_sub(duration)
        }
        .expect("presentation timestamp outside the representable Instant range")
    }

    /// The frame time an offscreen capture renders at — the explicit
    /// capture instant, mapped through the same anchor rather than a
    /// borrowed on-screen future timestamp. No platform target is reported.
    #[must_use]
    pub fn capture_time(&self) -> FrameTime {
        FrameTime::unpresented_at(self.instant_at(objc2_quartz_core::CACurrentMediaTime()))
    }

    /// Installs the anchor in `env` — once: [`crate::gpu_runtime::prepare`]
    /// runs it before any mount exists.
    ///
    /// # Panics
    ///
    /// When an anchor was already installed on this environment.
    pub fn install(env: &mut waterui_backend_core::Environment) {
        assert!(
            env.get::<Rc<Self>>().is_none(),
            "a presentation anchor is already installed in the WaterUI environment"
        );
        env.insert(Rc::new(Self::new()));
    }

    /// The environment's shared anchor.
    ///
    /// # Panics
    ///
    /// When no anchor was installed — [`install`] runs inside
    /// [`crate::gpu_runtime::prepare`] before any mount exists.
    #[must_use]
    pub fn get(env: &waterui_backend_core::Environment) -> Rc<Self> {
        env.get::<Rc<Self>>()
            .expect("presentation-time anchor is not installed in the WaterUI environment")
            .clone()
    }
}

impl Default for PresentationTime {
    fn default() -> Self {
        Self::new()
    }
}
