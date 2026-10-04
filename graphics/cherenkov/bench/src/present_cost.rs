//! `present-cost`: present-pass GPU time for the sRGB gamut-map candidates
//! (#96) and the #98 wide-gamut/HDR output kinds.
//!
//! One [`Presenter::texture_timed`] pass per frame bracketed by
//! pass-boundary timestamps. Runs on any `TIMESTAMP_QUERY` adapter — the
//! point of the mode is that the same command measures the pass on the
//! M1 and the iPad, where wall clock is evidence; on a shared VM it is a
//! sanity check only.
//!
//! The source is a synthetic `Rgba16Float` working-space texture: `oog`
//! keeps every pixel on the gamut-mapping path (fully saturated P3
//! primaries and secondaries), `mixed` puts half the frame in-gamut.

use std::path::Path;

use cherenkov_gpu::GpuConfig;
use cherenkov_gpu::interop::{
    OutputAlpha, Presenter, SharedDevice, TextureOutput, shader_delivery, wgpu,
};

use crate::BenchError;
use crate::PresentKind;

use crate::cherenkov_ad::{present_color, present_format};
use crate::cli::PresentPattern;

/// One `present-cost` report: what ran and the per-frame pass times.
#[derive(serde::Serialize)]
struct CostReport {
    /// Adapter name, backend and driver for provenance.
    adapter: String,
    backend: String,
    driver: String,
    /// Measured surface size.
    width: u32,
    height: u32,
    pattern: &'static str,
    present: &'static str,
    /// The display headroom presented to (#97).
    headroom: f32,
    warmup_frames: u32,
    measured_frames: u32,
    /// Pass GPU time per measured frame, milliseconds.
    samples_ms: Vec<f64>,
    mean_ms: f64,
    p50_ms: f64,
    p99_ms: f64,
    min_ms: f64,
    /// Pipeline creation was inside the first (warmup) frame.
    note: &'static str,
}

/// Fills a `Rgba16Float` texture with the pattern's working-space pixels.
#[allow(clippy::many_single_char_names, clippy::suboptimal_flops)] // interpolation notation
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the pattern interpolant stays within [0, 5)"
)]
fn fill_pattern(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    pattern: PresentPattern,
) {
    let mut data = Vec::with_capacity((width * height * 8) as usize);
    let p3_corners: [[f64; 3]; 6] = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
    ];
    for y in 0..height {
        for x in 0..width {
            let u = f64::from(x) / f64::from(width.max(1));
            let v = f64::from(y) / f64::from(height.max(1));
            // Interpolate between P3 corners on a diagonal: most points
            // land outside sRGB.
            let seg = (u * 5.0).min(4.999);
            let t = seg.fract();
            let a = p3_corners[seg as usize];
            let b = p3_corners[seg as usize + 1];
            let px = if matches!(pattern, PresentPattern::Mixed) && x < width / 2 {
                // In-gamut half: a muted sRGB-legal gradient.
                [0.15 + 0.7 * u, 0.2 + 0.5 * v, 0.6 - 0.3 * u]
            } else {
                [
                    a[0] + t * (b[0] - a[0]) * (1.0 + 0.25 * v),
                    a[1] + t * (b[1] - a[1]) * (1.0 + 0.25 * v),
                    a[2] + t * (b[2] - a[2]),
                ]
            };
            for c in px {
                data.extend_from_slice(&half::f16::from_f64(c).to_le_bytes());
            }
            data.extend_from_slice(&half::f16::from_f32(1.0).to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Times `frames` present passes of `size` pixels after `warmup` untimed
/// frames, writing the JSON report to `out`.
#[expect(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    reason = "linear setup-measure-report sequence; counts and timestamps fit f64"
)]
pub(crate) fn run(
    size: (u32, u32),
    frames: u32,
    warmup: u32,
    pattern: PresentPattern,
    present: PresentKind,
    headroom: f32,
    out: &Path,
) -> Result<(), BenchError> {
    let (width, height) = size;
    let config = GpuConfig {
        timestamps: true,
        ..GpuConfig::default()
    };
    let shared = SharedDevice::create(&config)
        .map_err(|e| BenchError::Gpu(format!("present-cost device: {e}")))?;
    if !shared
        .device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        return Err(BenchError::Gpu(
            "present-cost needs an adapter with TIMESTAMP_QUERY".into(),
        ));
    }
    let delivery = shader_delivery(shared.adapter.get_info().backend, &shared.device)
        .map_err(|e| BenchError::Gpu(format!("present-cost shader delivery: {e}")))?;
    let mut presenter = Presenter::new(&shared.device, delivery);

    let device = &shared.device;
    let queue = &shared.queue;
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("present-cost source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    fill_pattern(queue, &source, width, height, pattern);
    let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
    let format = present_format(present);
    let destination = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("present-cost destination"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("present-cost timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 2 * frames,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("present-cost resolve"),
        size: u64::from(2 * frames) * 8,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("present-cost staging"),
        size: u64::from(2 * frames) * 8,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let adapter_info = shared.adapter.get_info();
    let output = || TextureOutput {
        texture: &destination,
        color: present_color(present),
        alpha: OutputAlpha::Premultiplied,
        headroom,
    };
    for _ in 0..warmup {
        presenter.texture_timed(device, queue, &source_view, output(), None);
    }
    for i in 0..frames {
        presenter.texture_timed(
            device,
            queue,
            &source_view,
            output(),
            Some(wgpu::RenderPassTimestampWrites {
                query_set: &queries,
                beginning_of_pass_write_index: Some(2 * i),
                end_of_pass_write_index: Some(2 * i + 1),
            }),
        );
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("present-cost resolve"),
    });
    encoder.resolve_query_set(&queries, 0..2 * frames, &resolve, 0);
    encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, u64::from(2 * frames) * 8);
    let submission = queue.submit([encoder.finish()]);

    let (send, recv) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .map_err(|e| BenchError::Gpu(format!("present-cost wait: {e}")))?;
    recv.recv()
        .map_err(|e| BenchError::Gpu(format!("present-cost readback: {e}")))?
        .map_err(|e| BenchError::Gpu(format!("present-cost map: {e}")))?;
    let data = staging
        .slice(..)
        .get_mapped_range()
        .expect("buffer range is mapped and not overlapping");
    let stamps: Vec<u64> = data
        .as_chunks::<8>()
        .0
        .iter()
        .map(|b| u64::from_le_bytes(*b))
        .collect();
    drop(data);
    staging.unmap();

    // wgpu timestamps are nanoseconds.
    let mut samples_ms: Vec<f64> = stamps
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| (w[1].saturating_sub(w[0])) as f64 / 1.0e6)
        .collect();
    samples_ms.sort_by(f64::total_cmp);
    let n = samples_ms.len();
    let report = CostReport {
        adapter: adapter_info.name,
        backend: format!("{:?}", adapter_info.backend),
        driver: format!("{} {}", adapter_info.driver, adapter_info.driver_info),
        width,
        height,
        pattern: match pattern {
            PresentPattern::Oog => "oog",
            PresentPattern::Mixed => "mixed",
        },
        present: present.name(),
        headroom,
        warmup_frames: warmup,
        measured_frames: frames,
        mean_ms: samples_ms.iter().sum::<f64>() / n as f64,
        p50_ms: samples_ms[n / 2],
        p99_ms: samples_ms[(n * 99 / 100).min(n - 1)],
        min_ms: samples_ms[0],
        samples_ms,
        note: "pass-boundary GPU timestamps; one submit per frame",
    };
    let file = std::fs::File::create(out)
        .map_err(|e| BenchError::Gpu(format!("write {}: {e}", out.display())))?;
    serde_json::to_writer_pretty(file, &report)
        .map_err(|e| BenchError::Gpu(format!("report json: {e}")))?;
    tracing::info!(
        out = %out.display(),
        mean_ms = report.mean_ms,
        p50_ms = report.p50_ms,
        p99_ms = report.p99_ms,
        "present-cost"
    );
    Ok(())
}
