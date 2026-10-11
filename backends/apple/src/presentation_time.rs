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

use waterui_graphics::cherenkov::{FrameTime, Instant};

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

    /// Maps a `CACurrentMediaTime` timestamp — a link update's
    /// `targetPresentationTimestamp`, or an explicit offscreen capture
    /// time — onto the shared `Instant` axis.
    #[must_use]
    pub fn map(&self, media_time: f64) -> FrameTime {
        let delta = media_time - self.anchor_media;
        let duration = Duration::from_secs_f64(delta.abs());
        // A timestamp behind the anchor is legal (a queued update's target
        // can predate install). One the monotonic `Instant` axis cannot
        // represent is a broken invariant — fail fast rather than mint an
        // invented frame time.
        let instant = if delta >= 0.0 {
            self.anchor_instant.checked_add(duration)
        } else {
            self.anchor_instant.checked_sub(duration)
        }
        .expect("presentation timestamp outside the representable Instant range");
        FrameTime::at(instant)
    }

    /// The frame time an offscreen capture renders at — the explicit
    /// capture instant, mapped through the same anchor rather than a
    /// borrowed on-screen future timestamp.
    #[must_use]
    pub fn capture_time(&self) -> FrameTime {
        self.map(objc2_quartz_core::CACurrentMediaTime())
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
