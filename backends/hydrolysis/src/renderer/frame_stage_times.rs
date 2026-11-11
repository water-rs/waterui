//! Per-frame CPU stage timing behind the `frame-profile` feature.
//!
//! The stages are measured inside [`HydrolysisRenderer::flush_window_tree`]
//! and apply to every painter. The GPU spans a frame also carries come from
//! `gpu_profile`, which only the GPU painter links.

use core::time::Duration;

use super::HydrolysisRenderer;

/// Per-frame stage timing for `frame-profile` consumers.
///
/// The CPU split of the per-frame pass plus timestamped GPU spans. GPU fields
/// stay `None` on a device created without `TIMESTAMP_QUERY`, and always on a
/// painter without a GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameStageTimes {
    /// Runtime and reactive update plus the retained-tree patch: viewport
    /// setup, frame-boundary bookkeeping and `RenderNode::patch`.
    update: Duration,
    /// Measure and layout of the retained tree.
    layout: Duration,
    /// The retained tree's record into its nodes' programs.
    encode: Duration,
    /// Timestamped span covering the frame's layer-content submits — the
    /// engine's render and the effect passes around it. `None` when the
    /// device lacks `TIMESTAMP_QUERY`.
    pub(super) content_gpu: Option<Duration>,
    /// Timestamped span covering the final surface pass — the presenter's
    /// conversion submit, or the whole-window render on the direct
    /// GPU-surface path. `None` like `content_gpu`.
    pub(super) compositor_gpu: Option<Duration>,
    /// CPU time spent waiting for GPU work: the queue drain each marker does
    /// plus the blocking timestamp resolve — the wait for this frame's GPU
    /// submits to complete.
    pub(super) gpu_wait: Duration,
    /// CPU time spent reading the presented texture back for a snapshot.
    readback: Duration,
}

impl FrameStageTimes {
    /// Runtime and reactive update plus the retained-tree patch: viewport
    /// setup, frame-boundary bookkeeping and `RenderNode::patch`.
    #[must_use]
    pub const fn update(&self) -> Duration {
        self.update
    }

    /// Sets the `update`.
    pub const fn set_update(&mut self, update: Duration) {
        self.update = update;
    }

    /// Measure and layout of the retained tree.
    #[must_use]
    pub const fn layout(&self) -> Duration {
        self.layout
    }

    /// Sets the `layout`.
    pub const fn set_layout(&mut self, layout: Duration) {
        self.layout = layout;
    }

    /// The retained tree's record into its nodes' programs.
    #[must_use]
    pub const fn encode(&self) -> Duration {
        self.encode
    }

    /// Sets the `encode`.
    pub const fn set_encode(&mut self, encode: Duration) {
        self.encode = encode;
    }

    /// Timestamped span covering the frame's layer-content submits — the
    /// engine's render and the effect passes around it. `None` when the
    /// device lacks `TIMESTAMP_QUERY`.
    #[must_use]
    pub const fn content_gpu(&self) -> Option<Duration> {
        self.content_gpu
    }

    /// Timestamped span covering the final surface pass — the presenter's
    /// conversion submit, or the whole-window render on the direct
    /// GPU-surface path. `None` like `content_gpu`.
    #[must_use]
    pub const fn compositor_gpu(&self) -> Option<Duration> {
        self.compositor_gpu
    }

    /// CPU time spent waiting for GPU work: the queue drain each marker does
    /// plus the blocking timestamp resolve — the wait for this frame's GPU
    /// submits to complete.
    #[must_use]
    pub const fn gpu_wait(&self) -> Duration {
        self.gpu_wait
    }

    /// CPU time spent reading the presented texture back for a snapshot.
    #[must_use]
    pub const fn readback(&self) -> Duration {
        self.readback
    }

    /// Sets the `readback`.
    pub const fn set_readback(&mut self, readback: Duration) {
        self.readback = readback;
    }
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// The frame's accumulated CPU stage times plus the resolved GPU spans,
    /// resetting for the next frame. Called once per pump by the runner.
    pub fn take_frame_stage_times(&mut self) -> FrameStageTimes {
        core::mem::take(&mut self.frame_stage_times)
    }

    /// Digest of the last layout pass's placed bounds — a deterministic hash
    /// of every node's frame, for before/after correctness checks.
    #[must_use]
    pub const fn layout_signature(&self) -> Option<u64> {
        self.last_layout_signature
    }
}
