//! Shared `wgpu` device setup and readback helpers for the GPU
//! adapters (`vello-classic`, `vello-hybrid`).

use std::time::{Duration, Instant};

use cherenkov_oracle::F32Image;
use wgpu::{
    BufferDescriptor, BufferUsages, Device, DeviceDescriptor, Extent3d, Instance, MapMode,
    MemoryHints, PollType, Queue, SubmissionIndex, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureView,
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

/// A `wgpu` device + queue + adapter info, with supported profiler features.
pub struct Gpu {
    /// The device.
    pub device: Device,
    /// The queue.
    pub queue: Queue,
    /// Adapter info for provenance.
    pub info: wgpu::AdapterInfo,
    /// The target texture format chosen from the adapter's queried
    /// [`wgpu::Adapter::get_texture_format_features`]. Recorded in
    /// provenance.
    pub target_format: TextureFormat,
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
    /// `TARGET_USAGES`; adapters that allow none (e.g. a GLES backend
    /// without `STORAGE_BINDING`, or a driver that rejects the
    /// `Rgba8Unorm + COPY_SRC` combination) are skipped, and the failure
    /// reports what was tried instead of panicking inside `wgpu`.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when no adapter offers a usable target format or
    /// device creation fails.
    pub fn new(profiler_features: wgpu::Features) -> Result<Self, BenchError> {
        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let mut attempts = Vec::new();
        // (adapter, format, has-timestamps): prefer a timestamp-capable
        // adapter, then a usable format.
        let mut chosen: Option<(wgpu::Adapter, TextureFormat, bool)> = None;
        for adapter in adapters {
            let info = adapter.get_info();
            let ts = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
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
        let timestamps = supported.contains(wgpu::Features::TIMESTAMP_QUERY);
        tracing::info!(
            name = %info.name,
            backend = ?info.backend,
            timestamp_query = timestamps,
            timestamps_inside_encoders =
                supported.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
            "bench adapter"
        );
        // Classic's existing profiler uses encoder timestamps. Do not request
        // them on Metal: the stage-boundary emulation does not provide the
        // encoder interval needed by this measurement.
        let required = if info.backend == wgpu::Backend::Metal {
            wgpu::Features::empty()
        } else {
            supported & profiler_features
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("cherenkov-bench"),
            required_features: required,
            // Clamp the portable defaults to what the adapter reports:
            // iOS Metal offers 15 inter-stage varyings (60 components)
            // where `Limits::default` asks for 16.
            required_limits: wgpu::Limits::default().or_worse_values_from(&adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| BenchError::Gpu(format!("device request failed: {e}")))?;
        Ok(Self {
            device,
            queue,
            info,
            target_format,
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
    /// [`Gpu::new`] already validated against `TARGET_USAGES`.
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
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
///
/// # Panics
/// When the mapped buffer range cannot be read after a successful map.
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
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
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
    let data = slice
        .get_mapped_range()
        .expect("buffer range is mapped and not overlapping");
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
/// has completed, for at most `WAIT_TIMEOUT`; `what` names the work for
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
        Err(wgpu::PollError::Timeout) => {
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
    slice: wgpu::BufferSlice<'_>,
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
