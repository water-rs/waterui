//! Shared native GPU devices and engine-composed content presentation.

use alloc::sync::Arc;
use core::fmt;

use wgpu::{Adapter, Device, Instance, Queue, TextureFormat};

use super::{GpuContent, GpuContentView};
use crate::offscreen::{OffscreenImage, OffscreenSize};
use cherenkov::{Display, Engine, FrameTime, Next, Surface};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{
        GpuContentBox, OutputAlpha, OutputColor, Presenter, SharedDevice, TextureOutput,
        TextureTarget,
    },
};

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

    /// Creates an engine sharing the native host's device.
    ///
    /// # Errors
    /// When the engine cannot initialize its rendering resources.
    pub fn engine(&self) -> Result<Engine<Gpu>, cherenkov::EngineError> {
        Engine::new(GpuConfig {
            device: Some(SharedDevice {
                instance: self.instance().clone(),
                adapter: self.adapter().clone(),
                device: self.device().clone(),
                queue: self.queue().clone(),
            }),
            ..GpuConfig::default()
        })
    }

    /// Renders owned GPU content through the engine's offscreen surface.
    ///
    /// # Panics
    /// When engine creation, rendering, or readback fails.
    pub fn render_content(
        &self,
        content: impl GpuContent,
        size: OffscreenSize,
        scale: f32,
    ) -> OffscreenImage {
        let content = GpuContentView::new(content).take_engine_content(|| {});
        let mut renderer = GpuContentRenderer::new(self.clone(), content, size);
        renderer.render(size, scale);
        OffscreenImage::from_readback(
            &renderer
                .surface
                .readback()
                .expect("GPU content readback failed"),
        )
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

/// A retained engine surface for GPU content presented by a native host.
pub struct GpuContentRenderer {
    surface: Surface<Gpu>,
    engine: Engine<Gpu>,
    runtime: GpuRuntime,
    textures: std::sync::mpsc::Receiver<wgpu::Texture>,
    source: wgpu::Texture,
    presenter: Presenter,
}

impl fmt::Debug for GpuContentRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContentRenderer").finish_non_exhaustive()
    }
}

impl GpuContentRenderer {
    /// Moves the producer to a retained engine layer on the runtime's device.
    ///
    /// # Panics
    /// When engine or surface creation fails.
    pub fn new(runtime: GpuRuntime, content: GpuContentBox, size: OffscreenSize) -> Self {
        let engine = runtime
            .engine()
            .expect("native content engine creation failed");
        let pixels = (size.width(), size.height());
        let (target, textures) = TextureTarget::new(pixels);
        let surface = engine
            .surface(target)
            .expect("native content surface creation failed");
        let source = textures
            .try_recv()
            .expect("surface creation published its texture");
        surface.update(|tx| {
            tx[surface.root()].content(engine.gpu_content(pixels, content));
        });
        let presenter = Presenter::new(runtime.device());
        Self {
            surface,
            engine,
            runtime,
            textures,
            source,
            presenter,
        }
    }

    /// Renders a frame at the current size and display scale.
    ///
    /// # Panics
    /// When resizing, display configuration, or rendering fails.
    pub fn render(&mut self, size: OffscreenSize, scale: f32) -> Next {
        let pixels = (size.width(), size.height());
        if self.surface.size() != pixels {
            self.surface
                .resize(pixels)
                .expect("native content resize failed");
            self.surface.update(|tx| {
                tx[self.surface.root()].gpu_content_size(pixels);
            });
        }
        self.surface
            .display(Display {
                scale: f64::from(scale),
                ..Display::default()
            })
            .expect("native content display configuration failed");
        let next = self
            .engine
            .render(FrameTime::now())
            .expect("native content rendering failed");
        for texture in self.textures.try_iter() {
            self.source = texture;
        }
        next
    }

    /// Renders and composites into a native host's texture.
    /// Float targets carry extended linear Display P3; other targets carry sRGB.
    ///
    /// # Panics
    /// When the destination is empty or rendering fails.
    pub fn present(&mut self, target: &wgpu::Texture, scale: f32) -> Next {
        let size = OffscreenSize::try_from_pixels(target.width(), target.height())
            .expect("native target must be nonempty");
        let next = self.render(size, scale);
        let source = self
            .source
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.presenter.texture(
            self.runtime.device(),
            self.runtime.queue(),
            &source,
            TextureOutput {
                texture: target,
                color: if target.format() == TextureFormat::Rgba16Float {
                    OutputColor::LinearDisplayP3
                } else {
                    OutputColor::Srgb
                },
                alpha: OutputAlpha::Premultiplied,
            },
        );
        next
    }
}
