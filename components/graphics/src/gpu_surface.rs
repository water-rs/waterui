//! High-performance GPU rendering surface using wgpu.
//!
//! This module provides `GpuSurface`, a raw view that enables direct wgpu access
//! for custom GPU rendering at up to 120fps+.

extern crate alloc;

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

use waterui_core::{layout::StretchAxis, raw_view};

/// A boxed future for async setup operations.
pub type SetupFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

/// Picks the best surface format for a [`GpuSurface`].
///
/// `WaterUI` prefers HDR surfaces when available. If the platform/surface does not support an HDR
/// format, it falls back to a standard sRGB swapchain format (or the first supported format).
#[must_use]
pub fn preferred_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    // HDR (linear, extended range) preferred when supported by the surface.
    let hdr = wgpu::TextureFormat::Rgba16Float;
    if caps.formats.contains(&hdr) {
        return hdr;
    }

    // Otherwise, prefer sRGB for correct UI compositing on SDR displays.
    if let Some(fmt) = caps
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
    {
        return fmt;
    }

    // Fallback: use the first reported format.
    caps.formats
        .first()
        .copied()
        .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb)
}

/// GPU resources provided to the renderer during setup.
///
/// Contains references to the wgpu device, queue, and surface format
/// that the renderer can use to create pipelines, buffers, and other resources.
pub struct GpuContext<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The texture format of the surface.
    pub surface_format: wgpu::TextureFormat,
    /// Optional pipeline cache for faster pipeline creation.
    pub pipeline_cache: Option<&'a wgpu::PipelineCache>,
}

impl core::fmt::Debug for GpuContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuContext")
            .field("surface_format", &self.surface_format)
            .finish_non_exhaustive()
    }
}

impl GpuContext<'_> {
    /// Returns `true` if the surface format is HDR-capable (floating-point).
    #[must_use]
    pub const fn is_hdr(&self) -> bool {
        matches!(
            self.surface_format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
    }
}

/// Frame data provided during each render call.
///
/// Contains references to the GPU resources and the current frame's texture,
/// along with the current surface dimensions from the layout system.
pub struct GpuFrame<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The current frame's texture to render into.
    pub texture: &'a wgpu::Texture,
    /// A view into the current frame's texture.
    pub view: wgpu::TextureView,
    /// The texture format of the surface.
    pub format: wgpu::TextureFormat,
    /// Current width in pixels (from layout system).
    pub width: u32,
    /// Current height in pixels (from layout system).
    pub height: u32,
}

impl core::fmt::Debug for GpuFrame<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuFrame")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl GpuFrame<'_> {
    /// Returns `true` if the frame format is HDR-capable (floating-point).
    #[must_use]
    pub const fn is_hdr(&self) -> bool {
        matches!(
            self.format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
    }
}

/// Trait for GPU renderers.
///
/// Implement this trait to create custom GPU rendering logic.
/// The renderer will be called with GPU resources during setup,
/// and then called each frame to perform rendering.
///
/// # Async Setup
///
/// The `setup` method returns a future, allowing async initialization (e.g., SVG parsing).
/// For sync renderers, simply return `async {}` after doing sync work.
///
/// **Note:** The future does not require `Send` - it's created and awaited on the same thread.
/// For heavy CPU work, use `smol::unblock` to run on a thread pool.
///
/// # Example
///
/// ```ignore
/// struct TriangleRenderer {
///     pipeline: Option<wgpu::RenderPipeline>,
/// }
///
/// impl GpuRenderer for TriangleRenderer {
///     fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
///         // Sync work: create pipeline, buffers, etc.
///         self.pipeline = Some(ctx.device.create_render_pipeline(&...));
///         async {} // Immediately ready
///     }
///
///     fn render(&mut self, frame: &GpuFrame) {
///         let mut encoder = frame.device.create_command_encoder(&Default::default());
///         // ... render to frame.view ...
///         frame.queue.submit([encoder.finish()]);
///     }
/// }
/// ```
pub trait GpuRenderer: 'static {
    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, buffers, bind groups, and other
    /// GPU resources that persist across frames.
    ///
    /// Returns a future that completes when setup is done. For sync renderers,
    /// return `async {}` after performing sync work.
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()>;

    /// Called each frame to render.
    ///
    /// Use `frame.width` and `frame.height` to get the current surface dimensions.
    /// Render into `frame.view` or `frame.texture`.
    fn render(&mut self, frame: &GpuFrame);

    /// Called when the surface size changes (before render).
    ///
    /// Default implementation does nothing. Override if you need to
    /// recreate resources when the surface size changes.
    fn resize(&mut self, _width: u32, _height: u32) {}
}

/// Private object-safe trait for type-erased GPU renderers.
trait GpuRendererImpl: 'static {
    fn setup<'a>(&'a mut self, ctx: &'a GpuContext<'a>) -> SetupFuture<'a>;
    fn render(&mut self, frame: &GpuFrame);
    fn resize(&mut self, width: u32, height: u32);
}

impl<T: GpuRenderer> GpuRendererImpl for T {
    fn setup<'a>(&'a mut self, ctx: &'a GpuContext<'a>) -> SetupFuture<'a> {
        Box::pin(GpuRenderer::setup(self, ctx))
    }

    fn render(&mut self, frame: &GpuFrame) {
        GpuRenderer::render(self, frame);
    }

    fn resize(&mut self, width: u32, height: u32) {
        GpuRenderer::resize(self, width, height);
    }
}

/// A raw view for high-performance GPU rendering.
///
/// `GpuSurface` provides direct access to wgpu for custom rendering at
/// display refresh rates (60-120fps+). It stretches to fill available
/// space by default, similar to `SwiftUI`'s `Color`.
///
/// # Layout Behavior
///
/// - Stretches in both directions by default (`StretchAxis::Both`)
/// - Control size using `.frame()` modifier externally
/// - Current size is provided via `GpuFrame.width/height` during rendering
///
/// # Example
///
/// ```ignore
/// // Fill available space
/// GpuSurface::new(MyRenderer::default())
///
/// // Fixed size
/// GpuSurface::new(MyRenderer::default())
///     .frame(width: 400.0, height: 300.0)
/// ```
pub struct GpuSurface {
    /// The renderer that handles GPU drawing (type-erased).
    renderer: Box<dyn GpuRendererImpl>,
}

impl core::fmt::Debug for GpuSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuSurface").finish_non_exhaustive()
    }
}

impl GpuSurface {
    /// Creates a new GPU surface with the provided renderer.
    ///
    /// # Arguments
    ///
    /// * `renderer` - An implementation of `GpuRenderer` that handles setup and rendering.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let surface = GpuSurface::new(MyRenderer::default());
    /// ```
    #[must_use]
    pub fn new<R: GpuRenderer>(renderer: R) -> Self {
        Self {
            renderer: Box::new(renderer),
        }
    }

    /// Calls `setup` on the renderer, returning a future that completes when ready.
    pub fn setup<'a>(&'a mut self, ctx: &'a GpuContext<'a>) -> SetupFuture<'a> {
        self.renderer.setup(ctx)
    }

    /// Calls `render` on the renderer.
    pub fn render(&mut self, frame: &GpuFrame) {
        self.renderer.render(frame);
    }

    /// Calls `resize` on the renderer.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }
}

// Stretches in both directions by default
raw_view!(GpuSurface, StretchAxis::Both);
