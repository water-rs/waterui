//! Visual parity test harness for filter pipelines.
//!
//! [`ParityHarness`](crate::filter_parity::ParityHarness) provides a self-contained wgpu environment and the
//! plumbing needed to render any `F: Effect` against a caller-supplied
//! RGBA8 input buffer and read the result back into a `Vec<u8>`. Companion
//! comparators ([`pixel_max_abs_diff`](crate::filter_parity::pixel_max_abs_diff), [`pixel_mean_abs_diff`](crate::filter_parity::pixel_mean_abs_diff)) measure
//! how close two RGBA8 buffers are.
//!
//! Together, these let backend handlers register native compositor paths
//! (via [`crate::filter_registry::FilterHandlerRegistry`]) and prove
//! visually that they match the wgpu reference within a tolerance of
//! choice — without a separate CI golden-image pipeline.
//!
//! ```ignore
//! use waterui_graphics::filter_parity::{ParityHarness, pixel_max_abs_diff};
//! use waterui_graphics::filter_view::FilterAdapter;
//! use filtrate::filters::Brightness;
//!
//! let harness = ParityHarness::new().expect("no compatible adapter");
//! let input = harness.solid_rgba(64, 64, [128, 64, 32, 255]);
//! let reference = harness
//!     .render(FilterAdapter::new(Brightness(0.0_f32)), &input, 64, 64, 64, 64)
//!     .expect("render must succeed");
//! let candidate = harness
//!     .render(FilterAdapter::new(Brightness(0.0_f32)), &input, 64, 64, 64, 64)
//!     .expect("render must succeed");
//! assert_eq!(pixel_max_abs_diff(&reference, &candidate), 0);
//! ```

extern crate alloc;
extern crate std;

use alloc::vec::Vec;
use core::fmt;
use std::sync::mpsc;

use filtrate::{Effect, EffectContext, EffectInput, EffectOutput};

use crate::pollster;

/// Wraps a wgpu device, queue, and adapter info for use by parity tests.
pub struct ParityHarness {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_info: wgpu::AdapterInfo,
}

impl fmt::Debug for ParityHarness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParityHarness")
            .field("adapter", &self.adapter_info.name)
            .field("backend", &self.adapter_info.backend)
            .finish()
    }
}

impl ParityHarness {
    /// Try to acquire a high-performance adapter and device. Returns
    /// `None` when no compatible adapter is available (typical in CI
    /// containers without a GPU).
    #[must_use]
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        let adapter_info = adapter.get_info();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Self {
            device,
            queue,
            adapter_info,
        })
    }

    /// Returns adapter info for diagnostics. Useful for skipping tests on
    /// known-broken backends.
    #[must_use]
    pub const fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    /// Build a solid-colour RGBA8 buffer at the requested dimensions.
    #[must_use]
    pub fn solid_rgba(&self, width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            out.extend_from_slice(&rgba);
        }
        out
    }

    /// Render `filter` against `input_rgba` and return the output RGBA8
    /// buffer. Returns `None` only if the filter setup or render fails.
    pub fn render<F: Effect>(
        &self,
        mut filter: F,
        input_rgba: &[u8],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
    ) -> Option<Vec<u8>> {
        assert_eq!(
            input_rgba.len(),
            (input_width * input_height * 4) as usize,
            "input_rgba length must equal width * height * 4"
        );
        let format = wgpu::TextureFormat::Rgba8Unorm;

        let input_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ParityHarness input"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            input_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(input_width * 4),
                rows_per_image: Some(input_height),
            },
            wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
        );

        let output_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ParityHarness output"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let ctx = EffectContext {
            device: &self.device,
            queue: &self.queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
        };
        if pollster::block_on(filter.setup(&ctx)).is_err() {
            return None;
        }

        let input = EffectInput {
            device: &self.device,
            queue: &self.queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: input_width,
            height: input_height,
        };
        let output = EffectOutput {
            device: &self.device,
            queue: &self.queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: output_width,
            height: output_height,
        };

        if filter.render(&input, &output).is_err() {
            return None;
        }

        Some(self.readback_rgba8(&output_texture, output_width, output_height))
    }

    fn readback_rgba8(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<u8> {
        const BYTES_PER_PIXEL: u32 = 4;
        let unpadded_bpr = width * BYTES_PER_PIXEL;
        let padded_bpr = unpadded_bpr.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let copy_size = u64::from(padded_bpr * height);

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParityHarness readback buffer"),
            size: copy_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ParityHarness readback encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .expect("buffer map callback should return")
            .expect("buffer mapping must succeed");

        let mapped = slice.get_mapped_range();
        let mut out = vec![0u8; (width * height * BYTES_PER_PIXEL) as usize];
        for row in 0..height as usize {
            let src_start = row * padded_bpr as usize;
            let src_end = src_start + unpadded_bpr as usize;
            let dst_start = row * unpadded_bpr as usize;
            let dst_end = dst_start + unpadded_bpr as usize;
            out[dst_start..dst_end].copy_from_slice(&mapped[src_start..src_end]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }
}

/// Maximum absolute per-channel difference between two equally-sized
/// RGBA8 buffers. Returns `u32::MAX` when the buffers have different
/// lengths.
#[must_use]
pub fn pixel_max_abs_diff(reference: &[u8], candidate: &[u8]) -> u32 {
    if reference.len() != candidate.len() {
        return u32::MAX;
    }
    reference
        .iter()
        .zip(candidate.iter())
        .map(|(&a, &b)| u32::from(a.abs_diff(b)))
        .max()
        .unwrap_or(0)
}

/// Mean absolute per-channel difference between two equally-sized RGBA8
/// buffers, in the range `0.0..=255.0`. Returns `f64::INFINITY` when the
/// buffers have different lengths.
#[must_use]
pub fn pixel_mean_abs_diff(reference: &[u8], candidate: &[u8]) -> f64 {
    if reference.len() != candidate.len() || reference.is_empty() {
        return f64::INFINITY;
    }
    let total: u64 = reference
        .iter()
        .zip(candidate.iter())
        .map(|(&a, &b)| u64::from(a.abs_diff(b)))
        .sum();
    total as f64 / reference.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_max_abs_diff_zero_for_identical() {
        let a = [10_u8, 20, 30, 40, 50, 60, 70, 80];
        assert_eq!(pixel_max_abs_diff(&a, &a), 0);
    }

    #[test]
    fn pixel_max_abs_diff_picks_the_largest_channel_delta() {
        let a = [10_u8, 20, 30, 40];
        let b = [10_u8, 25, 30, 200];
        assert_eq!(pixel_max_abs_diff(&a, &b), 160);
    }

    #[test]
    fn pixel_mean_abs_diff_zero_for_identical() {
        let a = [10_u8, 20, 30, 40];
        assert!(pixel_mean_abs_diff(&a, &a).abs() < f64::EPSILON);
    }

    #[test]
    fn pixel_mean_abs_diff_handles_size_mismatch() {
        assert_eq!(pixel_mean_abs_diff(&[1, 2, 3], &[1, 2]), f64::INFINITY);
    }

    #[test]
    fn parity_self_brightness_zero_is_identity() {
        // When wgpu is unavailable (CI containers) the harness returns None;
        // the test then becomes a no-op. This mirrors the rest of the GPU
        // tests in this crate.
        let Some(harness) = ParityHarness::new() else {
            eprintln!("ParityHarness: no compatible adapter, skipping");
            return;
        };
        let input = harness.solid_rgba(8, 8, [123, 45, 67, 255]);
        let out_a = harness
            .render(
                crate::filter_view::FilterAdapter::new(filtrate::filters::Brightness(0.0_f32)),
                &input,
                8,
                8,
                8,
                8,
            )
            .expect("brightness(0) render should succeed");
        let out_b = harness
            .render(
                crate::filter_view::FilterAdapter::new(filtrate::filters::Brightness(0.0_f32)),
                &input,
                8,
                8,
                8,
                8,
            )
            .expect("second brightness(0) render should succeed");
        // Same filter, same input, same harness — pixel-identical.
        assert_eq!(pixel_max_abs_diff(&out_a, &out_b), 0);
        // Brightness(0) on an opaque solid colour leaves the channels
        // unchanged within the wgpu Rgba8Unorm round-trip tolerance.
        assert!(
            pixel_max_abs_diff(&input, &out_a) <= 1,
            "brightness(0) drifted by more than 1 LSB"
        );
    }
}
