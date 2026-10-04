//! Frame driving: [`Engine::render`](crate::Engine::render) input, the
//! [`Next`] scheduling answer, per-frame statistics and readback types.
//!
//! [`Engine::render`]: crate::Engine::render

use crate::Instant;
use std::ops::RangeInclusive;

/// The presentation timestamp handed to [`Engine::render`](crate::Engine::render).
#[derive(Clone, Copy, Debug)]
pub struct FrameTime(pub Instant);

impl FrameTime {
    /// The frame time at `t`.
    #[must_use]
    pub const fn at(t: Instant) -> Self {
        Self(t)
    }

    /// The frame time now.
    #[must_use]
    pub fn now() -> Self {
        Self(Instant::now())
    }
}

/// An inclusive refresh-rate range in hertz.
pub type RefreshRange = RangeInclusive<u32>;

/// The refresh range a surface requests when its host does not set one: up
/// to 120 Hz, the performance target on current high-refresh displays.
pub const DEFAULT_REFRESH: RefreshRange = 60..=120;

/// What the engine needs next, returned by
/// [`Engine::render`](crate::Engine::render).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// No animation is running; the display link may sleep.
    Idle,
    /// The next frame is needed at `time`, at a refresh rate in `rate`.
    At {
        /// When the next frame is due.
        time: Instant,
        /// The acceptable refresh rates.
        rate: RefreshRange,
    },
}

/// Identifies one [`Engine::render`](crate::Engine::render): the render
/// loop numbers every render from zero, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId(pub(crate) u64);

impl FrameId {
    /// Creates an identifier from a raw value.
    ///
    /// [`Engine::render`](crate::Engine::render) numbers renders itself; this
    /// is for harnesses driving a [`Renderer`](crate::Renderer) directly.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The render's position in the engine's sequence of renders.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The GPU time of one submitted frame, read from its timestamp queries.
///
/// GPU timings accumulate on the renderer and are returned only by
/// [`Engine::finish_timings`](crate::Engine::finish_timings), oldest first.
/// This tooling call waits for frames still on the GPU; rendering never
/// waits for GPU idle.
#[derive(Clone, Debug)]
pub struct FrameTiming {
    /// The frame measured.
    pub frame: FrameId,
    /// GPU seconds from the first pass's start to the last pass's end;
    /// `None` when the adapter wrote timestamps that do not increase
    /// across that span.
    pub gpu_seconds: Option<f64>,
    /// Each pass of the frame, in submission order; empty when the
    /// backend times only whole frames.
    pub passes: Vec<PassTiming>,
}

/// One timed render pass of a [`FrameTiming`].
#[derive(Clone, Debug)]
pub struct PassTiming {
    /// The pass's deterministic name: `"surface"` or `"scratch{n}"` by
    /// isolation depth.
    pub name: String,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target texture format (`"rgba16float"`, `"rgba8unorm"`, ...).
    pub format: &'static str,
    /// GPU seconds the pass took, between its pass-boundary timestamp
    /// writes; `None` when the adapter's end timestamp does not exceed
    /// its start.
    pub gpu_seconds: Option<f64>,
}

/// Wall-clock seconds the render thread spent in each phase of the last
/// [`crate::Engine::render`]; independent of GPU timestamp support.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phases {
    /// CPU lowering of every dirty surface (raster, instance build, uploads).
    pub lower_seconds: f64,
    /// Command encoding and queue submission.
    pub encode_seconds: f64,
    /// Timestamp bracket overhead: the drains around the stamps and the
    /// timestamp resolve/readback. Zero when timestamps are off.
    pub stamp_seconds: f64,
    /// Blocking waits on the GPU: for an upload staging slot still in
    /// flight, and the final drain of the queue.
    pub wait_seconds: f64,
}

/// Measurements of the last [`Engine::render`](crate::Engine::render).
#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    /// This render's frame, when it drew any surface; `None` when no
    /// surface had changed.
    pub frame: Option<FrameId>,
    /// CPU time spent in each render phase.
    pub phases: Phases,
    /// Render passes recorded (`render_to_texture` and effect passes).
    pub passes: u32,
    /// Scene commands encoded this frame.
    pub draws: u32,
    /// Pipeline state changes issued while encoding draw ranges.
    pub pipeline_switches: u32,
    /// Texture bind groups created this frame; cached groups are reused.
    pub bind_groups_created: u32,
    /// Layer content instances drawn.
    pub instances: u32,
    /// Glyphs rasterized this frame.
    pub glyphs_rasterized: u32,
    /// Paths rasterized this frame.
    pub paths_rasterized: u32,
    /// Display-list commands lowered this frame (a full lowering or the
    /// dirty commands of a slot update).
    pub commands_lowered: u32,
    /// Layers whose device-space content run was rebuilt rather than reused.
    pub layers_composed: u32,
    /// Projective layers' local images realized this frame: a local
    /// rasterization plus a mip build each. Zero on a warm matrix-only
    /// frame whose density buckets are retained.
    pub projective_realized: u32,
    /// Projective layers composed this frame, cached or fresh.
    pub projective_composed: u32,
}

/// Decoded pixels of a surface readback: premultiplied linear Display P3,
/// row-major.
#[derive(Clone, Debug)]
pub struct Readback {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height` premultiplied RGBA pixels.
    pub pixels: Vec<[f32; 4]>,
}

/// The storage format of an [`Offscreen`] target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OffscreenFormat {
    /// Linear `Rgba16Float`.
    #[default]
    LinearF16,
    /// Linear `Rgba32Float`.
    LinearF32,
}

/// An offscreen render target description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offscreen {
    /// The target size in pixels.
    pub size: (u32, u32),
    /// The target storage format.
    pub format: OffscreenFormat,
    /// Refresh range used by animated backend content.
    pub refresh: RefreshRange,
}

impl Offscreen {
    /// Configure the refresh range of this target.
    ///
    /// # Panics
    /// When the range is empty or contains zero hertz.
    #[must_use]
    pub fn rate(mut self, rate: RefreshRange) -> Self {
        assert!(
            *rate.start() > 0 && !rate.is_empty(),
            "refresh range must be positive and ordered"
        );
        self.refresh = rate;
        self
    }

    /// An offscreen target of `size` pixels in `format`.
    #[must_use]
    pub const fn new(size: (u32, u32), format: OffscreenFormat) -> Self {
        Self {
            size,
            format,
            refresh: DEFAULT_REFRESH,
        }
    }
}
