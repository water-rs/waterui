//! High-performance GPU rendering surface using wgpu.
//!
//! This module provides `GpuSurface`, a raw view that enables direct wgpu access
//! for custom GPU rendering.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::Future;
use core::num::NonZeroU32;
use core::pin::Pin;
use std::sync::mpsc;

use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_core::{Environment, Native, NativeView, View};

#[doc(hidden)]
pub use waterui_core::layout::{
    ProposalSize as __GpuProposalSize, Size as __GpuSize, StretchAxis as __GpuStretchAxis,
    SubView as __GpuSubView, ViewDimensions as __GpuViewDimensions,
};

/// Internal boxed future for object-safe GPU setup dispatch.
#[doc(hidden)]
pub type SetupFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

/// Picks the best surface format for a [`GpuSurface`].
///
/// `WaterUI` prefers HDR surfaces when available. If the platform/surface does not support an HDR
/// format, it falls back to a standard sRGB swapchain format (or the first supported format).
#[must_use]
pub fn preferred_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    preferred_surface_format_with_preference(caps, None)
}

/// Picks the best surface format for a [`GpuSurface`] with an optional HDR preference override.
///
/// - `Some(true)`: prefer HDR when supported.
/// - `Some(false)`: prefer SDR even if HDR is supported.
/// - `None`: follow `WATERUI_GPU_PREFER_HDR` behavior.
#[must_use]
pub fn surface_hdr_preference_from_env() -> Option<bool> {
    std::env::var("WATERUI_GPU_PREFER_HDR")
        .ok()
        .map(|v| !matches!(v.as_str(), "0" | "false" | "FALSE"))
}

/// Resolves final HDR preference from optional override and environment policy.
///
/// Resolution order:
/// - Explicit override from surface/renderer.
/// - `WATERUI_GPU_PREFER_HDR` when present.
/// - Default `true` (prefer HDR).
#[must_use]
pub fn resolve_surface_hdr_preference(prefer_hdr_override: Option<bool>) -> bool {
    prefer_hdr_override
        .or_else(surface_hdr_preference_from_env)
        .unwrap_or(true)
}

#[must_use]
pub fn preferred_surface_format_with_preference(
    caps: &wgpu::SurfaceCapabilities,
    prefer_hdr_override: Option<bool>,
) -> wgpu::TextureFormat {
    let hdr = wgpu::TextureFormat::Rgba16Float;
    let prefer_hdr = resolve_surface_hdr_preference(prefer_hdr_override);

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
    /// Handle to request redraws from outside `render()`.
    ///
    /// Clone this during `setup()` and call `request_redraw()` when external
    /// data arrives (e.g., nami signal change, timer, network response).
    pub redraw_handle: RedrawHandle,
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

/// A handle that can trigger a redraw of the associated `GpuSurface`.
///
/// Cheap to clone. Thread-safe (`Send + Sync`).
/// Obtain from [`GpuContext::redraw_handle`] during [`GpuView::setup`].
#[derive(Clone, Debug)]
pub struct RedrawHandle {
    dirty: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
}

impl RedrawHandle {
    /// Creates a new redraw handle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dirty: alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Mark the surface as needing a redraw.
    pub fn request_redraw(&self) {
        self.dirty
            .store(true, core::sync::atomic::Ordering::Release);
    }

    /// Check and clear the dirty flag. Returns `true` if a redraw was requested.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, core::sync::atomic::Ordering::AcqRel)
    }
}

impl Default for RedrawHandle {
    fn default() -> Self {
        Self::new()
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
    /// Internal: set to true when `request_redraw()` is called.
    redraw_requested: bool,
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

impl<'a> GpuFrame<'a> {
    /// Creates a frame payload for a single render pass.
    #[must_use]
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        texture: &'a wgpu::Texture,
        view: wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        pointer: PointerState,
        gesture: GestureState,
    ) -> Self {
        Self {
            device,
            queue,
            texture,
            view,
            format,
            width,
            height,
            pointer,
            gesture,
            redraw_requested: false,
        }
    }

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

    /// Request that `render()` be called again on the next frame.
    ///
    /// Use this for animations. If not called, the surface stays idle
    /// until an external event triggers a redraw via [`RedrawHandle`].
    pub fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }

    /// Check if redraw was requested during this render call.
    #[must_use]
    pub const fn was_redraw_requested(&self) -> bool {
        self.redraw_requested
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
/// The `setup` method is async, allowing async initialization (e.g., SVG parsing).
/// For sync renderers, just run setup code directly and return.
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
/// impl GpuView for TriangleRenderer {
///     async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
///         self.pipeline = Some(ctx.device.create_render_pipeline(&...));
///     }
///
///     fn render(&mut self, frame: &mut GpuFrame) {
///         let mut encoder = frame.device.create_command_encoder(&Default::default());
///         // ... render to frame.view ...
///         frame.queue.submit([encoder.finish()]);
///     }
/// }
/// ```
pub trait GpuView: SubView + 'static {
    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, buffers, bind groups, and other
    /// GPU resources that persist across frames.
    ///
    /// `ctx.redraw_handle` can be cloned here for external redraw triggers.
    /// `env` provides access to the WaterUI environment (theme, fonts, etc.).
    ///
    /// Async setup hook for GPU resources.
    #[allow(async_fn_in_trait)]
    async fn setup(&mut self, ctx: &GpuContext<'_>, env: &mut waterui_core::Environment);

    /// Called each frame to render.
    ///
    /// Use `frame.width` and `frame.height` to get the current surface dimensions.
    /// Render into `frame.view` or `frame.texture`.
    ///
    /// Call `frame.request_redraw()` to schedule another frame (for animations).
    fn render(&mut self, frame: &mut GpuFrame);

    /// Optional per-view surface dynamic range preference.
    ///
    /// - `Some(true)`: prefer HDR surface formats.
    /// - `Some(false)`: prefer SDR surface formats.
    /// - `None`: follow global `WATERUI_GPU_PREFER_HDR` behavior.
    ///
    /// This is evaluated by native backends during surface initialization.
    fn preferred_surface_hdr(&self) -> Option<bool> {
        None
    }
}

/// Implements `SubView` for a `GpuView` type using its layout defaults/overrides.
///
/// Use this macro at each concrete `impl GpuView for ...` site.
#[macro_export]
macro_rules! impl_gpu_subview {
    ($ty:ty) => {
        impl $crate::gpu_surface::__GpuSubView for $ty {
            fn measure(
                &self,
                proposal: $crate::gpu_surface::__GpuProposalSize,
            ) -> $crate::gpu_surface::__GpuViewDimensions {
                $crate::gpu_surface::__GpuViewDimensions::new($crate::gpu_surface::__GpuSize::new(
                    proposal.width.unwrap_or(0.0),
                    proposal.height.unwrap_or(0.0),
                ))
            }

            fn stretch_axis(&self) -> $crate::gpu_surface::__GpuStretchAxis {
                $crate::gpu_surface::__GpuStretchAxis::Both
            }

            fn priority(&self) -> i32 {
                0
            }
        }
    };
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
    /// For `render_offscreen`, this supports `Rgba8Unorm` and `Rgba8UnormSrgb`.
    /// For `render_offscreen_hdr`, this supports `Rgba16Float`.
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

/// Output of an HDR offscreen render pass (`RGBA16F` linear pixels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffscreenRenderOutputHdr {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA16F pixel data in row-major order (little-endian half-float components).
    pub rgba16f: Vec<u8>,
}

impl OffscreenRenderOutputHdr {
    /// Returns the maximum linear RGB channel value in the output.
    #[must_use]
    pub fn max_rgb_linear(&self) -> f32 {
        let mut max_rgb = 0.0f32;
        for px in self.rgba16f.chunks_exact(8) {
            let r = f16_to_f32(u16::from_le_bytes([px[0], px[1]]));
            let g = f16_to_f32(u16::from_le_bytes([px[2], px[3]]));
            let b = f16_to_f32(u16::from_le_bytes([px[4], px[5]]));
            max_rgb = max_rgb.max(r.max(g).max(b));
        }
        max_rgb
    }

    /// Returns the fraction of pixels whose RGB has HDR headroom (`> 1.0`).
    #[must_use]
    pub fn hdr_pixel_ratio(&self) -> f32 {
        let mut total = 0usize;
        let mut hdr = 0usize;
        for px in self.rgba16f.chunks_exact(8) {
            let r = f16_to_f32(u16::from_le_bytes([px[0], px[1]]));
            let g = f16_to_f32(u16::from_le_bytes([px[2], px[3]]));
            let b = f16_to_f32(u16::from_le_bytes([px[4], px[5]]));
            if r > 1.0 || g > 1.0 || b > 1.0 {
                hdr += 1;
            }
            total += 1;
        }
        if total == 0 {
            0.0
        } else {
            hdr as f32 / total as f32
        }
    }

    /// Encodes as PNG with automatic dynamic-range handling.
    ///
    /// - If HDR headroom is detected (`RGB > 1.0`), emits a standards-based HDR PNG
    ///   (PQ-coded 16-bit RGBA + `cICP` chunk).
    /// - Otherwise emits SDR PNG (16-bit sRGB transfer).
    pub fn to_png(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_auto_png(self.width, self.height, self.rgba16f.clone())
    }

    /// Encodes PNG with automatic dynamic-range handling without consuming output.
    pub fn into_png(self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_auto_png(self.width, self.height, self.rgba16f)
    }

    /// Encodes as SDR PNG using automatic tone mapping.
    pub fn to_sdr_png(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_sdr_tonemapped_png(self.width, self.height, self.rgba16f.clone())
    }

    /// Encodes as SDR PNG without consuming output.
    pub fn into_sdr_png(self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_sdr_tonemapped_png(self.width, self.height, self.rgba16f)
    }

    /// Converts to SDR `RGBA8` bytes using automatic tone mapping.
    pub fn to_sdr_rgba8(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        decode_sdr_tonemapped_rgba8(self.width, self.height, &self.rgba16f)
    }

    /// Converts to SDR `RGBA8` bytes using automatic tone mapping.
    pub fn into_sdr_rgba8(self) -> Result<Vec<u8>, OffscreenRenderError> {
        decode_sdr_tonemapped_rgba8(self.width, self.height, &self.rgba16f)
    }

    /// Saves the rendered image as PNG with automatic dynamic-range handling.
    pub fn save_png<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), OffscreenRenderError> {
        let png = self.to_png()?;
        std::fs::write(path, png).map_err(|e| OffscreenRenderError::PngWriteFailed(e.to_string()))
    }

    /// Saves the rendered image as SDR PNG using automatic tone mapping.
    pub fn save_sdr_png<P: AsRef<std::path::Path>>(
        &self,
        path: P,
    ) -> Result<(), OffscreenRenderError> {
        let png = self.to_sdr_png()?;
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

/// Private object-safe trait for type-erased GPU views.
trait GpuViewImpl: 'static {
    fn setup<'a>(
        &'a mut self,
        ctx: &'a GpuContext<'a>,
        env: &'a mut waterui_core::Environment,
    ) -> SetupFuture<'a>;
    fn render(&mut self, frame: &mut GpuFrame);
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;
    fn stretch_axis(&self) -> StretchAxis;
    fn priority(&self) -> i32;
    fn require_main_thread(&self) -> bool;
    fn preferred_surface_hdr(&self) -> Option<bool>;
}

impl<T: GpuView> GpuViewImpl for T {
    fn setup<'a>(
        &'a mut self,
        ctx: &'a GpuContext<'a>,
        env: &'a mut waterui_core::Environment,
    ) -> SetupFuture<'a> {
        Box::pin(GpuView::setup(self, ctx, env))
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        GpuView::render(self, frame);
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        SubView::measure(self, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        SubView::stretch_axis(self)
    }

    fn priority(&self) -> i32 {
        SubView::priority(self)
    }

    fn require_main_thread(&self) -> bool {
        SubView::require_main_thread(self)
    }

    fn preferred_surface_hdr(&self) -> Option<bool> {
        GpuView::preferred_surface_hdr(self)
    }
}

/// A raw view for high-performance GPU rendering.
///
/// `GpuSurface` provides direct access to wgpu for custom rendering and uses
/// on-demand scheduling by default.
///
/// Native backends render when the surface is dirty (size/input updates) and
/// keep rendering while `GpuFrame::request_redraw()` (or `RedrawHandle`) asks
/// for another frame.
/// It stretches to fill available space by default (like `SwiftUI`'s `Color`),
/// but renderers can override layout behavior by providing a custom `SubView` implementation.
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
    /// The GPU view that handles rendering (type-erased).
    renderer: Box<dyn GpuViewImpl>,
    /// Preferred maximum MSAA sample count for this surface.
    ///
    /// Backends use this as the cap when selecting a supported sample count.
    msaa_max_samples: NonZeroU32,
    /// Optional per-surface HDR preference override.
    ///
    /// `None` follows global `WATERUI_GPU_PREFER_HDR` behavior.
    /// `Some(true)` prefers HDR, `Some(false)` prefers SDR.
    surface_prefers_hdr: Option<bool>,
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

    /// Creates a new GPU surface with the provided GPU view.
    ///
    /// # Arguments
    ///
    /// * `view` - An implementation of `GpuView` that handles setup and rendering.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let surface = GpuSurface::new(MyRenderer::default());
    /// ```
    #[must_use]
    pub fn new<R: GpuView>(view: R) -> Self {
        Self {
            renderer: Box::new(view),
            msaa_max_samples: Self::default_msaa_max_samples(),
            surface_prefers_hdr: None,
        }
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

    /// Prefer HDR swapchain formats for this surface when available.
    ///
    /// This overrides global `WATERUI_GPU_PREFER_HDR` for this surface only.
    #[must_use]
    pub const fn prefer_hdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(true);
        self
    }

    /// Prefer SDR swapchain formats for this surface even when HDR is available.
    ///
    /// This overrides global `WATERUI_GPU_PREFER_HDR` for this surface only.
    #[must_use]
    pub const fn prefer_sdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(false);
        self
    }

    /// Returns this surface's HDR preference override.
    ///
    /// `None` means follow global environment preference.
    #[must_use]
    pub fn get_surface_prefers_hdr(&self) -> Option<bool> {
        self.surface_prefers_hdr
            .or_else(|| self.renderer.preferred_surface_hdr())
    }

    /// Renders this surface once into an offscreen texture and reads back RGBA8 pixels.
    ///
    /// This is intended for fast visual regression checks and snapshot generation
    /// without launching a full app window.
    pub fn render_offscreen(
        self,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        self.render_offscreen_frames(config, env, NonZeroU32::new(1).expect("non-zero literal"))
    }

    /// Renders this surface into an offscreen texture for `frame_count` frames and reads back RGBA8 pixels.
    ///
    /// This is useful for animated GPU views that need one or more warm-up frames before producing a stable snapshot.
    pub fn render_offscreen_frames(
        mut self,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
        frame_count: NonZeroU32,
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
            redraw_handle: RedrawHandle::new(),
        };
        crate::ready_now_or_panic(
            self.setup(&ctx, env),
            "gpu_surface::render_offscreen::setup",
        );

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
        let mut frame = GpuFrame {
            device,
            queue,
            texture: &texture,
            view,
            format: config.format,
            width,
            height,
            pointer: config.pointer,
            gesture: config.gesture,
            redraw_requested: false,
        };
        for _ in 0..frame_count.get() {
            self.render(&mut frame);
        }

        let rgba8 = readback_texture_rgba8(device, queue, &texture, width, height)?;
        Ok(OffscreenRenderOutput {
            width,
            height,
            rgba8,
        })
    }

    /// Renders this surface into an HDR offscreen texture and reads back `RGBA16F` pixels.
    ///
    /// Use this when you need to preserve HDR headroom during capture/export.
    pub fn render_offscreen_hdr(
        self,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        self.render_offscreen_hdr_frames(config, env, NonZeroU32::new(1).expect("non-zero literal"))
    }

    /// Renders this surface into an HDR offscreen texture for `frame_count` frames and reads back `RGBA16F` pixels.
    pub fn render_offscreen_hdr_frames(
        mut self,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
        frame_count: NonZeroU32,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        if config.format != wgpu::TextureFormat::Rgba16Float {
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
            redraw_handle: RedrawHandle::new(),
        };
        crate::ready_now_or_panic(
            self.setup(&ctx, env),
            "gpu_surface::render_offscreen_hdr::setup",
        );

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("waterui_offscreen_surface_hdr"),
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
        let mut frame = GpuFrame {
            device,
            queue,
            texture: &texture,
            view,
            format: config.format,
            width,
            height,
            pointer: config.pointer,
            gesture: config.gesture,
            redraw_requested: false,
        };
        for _ in 0..frame_count.get() {
            self.render(&mut frame);
        }

        let rgba16f = readback_texture_rgba16f(device, queue, &texture, width, height)?;
        Ok(OffscreenRenderOutputHdr {
            width,
            height,
            rgba16f,
        })
    }

    /// Calls `setup` on the GPU view, returning a future that completes when ready.
    pub fn setup<'a>(
        &'a mut self,
        ctx: &'a GpuContext<'a>,
        env: &'a mut waterui_core::Environment,
    ) -> SetupFuture<'a> {
        self.renderer.setup(ctx, env)
    }

    /// Calls `render` on the GPU view.
    pub fn render(&mut self, frame: &mut GpuFrame) {
        self.renderer.render(frame);
    }

    /// Returns this surface's measured size for the given proposal.
    #[must_use]
    pub fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        self.renderer.measure(proposal).size
    }

    /// Returns this surface's stretch behavior.
    #[must_use]
    pub fn stretch_axis(&self) -> StretchAxis {
        self.renderer.stretch_axis()
    }

    /// Returns this surface's layout priority.
    #[must_use]
    pub fn priority(&self) -> i32 {
        self.renderer.priority()
    }

    /// Returns whether layout measurement requires the main thread.
    #[must_use]
    pub fn require_main_thread(&self) -> bool {
        self.renderer.require_main_thread()
    }
}

impl SubView for GpuSurface {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(self.size_that_fits(proposal))
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.priority()
    }

    fn require_main_thread(&self) -> bool {
        GpuSurface::require_main_thread(self)
    }
}

impl NativeView for GpuSurface {
    fn stretch_axis(&self) -> StretchAxis {
        SubView::stretch_axis(self)
    }
}

impl View for GpuSurface {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(self)
    }
}

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

fn readback_texture_rgba16f(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, OffscreenRenderError> {
    const BYTES_PER_PIXEL: u32 = 8;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bpr = width * BYTES_PER_PIXEL;
    let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
    let copy_size = (padded_bpr * height) as u64;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("waterui_offscreen_readback_hdr"),
        size: copy_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("waterui_offscreen_readback_hdr_encoder"),
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

fn encode_auto_png(
    width: u32,
    height: u32,
    rgba16f: Vec<u8>,
) -> Result<Vec<u8>, OffscreenRenderError> {
    let (max_rgb, hdr_ratio) = analyze_hdr_headroom(&rgba16f);
    if max_rgb > 1.0 && hdr_ratio > 0.0 {
        encode_hdr_pq_png(width, height, rgba16f)
    } else {
        encode_sdr_png_linear(width, height, rgba16f)
    }
}

fn encode_sdr_tonemapped_png(
    width: u32,
    height: u32,
    rgba16f: Vec<u8>,
) -> Result<Vec<u8>, OffscreenRenderError> {
    let expected = validate_rgba16f_buffer(width, height, &rgba16f)?;
    let white_point = compute_tonemap_white_point(&rgba16f);
    let png16 = rgba16f_to_sdr_srgb16_bytes(&rgba16f, white_point, expected);
    encode_png16(width, height, &png16, None)
}

fn decode_sdr_tonemapped_rgba8(
    width: u32,
    height: u32,
    rgba16f: &[u8],
) -> Result<Vec<u8>, OffscreenRenderError> {
    let _ = validate_rgba16f_buffer(width, height, rgba16f)?;
    if rgba16f.is_empty() || width == 0 || height == 0 {
        return Ok(Vec::new());
    }
    let white_point = compute_tonemap_white_point(rgba16f);
    Ok(rgba16f_to_sdr_srgb8_bytes(
        rgba16f,
        white_point,
        (width as usize) * (height as usize) * 4,
    ))
}

fn encode_sdr_png_linear(
    width: u32,
    height: u32,
    rgba16f: Vec<u8>,
) -> Result<Vec<u8>, OffscreenRenderError> {
    let expected = validate_rgba16f_buffer(width, height, &rgba16f)?;
    let png16 = rgba16f_to_sdr_srgb16_bytes(&rgba16f, 1.0, expected);
    encode_png16(width, height, &png16, None)
}

fn encode_hdr_pq_png(
    width: u32,
    height: u32,
    rgba16f: Vec<u8>,
) -> Result<Vec<u8>, OffscreenRenderError> {
    let expected = validate_rgba16f_buffer(width, height, &rgba16f)?;
    // Treat linear 1.0 as HDR reference white (203 nits), then encode absolute PQ.
    // This keeps >1.0 headroom while producing standards-based HDR signaling.
    const SDR_WHITE_NITS: f32 = 203.0;
    let mut png16 = Vec::with_capacity(expected);
    for px in rgba16f.chunks_exact(8) {
        let r = linear_to_pq(
            (f16_to_f32(u16::from_le_bytes([px[0], px[1]])).max(0.0)) * SDR_WHITE_NITS,
        );
        let g = linear_to_pq(
            (f16_to_f32(u16::from_le_bytes([px[2], px[3]])).max(0.0)) * SDR_WHITE_NITS,
        );
        let b = linear_to_pq(
            (f16_to_f32(u16::from_le_bytes([px[4], px[5]])).max(0.0)) * SDR_WHITE_NITS,
        );
        let a = f16_to_f32(u16::from_le_bytes([px[6], px[7]])).clamp(0.0, 1.0);
        let r16 = (r * 65535.0).round() as u16;
        let g16 = (g * 65535.0).round() as u16;
        let b16 = (b * 65535.0).round() as u16;
        let a16 = (a * 65535.0).round() as u16;
        png16.extend_from_slice(&r16.to_be_bytes());
        png16.extend_from_slice(&g16.to_be_bytes());
        png16.extend_from_slice(&b16.to_be_bytes());
        png16.extend_from_slice(&a16.to_be_bytes());
    }

    // PNG cICP: BT.2020 primaries + PQ transfer + RGB matrix + full-range.
    // This is the broadest interoperable HDR signaling baseline for offscreen exports.
    let cicp = [9u8, 16u8, 0u8, 1u8];
    encode_png16(width, height, &png16, Some(&cicp))
}

fn validate_rgba16f_buffer(
    width: u32,
    height: u32,
    rgba16f: &[u8],
) -> Result<usize, OffscreenRenderError> {
    if rgba16f.is_empty() || width == 0 || height == 0 {
        return Ok(0);
    }
    let expected = width as usize * height as usize * 8;
    if rgba16f.len() != expected {
        return Err(OffscreenRenderError::PngEncodingFailed(format!(
            "invalid RGBA16F buffer size: expected {expected}, got {}",
            rgba16f.len()
        )));
    }
    Ok(expected)
}

fn encode_png16(
    width: u32,
    height: u32,
    rgba16_be: &[u8],
    cicp: Option<&[u8; 4]>,
) -> Result<Vec<u8>, OffscreenRenderError> {
    if rgba16_be.is_empty() || width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    let mut png_bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::Adaptive);
    {
        let mut writer = encoder
            .write_header()
            .map_err(|e| OffscreenRenderError::PngEncodingFailed(e.to_string()))?;
        if let Some(cicp_bytes) = cicp {
            writer
                .write_chunk(png::chunk::cICP, cicp_bytes)
                .map_err(|e| OffscreenRenderError::PngEncodingFailed(e.to_string()))?;
        }
        writer
            .write_image_data(rgba16_be)
            .map_err(|e| OffscreenRenderError::PngEncodingFailed(e.to_string()))?;
    }
    Ok(png_bytes)
}

fn rgba16f_to_sdr_srgb16_bytes(rgba16f: &[u8], white_point: f32, capacity: usize) -> Vec<u8> {
    let white_point = white_point.max(1.0);
    let mut png16 = Vec::with_capacity(capacity);
    for px in rgba16f.chunks_exact(8) {
        let r = linear_to_srgb(
            (f16_to_f32(u16::from_le_bytes([px[0], px[1]])).max(0.0) / white_point).min(1.0),
        );
        let g = linear_to_srgb(
            (f16_to_f32(u16::from_le_bytes([px[2], px[3]])).max(0.0) / white_point).min(1.0),
        );
        let b = linear_to_srgb(
            (f16_to_f32(u16::from_le_bytes([px[4], px[5]])).max(0.0) / white_point).min(1.0),
        );
        let a = f16_to_f32(u16::from_le_bytes([px[6], px[7]])).clamp(0.0, 1.0);
        let r16 = (r * 65535.0).round() as u16;
        let g16 = (g * 65535.0).round() as u16;
        let b16 = (b * 65535.0).round() as u16;
        let a16 = (a * 65535.0).round() as u16;
        png16.extend_from_slice(&r16.to_be_bytes());
        png16.extend_from_slice(&g16.to_be_bytes());
        png16.extend_from_slice(&b16.to_be_bytes());
        png16.extend_from_slice(&a16.to_be_bytes());
    }
    png16
}

fn rgba16f_to_sdr_srgb8_bytes(rgba16f: &[u8], white_point: f32, capacity: usize) -> Vec<u8> {
    let white_point = white_point.max(1.0);
    let mut rgba8 = Vec::with_capacity(capacity);
    for px in rgba16f.chunks_exact(8) {
        let r = linear_to_srgb(
            (f16_to_f32(u16::from_le_bytes([px[0], px[1]])).max(0.0) / white_point).min(1.0),
        );
        let g = linear_to_srgb(
            (f16_to_f32(u16::from_le_bytes([px[2], px[3]])).max(0.0) / white_point).min(1.0),
        );
        let b = linear_to_srgb(
            (f16_to_f32(u16::from_le_bytes([px[4], px[5]])).max(0.0) / white_point).min(1.0),
        );
        let a = f16_to_f32(u16::from_le_bytes([px[6], px[7]])).clamp(0.0, 1.0);
        rgba8.push((r * 255.0).round() as u8);
        rgba8.push((g * 255.0).round() as u8);
        rgba8.push((b * 255.0).round() as u8);
        rgba8.push((a * 255.0).round() as u8);
    }
    rgba8
}

fn compute_tonemap_white_point(rgba16f: &[u8]) -> f32 {
    let mut frame_max = Vec::with_capacity(rgba16f.len() / 8);
    for px in rgba16f.chunks_exact(8) {
        let r = f16_to_f32(u16::from_le_bytes([px[0], px[1]])).max(0.0);
        let g = f16_to_f32(u16::from_le_bytes([px[2], px[3]])).max(0.0);
        let b = f16_to_f32(u16::from_le_bytes([px[4], px[5]])).max(0.0);
        frame_max.push(r.max(g).max(b));
    }
    if frame_max.is_empty() {
        return 1.0;
    }
    frame_max.sort_unstable_by(|a, b| a.total_cmp(b));
    let idx = (((frame_max.len() - 1) as f32) * 0.995).round() as usize;
    frame_max[idx].max(1.0)
}

fn analyze_hdr_headroom(rgba16f: &[u8]) -> (f32, f32) {
    let mut max_rgb = 0.0f32;
    let mut total = 0usize;
    let mut hdr = 0usize;
    for px in rgba16f.chunks_exact(8) {
        let r = f16_to_f32(u16::from_le_bytes([px[0], px[1]])).max(0.0);
        let g = f16_to_f32(u16::from_le_bytes([px[2], px[3]])).max(0.0);
        let b = f16_to_f32(u16::from_le_bytes([px[4], px[5]])).max(0.0);
        let m = r.max(g).max(b);
        max_rgb = max_rgb.max(m);
        if m > 1.0 {
            hdr += 1;
        }
        total += 1;
    }
    let ratio = if total == 0 {
        0.0
    } else {
        hdr as f32 / total as f32
    };
    (max_rgb, ratio)
}

#[inline]
fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
fn linear_to_pq(luminance_nits: f32) -> f32 {
    // ST-2084 PQ OETF, input in nits (0..10000).
    let y = (luminance_nits / 10_000.0).clamp(0.0, 1.0);
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 32.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 128.0;
    let c3 = 2392.0 / 128.0;
    let ym1 = y.powf(m1);
    let num = c1 + c2 * ym1;
    let den = 1.0 + c3 * ym1;
    (num / den).powf(m2)
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 0x1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let frac = (bits & 0x03ff) as u32;

    let f32_bits = if exp == 0 {
        if frac == 0 {
            sign << 31
        } else {
            let mut frac_norm = frac;
            let mut e = -14i32;
            while (frac_norm & 0x0400) == 0 {
                frac_norm <<= 1;
                e -= 1;
            }
            frac_norm &= 0x03ff;
            let exp32 = (e + 127) as u32;
            (sign << 31) | (exp32 << 23) | (frac_norm << 13)
        }
    } else if exp == 0x1f {
        (sign << 31) | 0x7f80_0000 | (frac << 13)
    } else {
        let exp32 = (exp as i32 - 15 + 127) as u32;
        (sign << 31) | (exp32 << 23) | (frac << 13)
    };
    f32::from_bits(f32_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba16f_pixel_le(r: u16, g: u16, b: u16, a: u16) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&r.to_le_bytes());
        bytes.extend_from_slice(&g.to_le_bytes());
        bytes.extend_from_slice(&b.to_le_bytes());
        bytes.extend_from_slice(&a.to_le_bytes());
        bytes
    }

    #[test]
    fn hdr_output_to_sdr_rgba8_preserves_sdr_values() {
        // half-float: 0.5, 0.5, 0.5, 1.0
        let output = OffscreenRenderOutputHdr {
            width: 1,
            height: 1,
            rgba16f: rgba16f_pixel_le(0x3800, 0x3800, 0x3800, 0x3c00),
        };

        let rgba8 = output
            .to_sdr_rgba8()
            .expect("sdr conversion should succeed");
        assert_eq!(rgba8, vec![188, 188, 188, 255]);
    }

    #[test]
    fn hdr_output_to_sdr_rgba8_tonemaps_hdr_values() {
        // half-float: 2.0, 2.0, 2.0, 1.0
        let output = OffscreenRenderOutputHdr {
            width: 1,
            height: 1,
            rgba16f: rgba16f_pixel_le(0x4000, 0x4000, 0x4000, 0x3c00),
        };

        let rgba8 = output
            .to_sdr_rgba8()
            .expect("hdr tone mapping should succeed");
        assert_eq!(rgba8, vec![255, 255, 255, 255]);
    }
}
