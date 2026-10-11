//! Shared `wgpu` device setup, readback and timestamp helpers for the GPU
//! adapters (`vello-classic`, `vello-hybrid`).

use std::time::{Duration, Instant};

use cherenkov_oracle::F32Image;
use wgpu29::{
    Buffer, BufferDescriptor, BufferUsages, Device, DeviceDescriptor, Extent3d, Instance, MapMode,
    MemoryHints, PollType, QuerySet, QueryType, Queue, SubmissionIndex, Texture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureView,
};

use crate::{BenchError, DeviceInfo};

/// Target usages the render target texture must allow.
const TARGET_USAGES: TextureUsages = TextureUsages::from_bits_retain(
    TextureUsages::RENDER_ATTACHMENT.bits()
        | TextureUsages::TEXTURE_BINDING.bits()
        | TextureUsages::STORAGE_BINDING.bits()
        | TextureUsages::COPY_SRC.bits(),
);

/// Candidate target formats, best first: straight RGBA, then BGRA
/// (readback swaps the channels).
const TARGET_FORMATS: [TextureFormat; 2] = [TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Unorm];

/// The longest a single GPU wait (drain, timestamp or pixel readback) may
/// block before it fails with [`BenchError::Gpu`] naming what was awaited.
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// A `wgpu` device + queue + adapter info, requested with timestamp-query
/// support.
pub struct Gpu {
    /// The device.
    pub device: Device,
    /// The queue.
    pub queue: Queue,
    /// Adapter info for provenance.
    pub info: wgpu29::AdapterInfo,
    /// The target texture format chosen from the adapter's queried
    /// [`wgpu29::Adapter::get_texture_format_features`]. Recorded in
    /// provenance.
    pub target_format: TextureFormat,
    /// Whether timestamp queries are available.
    pub timestamps: bool,
    /// Timestamp query set (`Some` iff `timestamps`).
    pub query_set: Option<QuerySet>,
    /// Buffer receiving resolved query pairs (`Some` iff `timestamps`).
    pub query_buffer: Option<Buffer>,
    /// A 1×1 attachment cleared by the marker render pass whose boundary
    /// carries each frame timestamp (`Some` iff `timestamps`).
    pub marker: Option<TextureView>,
}

/// The wgpu handle types do not format.
impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Creates an instance, adapter and device.
    ///
    /// Every enumerated adapter is queried for a target format that allows
    /// [`TARGET_USAGES`]; adapters that allow none (e.g. a GLES backend
    /// without `STORAGE_BINDING`, or a driver that rejects the
    /// `Rgba8Unorm + COPY_SRC` combination) are skipped, and the failure
    /// reports what was tried instead of panicking inside `wgpu`.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when no adapter offers a usable target format or
    /// device creation fails.
    pub fn new() -> Result<Self, BenchError> {
        let instance = Instance::new(wgpu29::InstanceDescriptor {
            backends: wgpu29::Backends::PRIMARY | wgpu29::Backends::GL,
            ..wgpu29::InstanceDescriptor::new_without_display_handle()
        });
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu29::Backends::all()));
        let mut attempts = Vec::new();
        // (adapter, format, has-timestamps): prefer a timestamp-capable
        // adapter, then a usable format.
        let mut chosen: Option<(wgpu29::Adapter, TextureFormat, bool)> = None;
        for adapter in adapters {
            let info = adapter.get_info();
            let ts = adapter
                .features()
                .contains(wgpu29::Features::TIMESTAMP_QUERY);
            for format in TARGET_FORMATS {
                let feats = adapter.get_texture_format_features(format);
                if feats.allowed_usages.contains(TARGET_USAGES) {
                    if chosen.as_ref().is_none_or(|(_, _, old)| ts && !*old) {
                        chosen = Some((adapter.clone(), format, ts));
                    }
                    break;
                }
                attempts.push(format!(
                    "{} ({:?}): {format:?} lacks {TARGET_USAGES:?} (has {:?})",
                    info.name, info.backend, feats.allowed_usages
                ));
            }
        }
        let Some((adapter, target_format, _)) = chosen else {
            return Err(BenchError::Gpu(format!(
                "no wgpu adapter exposes a target format allowing {TARGET_USAGES:?}; tried: {}",
                attempts.join("; ")
            )));
        };
        let info = adapter.get_info();
        let supported = adapter.features();
        let timestamps = supported.contains(wgpu29::Features::TIMESTAMP_QUERY);
        tracing::info!(
            name = %info.name,
            backend = ?info.backend,
            timestamp_query = timestamps,
            timestamps_inside_encoders =
                supported.contains(wgpu29::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
            "bench adapter"
        );
        // Only pass-boundary timestamps are used: Metal on Apple GPUs
        // advertises `TIMESTAMP_QUERY_INSIDE_ENCODERS` but samples only at
        // stage boundaries, and wgpu's dummy-blit emulation of an
        // encoder-level stamp can leave the command buffer unfinished.
        let mut required = wgpu29::Features::empty();
        if timestamps {
            required |= wgpu29::Features::TIMESTAMP_QUERY;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("cherenkov-bench"),
            required_features: required,
            // Clamp the portable defaults to what the adapter reports:
            // iOS Metal offers 15 inter-stage varyings (60 components)
            // where `Limits::default` asks for 16.
            required_limits: wgpu29::Limits::default().or_worse_values_from(&adapter.limits()),
            experimental_features: wgpu29::ExperimentalFeatures::disabled(),
            memory_hints: MemoryHints::Performance,
            trace: wgpu29::Trace::Off,
        }))
        .map_err(|e| BenchError::Gpu(format!("device request failed: {e}")))?;
        let (query_set, query_buffer, marker) = if timestamps {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("timestamp marker"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            (
                Some(device.create_query_set(&wgpu29::QuerySetDescriptor {
                    label: Some("frame timestamps"),
                    ty: QueryType::Timestamp,
                    count: 2,
                })),
                // wgpu allows MAP_READ only with COPY_DST, so the query
                // resolves into a device buffer that is then copied to
                // staging for the readback.
                Some(device.create_buffer(&BufferDescriptor {
                    label: Some("timestamp resolve buffer"),
                    size: 16,
                    usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })),
                Some(texture.create_view(&wgpu29::TextureViewDescriptor::default())),
            )
        } else {
            (None, None, None)
        };
        Ok(Self {
            device,
            queue,
            info,
            target_format,
            timestamps,
            query_set,
            query_buffer,
            marker,
        })
    }

    /// Device info provenance for reports, including the chosen target
    /// texture format.
    #[must_use]
    pub fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            adapter: Some(self.info.name.clone()),
            backend: Some(format!("{:?}", self.info.backend)),
            driver: Some(self.info.driver.clone()),
            driver_info: Some(self.info.driver_info.clone()),
            vendor: Some(self.info.vendor),
            device: Some(self.info.device),
            target_format: Some(format!("{:?}", self.target_format)),
            cpu: crate::cpu_model(),
            thermal_celsius: crate::thermal_celsius(),
        }
    }
}

/// An 8-bit-per-channel render target the engines draw into, in the
/// adapter-queried [`Gpu::target_format`].
#[derive(Debug)]
pub struct Target {
    /// The texture.
    pub texture: Texture,
    /// Its view.
    pub view: TextureView,
    /// Its format (equals [`Gpu::target_format`]).
    pub format: TextureFormat,
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
}

impl Target {
    /// Creates a render target of `format` — a format
    /// [`Gpu::new`] already validated against [`TARGET_USAGES`].
    #[must_use]
    pub fn new(device: &Device, width: u32, height: u32, format: TextureFormat) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("cherenkov target"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TARGET_USAGES,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu29::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            format,
            width,
            height,
        }
    }
}

/// Copies `target` into a mapped buffer and converts it to working-space
/// pixels.
///
/// # Errors
/// [`BenchError::Gpu`] on buffer map failure.
pub fn readback(gpu: &Gpu, target: &Target) -> Result<F32Image, BenchError> {
    let bytes_per_row = (target.width * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&BufferDescriptor {
        label: Some("readback"),
        size: u64::from(bytes_per_row) * u64::from(target.height),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu29::CommandEncoderDescriptor {
            label: Some("readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu29::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu29::Origin3d::ZERO,
            aspect: wgpu29::TextureAspect::All,
        },
        wgpu29::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu29::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(target.height),
            },
        },
        Extent3d {
            width: target.width,
            height: target.height,
            depth_or_array_layers: 1,
        },
    );
    let submission = gpu.queue.submit([encoder.finish()]);
    tracing::trace!(?submission, "readback submitted");
    let slice = buf.slice(..);
    map_read(gpu, slice, submission, "the pixel readback")?;
    let data = slice.get_mapped_range();
    let mut rgba8 = Vec::with_capacity((target.width * target.height * 4) as usize);
    for row in 0..target.height {
        let start = (row * bytes_per_row) as usize;
        rgba8.extend_from_slice(&data[start..start + (target.width * 4) as usize]);
    }
    drop(data);
    buf.unmap();
    // A BGRA target needs the R/B channels swapped back to RGBA order.
    if target.format == TextureFormat::Bgra8Unorm {
        for px in rgba8.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
    }
    Ok(crate::convert::rgba8_to_working(
        target.width,
        target.height,
        &rgba8,
    ))
}

/// Blocks until `submission` (or, for `None`, everything submitted so far)
/// has completed, for at most [`WAIT_TIMEOUT`]; `what` names the work for
/// the error.
///
/// # Errors
/// [`BenchError::Gpu`] when the wait times out or the device is lost.
pub fn wait(gpu: &Gpu, submission: Option<SubmissionIndex>, what: &str) -> Result<(), BenchError> {
    let start = Instant::now();
    let status = gpu.device.poll(PollType::Wait {
        submission_index: submission,
        timeout: Some(WAIT_TIMEOUT),
    });
    let elapsed = start.elapsed();
    match status {
        Ok(status) => {
            tracing::trace!(
                what,
                ?status,
                wait_ms = elapsed.as_secs_f64() * 1e3,
                "waited"
            );
            Ok(())
        }
        Err(wgpu29::PollError::Timeout) => {
            tracing::error!(what, ?elapsed, "GPU wait timed out");
            Err(BenchError::Gpu(format!(
                "the GPU did not finish {what} within {WAIT_TIMEOUT:?}"
            )))
        }
        Err(e) => Err(BenchError::Gpu(format!("waiting for {what} failed: {e}"))),
    }
}

/// Maps `slice` for reading once `submission` has completed, bounded by
/// [`wait`].
fn map_read(
    gpu: &Gpu,
    slice: wgpu29::BufferSlice<'_>,
    submission: SubmissionIndex,
    what: &str,
) -> Result<(), BenchError> {
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    wait(gpu, Some(submission), what)?;
    match rx.try_recv() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(BenchError::Gpu(format!("{what}: map failed: {e}"))),
        Err(_) => Err(BenchError::Gpu(format!(
            "{what}: the map callback did not run after the wait"
        ))),
    }
}

/// Writes timestamp `index` at the boundary of a marker render pass (a
/// clear of [`Gpu::marker`]) in `encoder`: index 0 at the pass's start,
/// any other at its end. Pass boundaries are the one timestamp position
/// every `TIMESTAMP_QUERY` backend supports, Metal on Apple GPUs included.
fn stamp(gpu: &Gpu, encoder: &mut wgpu29::CommandEncoder, index: u32) {
    let (Some(qs), Some(marker)) = (&gpu.query_set, &gpu.marker) else {
        return;
    };
    let (beginning, end) = if index == 0 {
        (Some(index), None)
    } else {
        (None, Some(index))
    };
    encoder.begin_render_pass(&wgpu29::RenderPassDescriptor {
        label: Some("timestamp marker"),
        color_attachments: &[Some(wgpu29::RenderPassColorAttachment {
            view: marker,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu29::Operations {
                load: wgpu29::LoadOp::Clear(wgpu29::Color::BLACK),
                store: wgpu29::StoreOp::Discard,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: Some(wgpu29::RenderPassTimestampWrites {
            query_set: qs,
            beginning_of_pass_write_index: beginning,
            end_of_pass_write_index: end,
        }),
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

/// Drains the queue completely, then writes timestamp `index` in its own
/// submission.
///
/// On job-scheduled tiled GPUs (Mali et al.) a standalone timestamp
/// submission shares no hazard with the engine's work, so a marker submitted
/// alongside the frame can run *concurrently* and bracket an empty interval.
/// Draining first (a bounded [`wait`] on every prior submission) guarantees
/// the marker executes when the queue is empty, so a `stamp(0)` → engine
/// submit → `stamp(1)` sequence brackets the engine's real GPU execution.
/// This serializes CPU and GPU for the measured frame — GPU-timed `measure`
/// is a synchronous probe, not a pipelined frame rate.
///
/// No-op when timestamps are unsupported.
///
/// # Errors
/// [`BenchError::Gpu`] when the drain times out or the device is lost.
pub fn drain_and_stamp(gpu: &Gpu, index: u32) -> Result<(), BenchError> {
    if gpu.query_set.is_none() {
        return Ok(());
    }
    wait(gpu, None, "the queue before a frame timestamp")?;
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu29::CommandEncoderDescriptor {
            label: Some("timestamp"),
        });
    stamp(gpu, &mut encoder, index);
    let submission = gpu.queue.submit([encoder.finish()]);
    tracing::trace!(index, ?submission, "frame timestamp submitted");
    Ok(())
}

/// Resolves a timestamp query pair written around a submission into
/// seconds. `None` when timestamps are unsupported.
///
/// # Errors
/// [`BenchError::Gpu`] on buffer map failure.
#[expect(
    clippy::cast_precision_loss,
    reason = "a tick delta of a timed frame fits f64 mantissa"
)]
pub fn resolve_timestamps(gpu: &Gpu) -> Result<Option<f64>, BenchError> {
    let (Some(qs), Some(buf)) = (&gpu.query_set, &gpu.query_buffer) else {
        return Ok(None);
    };
    let staging = gpu.device.create_buffer(&BufferDescriptor {
        label: Some("timestamp staging"),
        size: 16,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu29::CommandEncoderDescriptor {
            label: Some("timestamp resolve"),
        });
    encoder.resolve_query_set(qs, 0..2, buf, 0);
    encoder.copy_buffer_to_buffer(buf, 0, &staging, 0, 16);
    let submission = gpu.queue.submit([encoder.finish()]);
    tracing::trace!(?submission, "timestamps resolved");
    let slice = staging.slice(..);
    map_read(gpu, slice, submission, "the timestamp readback")?;
    let data = slice.get_mapped_range();
    let ticks: &[u64] = bytemuck::cast_slice(&data);
    let seconds = if ticks.len() >= 2 && ticks[1] > ticks[0] {
        Some(f64::from(gpu.queue.get_timestamp_period()) * (ticks[1] - ticks[0]) as f64 * 1e-9)
    } else {
        None
    };
    drop(data);
    staging.unmap();
    Ok(seconds)
}
