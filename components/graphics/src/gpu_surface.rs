//! High-performance GPU rendering surface using wgpu.
//!
//! This module provides `GpuSurface`, a raw view that enables direct wgpu access
//! for custom GPU rendering at up to 120fps+.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::Future;
use core::num::NonZeroU32;
use core::pin::Pin;
use std::sync::mpsc;

use waterui_core::{layout::StretchAxis, raw_view};

use crate::gpu_view::GpuView;

/// A boxed future for async setup operations.
pub type SetupFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

/// Picks the best surface format for a [`GpuSurface`].
///
/// `WaterUI` prefers HDR surfaces when available. If the platform/surface does not support an HDR
/// format, it falls back to a standard sRGB swapchain format (or the first supported format).
#[must_use]
pub fn preferred_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    let hdr = wgpu::TextureFormat::Rgba16Float;
    // Default to HDR across all platforms. Users can explicitly opt out with:
    // WATERUI_GPU_PREFER_HDR=0|false|FALSE
    let prefer_hdr = std::env::var("WATERUI_GPU_PREFER_HDR")
        .ok()
        .is_none_or(|v| !matches!(v.as_str(), "0" | "false" | "FALSE"));

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
/// `GpuSurface` automatically listens to gestures routed through itself and
/// native backends forward the resulting snapshot to the renderer each frame.
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
    /// Use this to implement zoom/pan interactions. `GpuSurface` automatically
    /// forwards gestures routed through it as a per-frame snapshot.
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

/// Strongly-typed non-zero dimensions for offscreen rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffscreenSize {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl OffscreenSize {
    /// Creates a new offscreen size from validated non-zero dimensions.
    #[must_use]
    pub const fn new(width: NonZeroU32, height: NonZeroU32) -> Self {
        Self { width, height }
    }

    /// Creates a new offscreen size from raw pixel dimensions.
    pub fn try_from_pixels(width: u32, height: u32) -> Result<Self, OffscreenRenderError> {
        let (raw_width, raw_height) = (width, height);
        let Some(width) = NonZeroU32::new(width) else {
            return Err(OffscreenRenderError::InvalidSize {
                width: raw_width,
                height: raw_height,
            });
        };
        let Some(height) = NonZeroU32::new(height) else {
            return Err(OffscreenRenderError::InvalidSize {
                width: raw_width,
                height: raw_height,
            });
        };
        Ok(Self { width, height })
    }

    /// Returns width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    /// Returns height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

/// Configuration for one-shot offscreen rendering.
#[derive(Debug, Clone, Copy)]
pub struct OffscreenRenderConfig {
    /// Offscreen output size in pixels.
    pub size: OffscreenSize,
    /// Render target texture format.
    ///
    /// For readback this currently supports `Rgba8Unorm` and `Rgba8UnormSrgb`.
    pub format: wgpu::TextureFormat,
    /// Optional explicit MSAA sample count. `None` means auto-select.
    pub msaa_samples: Option<NonZeroU32>,
    /// Pointer state snapshot used for hover/pressed shader behavior.
    pub pointer: PointerState,
    /// Gesture state snapshot used for zoom/pan renderers.
    pub gesture: GestureState,
}

impl Default for OffscreenRenderConfig {
    fn default() -> Self {
        Self {
            size: OffscreenSize::try_from_pixels(1024, 768)
                .expect("static offscreen defaults must be non-zero"),
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            msaa_samples: None,
            pointer: PointerState::default(),
            gesture: GestureState::new(),
        }
    }
}

impl OffscreenRenderConfig {
    /// Creates config with the given size and default settings.
    #[must_use]
    pub fn new(size: OffscreenSize) -> Self {
        Self {
            size,
            ..Self::default()
        }
    }

    /// Sets output texture format.
    #[must_use]
    pub const fn format(mut self, format: wgpu::TextureFormat) -> Self {
        self.format = format;
        self
    }

    /// Sets fixed MSAA sample count.
    #[must_use]
    pub const fn msaa_samples(mut self, samples: NonZeroU32) -> Self {
        self.msaa_samples = Some(samples);
        self
    }

    /// Sets pointer snapshot used by the offscreen frame.
    #[must_use]
    pub const fn pointer(mut self, pointer: PointerState) -> Self {
        self.pointer = pointer;
        self
    }

    /// Sets gesture snapshot used by the offscreen frame.
    #[must_use]
    pub const fn gesture(mut self, gesture: GestureState) -> Self {
        self.gesture = gesture;
        self
    }
}

/// Output of an offscreen render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffscreenRenderOutput {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 pixel data in row-major order.
    pub rgba8: Vec<u8>,
}

impl OffscreenRenderOutput {
    /// Encodes the RGBA data as a PNG byte buffer.
    pub fn into_png(self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_png(self.width, self.height, self.rgba8)
    }

    /// Encodes the RGBA data as PNG without consuming output.
    pub fn to_png(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_png(self.width, self.height, self.rgba8.clone())
    }

    /// Saves the rendered image as a PNG file.
    pub fn save_png<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), OffscreenRenderError> {
        let png = self.to_png()?;
        std::fs::write(path, png).map_err(|e| OffscreenRenderError::PngWriteFailed(e.to_string()))
    }
}

/// Errors produced by offscreen rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffscreenRenderError {
    /// Width and height must both be non-zero.
    InvalidSize {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
    },
    /// Texture format is unsupported for RGBA8 readback.
    UnsupportedReadbackFormat(wgpu::TextureFormat),
    /// Requested fixed MSAA sample count is unsupported for format/adapter.
    UnsupportedMsaaSamples {
        /// Requested sample count.
        requested: u32,
        /// Target texture format.
        format: wgpu::TextureFormat,
    },
    /// Shared GPU context initialization failed.
    SharedContextInitFailed(String),
    /// GPU readback buffer mapping failed.
    ReadbackMapFailed(String),
    /// Mapping completion channel unexpectedly closed.
    ReadbackChannelClosed,
    /// PNG encoding failed.
    PngEncodingFailed(String),
    /// Writing PNG to disk failed.
    PngWriteFailed(String),
}

impl core::fmt::Display for OffscreenRenderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSize { width, height } => {
                write!(f, "offscreen size must be non-zero, got {width}x{height}")
            }
            Self::UnsupportedReadbackFormat(format) => {
                write!(f, "unsupported offscreen readback format: {format:?}")
            }
            Self::UnsupportedMsaaSamples { requested, format } => {
                write!(
                    f,
                    "unsupported MSAA sample count {requested} for format {format:?}"
                )
            }
            Self::SharedContextInitFailed(error) => {
                write!(f, "failed to initialize GPU context: {error}")
            }
            Self::ReadbackMapFailed(error) => write!(f, "GPU readback mapping failed: {error}"),
            Self::ReadbackChannelClosed => write!(f, "GPU readback channel closed"),
            Self::PngEncodingFailed(error) => write!(f, "PNG encoding failed: {error}"),
            Self::PngWriteFailed(error) => write!(f, "PNG write failed: {error}"),
        }
    }
}

impl std::error::Error for OffscreenRenderError {}

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
    /// Preferred maximum MSAA sample count for this surface.
    ///
    /// Backends use this as the cap when selecting a supported sample count.
    msaa_max_samples: NonZeroU32,
}

impl core::fmt::Debug for GpuSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuSurface").finish_non_exhaustive()
    }
}

impl GpuSurface {
    fn default_msaa_max_samples() -> NonZeroU32 {
        const FALLBACK: NonZeroU32 = NonZeroU32::new(4).expect("non-zero literal");
        let requested = std::env::var("WATERUI_GPU_MSAA")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok());
        let samples = match requested.unwrap_or(4) {
            1 => 1,
            2 => 2,
            4 => 4,
            8 => 8,
            16 => 16,
            _ => FALLBACK.get(),
        };
        NonZeroU32::new(samples).unwrap_or(FALLBACK)
    }

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
            msaa_max_samples: Self::default_msaa_max_samples(),
        }
    }

    /// Creates a new GPU surface from a [`GpuView`].
    ///
    /// This allows environment-aware renderer construction.
    #[must_use]
    pub fn from_gpu_view<V: GpuView>(view: V, env: &waterui_core::Environment) -> Self {
        let mut env = env.clone();
        Self::new(view.gpu_body(&mut env))
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

    /// Sets the preferred maximum MSAA sample count for this surface.
    ///
    /// Backends still clamp this to adapter/format-supported values.
    #[must_use]
    pub const fn msaa_max_samples(mut self, samples: NonZeroU32) -> Self {
        self.msaa_max_samples = samples;
        self
    }

    /// Returns the preferred maximum MSAA sample count for this surface.
    #[must_use]
    pub const fn get_msaa_max_samples(&self) -> NonZeroU32 {
        self.msaa_max_samples
    }

    /// Renders this surface once into an offscreen texture and reads back RGBA8 pixels.
    ///
    /// This is intended for fast visual regression checks and snapshot generation
    /// without launching a full app window.
    pub fn render_offscreen(
        mut self,
        config: OffscreenRenderConfig,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        if !matches!(
            config.format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
        ) {
            return Err(OffscreenRenderError::UnsupportedReadbackFormat(
                config.format,
            ));
        }

        crate::shared_context::init_shared_context()
            .map_err(|e| OffscreenRenderError::SharedContextInitFailed(e.to_string()))?;
        let shared = crate::shared_context::shared_context();
        let guard = shared.read();

        let width = config.size.width();
        let height = config.size.height();
        let adapter = &guard.adapter;
        let max_msaa = config
            .msaa_samples
            .map_or(self.msaa_max_samples.get(), NonZeroU32::get)
            .max(1);
        let supported_msaa = preferred_msaa_samples(adapter, config.format, max_msaa);
        let msaa_samples = match config.msaa_samples {
            Some(requested) if requested.get() != supported_msaa => {
                return Err(OffscreenRenderError::UnsupportedMsaaSamples {
                    requested: requested.get(),
                    format: config.format,
                });
            }
            Some(requested) => requested.get(),
            None => supported_msaa,
        };

        let device = guard.device.as_ref();
        let queue = guard.queue.as_ref();

        let ctx = GpuContext {
            adapter: Some(adapter),
            device,
            queue,
            surface_format: config.format,
            msaa_samples,
            pipeline_cache: guard.pipeline_cache.as_ref(),
        };
        crate::pollster::block_on(self.setup(&ctx));
        self.resize(width, height);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("waterui_offscreen_surface"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let frame = GpuFrame {
            device,
            queue,
            texture: &texture,
            view,
            format: config.format,
            width,
            height,
            pointer: config.pointer,
            gesture: config.gesture,
        };
        self.render(&frame);

        let rgba8 = readback_texture_rgba8(device, queue, &texture, width, height)?;
        Ok(OffscreenRenderOutput {
            width,
            height,
            rgba8,
        })
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

fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, OffscreenRenderError> {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bpr = width * BYTES_PER_PIXEL;
    let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
    let copy_size = (padded_bpr * height) as u64;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("waterui_offscreen_readback"),
        size: copy_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("waterui_offscreen_readback_encoder"),
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
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let map_result = rx
        .recv()
        .map_err(|_| OffscreenRenderError::ReadbackChannelClosed)?;
    map_result.map_err(|e| OffscreenRenderError::ReadbackMapFailed(e.to_string()))?;

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
    Ok(out)
}

fn encode_png(
    width: u32,
    height: u32,
    rgba_data: Vec<u8>,
) -> Result<Vec<u8>, OffscreenRenderError> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba};

    if rgba_data.is_empty() || width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    let Some(img): Option<ImageBuffer<Rgba<u8>, _>> =
        ImageBuffer::from_raw(width, height, rgba_data)
    else {
        return Err(OffscreenRenderError::PngEncodingFailed(
            "invalid RGBA buffer size".to_string(),
        ));
    };

    let mut png_bytes = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut png_bytes, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(img.as_raw(), width, height, ExtendedColorType::Rgba8)
        .map_err(|e| OffscreenRenderError::PngEncodingFailed(e.to_string()))?;
    Ok(png_bytes)
}
