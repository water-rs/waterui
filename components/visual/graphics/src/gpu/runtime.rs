//! A `wgpu` device for hosts that present [`GpuContent`](super::GpuContent)
//! themselves.
//!
//! A backend rendering through a Cherenkov engine hands content the engine's
//! device. A native host — Android's `SurfaceView`, an Apple layer showing a
//! Metal texture — has no engine on the presenting thread and owns its device
//! here instead, one per environment, shared by every GPU view it presents.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;

use wgpu::{Adapter, Device, Instance, Queue, TextureFormat};

use super::{Context, Frame, GpuContent, RedrawHandle};
use crate::offscreen::{OffscreenImage, OffscreenSize};

/// Why a [`GpuRuntime`] could not be created.
#[derive(Debug, thiserror::Error)]
pub enum GpuRuntimeError {
    /// No adapter satisfied the request.
    #[error("no compatible GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// The adapter refused the device.
    #[error(transparent)]
    Device(#[from] wgpu::RequestDeviceError),
}

struct Inner {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
}

/// A shared `wgpu` instance, adapter, device and queue.
///
/// Cloning shares the same device.
#[derive(Clone)]
pub struct GpuRuntime(Arc<Inner>);

impl fmt::Debug for GpuRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuRuntime")
            .field("adapter", &self.0.adapter.get_info().name)
            .finish_non_exhaustive()
    }
}

impl GpuRuntime {
    /// Requests a high-performance adapter and a device with default limits.
    ///
    /// # Errors
    /// [`GpuRuntimeError`] when no adapter or device is available.
    pub async fn new() -> Result<Self, GpuRuntimeError> {
        let instance =
            Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("waterui GpuRuntime"),
                ..Default::default()
            })
            .await?;
        Ok(Self(Arc::new(Inner {
            instance,
            adapter,
            device,
            queue,
        })))
    }

    /// The instance surfaces are created from.
    #[must_use]
    pub fn instance(&self) -> &Instance {
        &self.0.instance
    }

    /// The adapter the device was created from.
    #[must_use]
    pub fn adapter(&self) -> &Adapter {
        &self.0.adapter
    }

    /// The device.
    #[must_use]
    pub fn device(&self) -> &Device {
        &self.0.device
    }

    /// The device's queue.
    #[must_use]
    pub fn queue(&self) -> &Queue {
        &self.0.queue
    }

    /// Sets `content` up on this device, renders one frame into an
    /// `Rgba8Unorm` texture of `size` pixels and reads it back.
    ///
    /// For tests and smoke checks that want a [`GpuContent`]'s first frame as
    /// an image without a presenting host.
    ///
    /// # Panics
    /// When the readback buffer cannot be mapped.
    pub fn render_content(
        &self,
        content: &mut dyn GpuContent,
        size: OffscreenSize,
        scale: f32,
    ) -> OffscreenImage {
        let (width, height) = (size.width(), size.height());
        let format = TextureFormat::Rgba8Unorm;
        let device = self.device();
        let queue = self.queue();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("waterui GpuRuntime::render_content"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        content.setup(&Context {
            adapter: self.adapter(),
            device,
            queue,
            format,
            redraw: RedrawHandle::new(|| {}),
        });
        let mut frame = Frame::new(
            device,
            queue,
            &texture,
            &view,
            format,
            (width, height),
            scale,
            (Duration::ZERO, Duration::ZERO),
        );
        content.render(&mut frame);

        let bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("waterui GpuRuntime::render_content readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).expect("readback receiver dropped");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("waiting for the readback failed");
        receiver
            .recv()
            .expect("readback callback never ran")
            .expect("mapping the readback buffer failed");
        let mapped = slice.get_mapped_range();
        let mut rgba8 = Vec::with_capacity((width * height * 4) as usize);
        for row in mapped.chunks_exact(bytes_per_row as usize) {
            rgba8.extend_from_slice(&row[..(width * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();
        OffscreenImage {
            width,
            height,
            rgba8,
        }
    }
}

/// The swapchain format a host configures for `capabilities`.
///
/// Prefers a 16-bit float format when the host asks for HDR and the surface
/// offers one; otherwise the first 8-bit sRGB-encoded format, then the first
/// format the surface offers at all.
///
/// # Panics
/// When the surface offers no format.
#[must_use]
pub fn preferred_surface_format(
    capabilities: &wgpu::SurfaceCapabilities,
    prefer_hdr: bool,
) -> TextureFormat {
    let formats = &capabilities.formats;
    if prefer_hdr && let Some(f) = formats.iter().find(|f| **f == TextureFormat::Rgba16Float) {
        return *f;
    }
    formats
        .iter()
        .find(|f| f.is_srgb())
        .or_else(|| formats.first())
        .copied()
        .expect("surface offers no texture format")
}
