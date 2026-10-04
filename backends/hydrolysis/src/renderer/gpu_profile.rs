//! Per-frame stage timing behind the `frame-profile` feature.
//!
//! CPU stages are measured inside
//! [`HydrolysisRenderer::flush_window_tree`]; GPU stages use `wgpu`
//! timestamp queries: three marker submits bracket the frame's layer-content
//! submits (the engine's render and the effect passes around it) and the
//! presentation pass, and one blocking resolve reports both spans. Each marker
//! first drains the queue — on a tiled GPU a timestamp submission that shares
//! no memory hazard with the work it brackets may be scheduled concurrently
//! with it, so the drain is what pins the write to the boundary — then the
//! resolve waits out whatever remains. A device without
//! `wgpu::Features::TIMESTAMP_QUERY` reports the GPU stages as `None` — GPU
//! time is never estimated.

use super::*;

/// Timestamp slots one frame writes, in submission order: before the
/// layer-content submits, before the composite submit, and after it.
const QUERY_SLOTS: u32 = 3;

/// Per-frame stage timing for `frame-profile` consumers: the CPU split of the
/// per-frame pass plus timestamped GPU spans. GPU fields stay `None` on a
/// device created without `TIMESTAMP_QUERY`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStageTimes {
    /// Runtime and reactive update plus the retained-tree patch: viewport
    /// setup, frame-boundary bookkeeping and `RenderNode::patch`.
    pub update: Duration,
    /// Measure and layout of the retained tree.
    pub layout: Duration,
    /// The retained tree's flush into `Recording` and the scene-layer
    /// bookkeeping up to `flush_scene_layer`.
    pub encode: Duration,
    /// Timestamped span covering the frame's layer-content submits — the
    /// engine's render and the effect passes around it. `None` when the
    /// device lacks `TIMESTAMP_QUERY`.
    pub content_gpu: Option<Duration>,
    /// Timestamped span covering the final surface pass — the presenter's
    /// conversion submit, or the whole-window render on the direct
    /// GPU-surface path. `None` like `content_gpu`.
    pub compositor_gpu: Option<Duration>,
    /// CPU time spent waiting for GPU work: the queue drain each marker does
    /// plus the blocking timestamp resolve — the wait for this frame's GPU
    /// submits to complete.
    pub gpu_wait: Duration,
    /// CPU time spent reading the presented texture back for a snapshot.
    pub readback: Duration,
}

/// Which GPU a runtime renders on — attribution for a frame-profile report.
#[derive(Clone, Debug)]
pub struct GpuIdentity {
    /// `Adapter::get_info` for the adapter the device was requested from.
    pub adapter: wgpu::AdapterInfo,
    /// Features the adapter advertises.
    pub adapter_features: wgpu::Features,
    /// Features the device was opened with — `TIMESTAMP_QUERY` may be absent
    /// here even when the adapter has it, when the device request ran before
    /// the feature was wired in.
    pub device_features: wgpu::Features,
}

/// The timestamp query set plus its resolve and staging buffers, created once
/// a `TIMESTAMP_QUERY` device exists. The resolve lands in a `COPY_SRC` buffer
/// and is copied into a `MAP_READ` one — wgpu forbids `MAP_READ` combining
/// with `QUERY_RESOLVE`.
pub(crate) struct GpuFrameProfiler {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
}

impl GpuFrameProfiler {
    /// A profiler for `device`, or `None` when the device lacks
    /// `TIMESTAMP_QUERY_INSIDE_ENCODERS` — the markers are written outside
    /// render passes, so plain `TIMESTAMP_QUERY` alone does not suffice.
    pub(crate) fn new(device: &wgpu::Device) -> Option<Self> {
        if !device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
        {
            return None;
        }
        Some(Self {
            query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("hydrolysis_frame_profile_queries"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERY_SLOTS,
            }),
            resolve_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hydrolysis_frame_profile_resolve"),
                size: u64::from(QUERY_SLOTS) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hydrolysis_frame_profile_staging"),
                size: u64::from(QUERY_SLOTS) * 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        })
    }

    /// Submits a timestamp write into `slot` after draining the queue, and
    /// returns how long the drain blocked the CPU.
    ///
    /// The drain is the ordering guarantee: submissions that share no memory
    /// hazard may execute concurrently on the GPU, and a bare timestamp write
    /// is exactly that, so on Mali both markers resolved back-to-back while
    /// the content submits they bracket were still running. Waiting for the
    /// queue to go idle first means the timestamp only writes once every
    /// earlier submission has completed, pinning it to the boundary it marks.
    fn mark(&self, device: &wgpu::Device, queue: &wgpu::Queue, slot: u32) -> Duration {
        let drain_started_at = Instant::now();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let waited = drain_started_at.elapsed();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_frame_profile_marker"),
        });
        encoder.write_timestamp(&self.query_set, slot);
        queue.submit([encoder.finish()]);
        waited
    }

    /// Submits the resolve and waits for it — the frame's GPU drain. Returns
    /// the resolved nanosecond timestamps in slot order.
    fn resolve(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> [u64; QUERY_SLOTS as usize] {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_frame_profile_resolve_encoder"),
        });
        encoder.resolve_query_set(&self.query_set, 0..QUERY_SLOTS, &self.resolve_buffer, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve_buffer,
            0,
            &self.staging_buffer,
            0,
            u64::from(QUERY_SLOTS) * 8,
        );
        queue.submit([encoder.finish()]);

        let slice = self.staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        receiver
            .recv()
            .expect("hydrolysis frame profile map callback dropped")
            .expect("hydrolysis failed to map frame profile buffer");

        let mapped = slice
            .get_mapped_range()
            .expect("hydrolysis failed to read frame profile buffer");
        let mut timestamps = [0u64; QUERY_SLOTS as usize];
        for (slot, bytes) in mapped.as_chunks::<8>().0.iter().enumerate() {
            timestamps[slot] = u64::from_le_bytes(*bytes);
        }
        drop(mapped);
        self.staging_buffer.unmap();
        timestamps
    }
}

impl HydrolysisRenderer {
    /// Writes a timestamp marker when this frame can be profiled, folding the
    /// marker's queue drain into `gpu_wait`; a no-op when the frame's context
    /// has no profiler (a device without `TIMESTAMP_QUERY`).
    pub(crate) fn gpu_profile_mark(
        &mut self,
        profiler: Option<&GpuFrameProfiler>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: u32,
    ) {
        if let Some(profiler) = profiler {
            self.core.state_mut().counters.gpu_submissions += 1;
            self.frame_stage_times.gpu_wait += profiler.mark(device, queue, slot);
        }
    }

    /// Resolves this frame's markers into `frame_stage_times`, blocking until
    /// the GPU drains the frame's submits. Called once per presented frame by
    /// the surface render path; a no-op when the frame's context has no
    /// profiler.
    pub(crate) fn finish_gpu_frame_profile(
        &mut self,
        gpu_context_id: u64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let Some(profiler) = self
            .cherenkov_windows
            .get(&gpu_context_id)
            .and_then(|window| window.gpu_profiler.as_ref())
        else {
            return;
        };
        let wait_started_at = Instant::now();
        self.core.state_mut().counters.gpu_submissions += 1;
        let [before_content, before_composite, end] = profiler.resolve(device, queue);
        self.frame_stage_times.gpu_wait += wait_started_at.elapsed();
        // Query results are device-clock ticks; the period turns them into ns
        // (1.0 on most adapters, but not guaranteed).
        let period = f64::from(queue.get_timestamp_period());
        let ns = |ticks: u64| Duration::from_nanos((ticks as f64 * period).round() as u64);
        self.frame_stage_times.content_gpu =
            Some(ns(before_composite.saturating_sub(before_content)));
        self.frame_stage_times.compositor_gpu = Some(ns(end.saturating_sub(before_composite)));
    }

    /// The frame's accumulated CPU stage times plus the resolved GPU spans,
    /// resetting for the next frame. Called once per pump by the runner.
    pub fn take_frame_stage_times(&mut self) -> FrameStageTimes {
        core::mem::take(&mut self.frame_stage_times)
    }

    /// Digest of the last layout pass's placed bounds — a deterministic hash
    /// of every node's frame, for before/after correctness checks.
    #[must_use]
    pub fn layout_signature(&self) -> Option<u64> {
        self.last_layout_signature
    }
}
