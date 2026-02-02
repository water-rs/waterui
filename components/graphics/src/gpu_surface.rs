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
    let hdr = wgpu::TextureFormat::Rgba16Float;
    let prefer_hdr = if cfg!(target_os = "android") {
        std::env::var("WATERUI_GPU_PREFER_HDR")
            .ok()
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
    } else {
        true
    };

    // HDR (linear, extended range) preferred when supported by the surface.
    if prefer_hdr && caps.formats.contains(&hdr) {
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

    // HDR as a fallback (when sRGB is unavailable).
    if caps.formats.contains(&hdr) {
        return hdr;
    }

    // Fallback: use the first reported format.
    caps.formats
        .first()
        .copied()
        .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb)
}

/// Picks a preferred MSAA sample count for a given format, capped by `max_samples`.
///
/// This uses adapter-reported format features (WebGPU-compatible) and falls back to 1
/// if multisampling isn't supported for that format on the current backend.
#[must_use]
pub fn preferred_msaa_samples(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    max_samples: u32,
) -> u32 {
    let max_samples = max_samples.max(1);
    let features = adapter.get_texture_format_features(format);
    for sample_count in [16u32, 8, 4, 2, 1] {
        if sample_count <= max_samples && features.flags.sample_count_supported(sample_count) {
            return sample_count;
        }
    }
    1
}

/// GPU resources provided to the renderer during setup.
///
/// Contains references to the wgpu device, queue, and surface format
/// that the renderer can use to create pipelines, buffers, and other resources.
pub struct GpuContext<'a> {
    /// The GPU adapter used to create the device/queue.
    ///
    /// This is optional because some backends may not keep an adapter handle around.
    /// When present, renderers can use it for format capability queries.
    pub adapter: Option<&'a wgpu::Adapter>,
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The texture format of the surface.
    pub surface_format: wgpu::TextureFormat,
    /// Preferred MSAA sample count for `surface_format`.
    ///
    /// Backends should set this based on `adapter.get_texture_format_features(surface_format)`.
    /// Renderers that want MSAA should use this value for pipeline and attachments.
    pub msaa_samples: u32,
    /// Optional pipeline cache for faster pipeline creation.
    pub pipeline_cache: Option<&'a wgpu::PipelineCache>,
}

impl core::fmt::Debug for GpuContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuContext")
            .field("surface_format", &self.surface_format)
            .field("msaa_samples", &self.msaa_samples)
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

/// Pointer/cursor state for GPU surfaces.
///
/// Provides information about the current pointer position and press state,
/// enabling GPU renderers to implement hover effects, hit detection, and
/// interactive feedback directly in shaders.
#[derive(Debug, Clone, Copy, Default)]
pub struct PointerState {
    /// Current pointer position in surface-local coordinates (pixels).
    /// `None` if the pointer is not over this surface.
    pub position: Option<waterui_core::layout::Point>,
    /// Position where the current hit (press/touch) started.
    /// `None` if not currently pressed. Use `hit.is_some()` to check press state.
    pub hit: Option<waterui_core::layout::Point>,
}

impl PointerState {
    /// Returns the normalized position (0.0 to 1.0) within the given dimensions.
    /// Returns `None` if there is no active pointer position.
    #[must_use]
    pub fn normalized(&self, width: u32, height: u32) -> Option<(f32, f32)> {
        self.position
            .map(|p| (p.x / width as f32, p.y / height as f32))
    }

    /// Returns `true` if the pointer is hovering over this surface.
    #[must_use]
    pub const fn is_hovering(&self) -> bool {
        self.position.is_some()
    }
}

/// Gesture state for interactive GPU surfaces.
///
/// Tracks multi-touch gestures like pinch-to-zoom, pan/drag, and double-tap.
/// Native backends update this state based on platform gesture recognizers.
#[derive(Debug, Clone, Copy, Default)]
pub struct GestureState {
    /// Cumulative pinch scale factor (1.0 = no scaling).
    /// Updated continuously during pinch gestures.
    pub pinch_scale: f32,
    /// Center point of the pinch gesture in surface-local pixels.
    /// `None` if no pinch gesture is active.
    pub pinch_center: Option<waterui_core::layout::Point>,
    /// Pan/drag offset in pixels since gesture began.
    pub pan_offset: waterui_core::layout::Point,
    /// Whether a double-tap was detected this frame.
    pub double_tap: bool,
    /// Whether a gesture is currently in progress.
    pub active: bool,
}

impl GestureState {
    /// Creates a new gesture state with default values (no active gesture).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pinch_scale: 1.0,
            pinch_center: None,
            pan_offset: waterui_core::layout::Point::new(0.0, 0.0),
            double_tap: false,
            active: false,
        }
    }

    /// Returns `true` if a pinch gesture is active.
    #[must_use]
    pub const fn is_pinching(&self) -> bool {
        self.pinch_center.is_some()
    }

    /// Returns `true` if the user is panning.
    #[must_use]
    pub fn is_panning(&self) -> bool {
        self.active && (self.pan_offset.x != 0.0 || self.pan_offset.y != 0.0)
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
    /// Pointer/cursor state for this frame.
    ///
    /// Use this to implement hover effects, hit detection, and interactive
    /// feedback in your renderer. The position is in surface-local pixel
    /// coordinates.
    pub pointer: PointerState,
    /// Gesture state for this frame.
    ///
    /// Use this to implement zoom/pan interactions. The native backend
    /// updates this based on platform gesture recognizers.
    pub gesture: GestureState,
}

impl core::fmt::Debug for GpuFrame<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuFrame")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pointer", &self.pointer)
            .field("gesture", &self.gesture)
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

    /// Returns the normalized pointer position (0.0 to 1.0).
    /// Returns `None` if the pointer is not over this surface.
    #[must_use]
    pub fn pointer_normalized(&self) -> Option<(f32, f32)> {
        self.pointer.normalized(self.width, self.height)
    }

    /// Returns `true` if the pointer is hovering over this surface.
    #[must_use]
    pub const fn is_hovering(&self) -> bool {
        self.pointer.is_hovering()
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

/// Rendering mode for a `GpuSurface`.
///
/// - `Continuous`: render at display refresh rate (for time-based animations like flames).
/// - `OnDemand`: render only when inputs change (pointer/gesture/size or explicit invalidation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuSurfaceRenderMode {
    /// Render continuously (vsync-driven).
    Continuous = 0,
    /// Render only when the surface is marked dirty by the native backend.
    OnDemand = 1,
}

impl Default for GpuSurfaceRenderMode {
    fn default() -> Self {
        Self::Continuous
    }
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
    /// Whether this surface should render continuously or only on demand.
    render_mode: GpuSurfaceRenderMode,
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
            render_mode: GpuSurfaceRenderMode::Continuous,
        }
    }

    /// Sets the render mode for this surface.
    #[must_use]
    pub const fn render_mode(mut self, mode: GpuSurfaceRenderMode) -> Self {
        self.render_mode = mode;
        self
    }

    /// Render at display refresh rate (use for time-based animations).
    #[must_use]
    pub const fn continuous(self) -> Self {
        self.render_mode(GpuSurfaceRenderMode::Continuous)
    }

    /// Render only when the native backend marks the surface dirty.
    #[must_use]
    pub const fn on_demand(self) -> Self {
        self.render_mode(GpuSurfaceRenderMode::OnDemand)
    }

    /// Returns the current render mode.
    #[must_use]
    pub const fn get_render_mode(&self) -> GpuSurfaceRenderMode {
        self.render_mode
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
