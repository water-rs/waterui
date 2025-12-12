//! High-performance GPU rendering surface using wgpu.
//!
//! This module provides `GpuSurface`, a raw view that enables direct wgpu access
//! for custom GPU rendering at up to 120fps+.

extern crate alloc;

use alloc::boxed::Box;

use waterui_core::{layout::StretchAxis, raw_view};

/// Picks the best surface format for a [`GpuSurface`].
///
/// WaterUI prefers HDR surfaces when available. If the platform/surface does not support an HDR
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
/// # Example
///
/// ```ignore
/// struct TriangleRenderer {
///     pipeline: Option<wgpu::RenderPipeline>,
/// }
///
/// impl GpuRenderer for TriangleRenderer {
///     fn setup(&mut self, ctx: &GpuContext) {
///         // Create pipeline, buffers, etc.
///         self.pipeline = Some(ctx.device.create_render_pipeline(&...));
///     }
///
///     fn render(&mut self, frame: &GpuFrame) {
///         let mut encoder = frame.device.create_command_encoder(&Default::default());
///         // ... render to frame.view ...
///         frame.queue.submit([encoder.finish()]);
///     }
/// }
/// ```
pub trait GpuRenderer: Send + 'static {
    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, buffers, bind groups, and other
    /// GPU resources that persist across frames.
    fn setup(&mut self, ctx: &GpuContext);

    /// Called each frame to render.
    ///
    /// Use `frame.width` and `frame.height` to get the current surface dimensions.
    /// Render into `frame.view` or `frame.texture`.
    fn render(&mut self, frame: &GpuFrame);

    /// Called when the surface size changes (before render).
    ///
    /// Override this to recreate size-dependent resources like
    /// depth buffers or render targets.
    fn resize(&mut self, _width: u32, _height: u32) {}
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
    /// The renderer that handles GPU drawing.
    pub renderer: Box<dyn GpuRenderer>,
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
}

// Stretches in both directions by default, like SwiftUI's Color
raw_view!(GpuSurface, StretchAxis::Both);
