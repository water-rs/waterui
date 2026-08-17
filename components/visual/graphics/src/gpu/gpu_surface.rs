//! High-performance GPU rendering surface using wgpu.
//!
//! This module provides `GpuSurface`, a raw view that enables direct wgpu access
//! for custom GPU rendering.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::num::NonZeroU32;
use core::pin::Pin;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;
use futures::channel::oneshot;
use num_traits::ToPrimitive;
pub use shaderloom::WgslModuleCache;
use parking_lot::RwLock;
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_core::{Environment, MainThreadBound, Native, NativeView, View};

use crate::shared_context::GpuRuntime;

type ErasedSetupFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

/// Picks the best surface format for a [`GpuSurface`].
///
/// `WaterUI` prefers HDR surfaces when available. If the platform/surface does not support an HDR
/// format, it falls back to a standard sRGB swapchain format (or the first supported format).
///
/// # Panics
///
/// Panics when the surface reports no supported formats.
#[must_use]
pub fn preferred_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    preferred_surface_format_with_preference(caps, true)
}

/// Picks the best supported surface format for the platform-resolved HDR preference.
///
/// `WaterUI` prefers HDR surfaces when requested and supported, otherwise it
/// prefers an sRGB SDR format for correct UI compositing.
///
/// # Panics
///
/// Panics when the surface reports no supported formats.
#[must_use]
pub fn preferred_surface_format_with_preference(
    caps: &wgpu::SurfaceCapabilities,
    prefer_hdr: bool,
) -> wgpu::TextureFormat {
    let hdr = wgpu::TextureFormat::Rgba16Float;

    if prefer_hdr && caps.formats.contains(&hdr) {
        return hdr;
    }

    caps.formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| caps.formats.first().copied())
        .expect("surface must report at least one supported format")
}

/// Picks a preferred MSAA sample count for a given format, capped by `max_samples`.
///
/// This uses adapter-reported format features (WebGPU-compatible) and falls back to 1
/// if multisampling isn't supported for that format on the current backend.
#[must_use]
pub fn preferred_msaa_samples(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    max_samples: NonZeroU32,
) -> u32 {
    let max_samples = max_samples.get();
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
    pub adapter: &'a wgpu::Adapter,
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The texture format of the surface.
    pub surface_format: wgpu::TextureFormat,
    /// Module cache for shaders this surface assembles at runtime.
    ///
    /// Owned by the host alongside the device, so two surfaces producing
    /// byte-identical WGSL compile it once.
    pub shader_cache: &'a WgslModuleCache,
    /// The scene renderer shared by every vector-drawing view on this device.
    ///
    /// Owned by the host alongside the device for the same reason as
    /// `shader_cache`: a renderer's pipelines depend on the device, not on what
    /// is drawn with them, so a dozen icons share one rather than each building
    /// its own and reaching a first frame separately.
    pub scene_renderer: &'a Arc<crate::gpu::shared_context::SharedSceneRenderer>,
    /// Preferred MSAA sample count for `surface_format`.
    ///
    /// Prefer [`GpuContext::new`], which derives this from the adapter's
    /// reported format features instead of leaving each backend to repeat
    /// the same query.
    pub msaa_samples: u32,
    /// Handle to request redraws from outside `render()`.
    ///
    /// Clone this during `setup()` and call `request_redraw()` when external
    /// data arrives (e.g., nami signal change, timer, network response).
    pub redraw_handle: RedrawHandle,
}

/// Largest MSAA sample count the adapter supports for `format`, capped at
/// `max_samples` and never below 1.
#[must_use]
fn supported_msaa_samples(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    max_samples: u32,
) -> u32 {
    let flags = adapter.get_texture_format_features(format).flags;
    [16, 8, 4, 2]
        .into_iter()
        .filter(|count| *count <= max_samples)
        .find(|count| flags.sample_count_supported(*count))
        .unwrap_or(1)
}

impl fmt::Debug for GpuContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContext")
            .field("surface_format", &self.surface_format)
            .field("msaa_samples", &self.msaa_samples)
            .finish_non_exhaustive()
    }
}

impl<'a> GpuContext<'a> {
    /// Creates a context, deriving the MSAA sample count from what the
    /// adapter actually supports for `surface_format`.
    ///
    /// `max_samples` caps the search: pass the highest count the renderer is
    /// willing to allocate attachments for. The result is the largest
    /// supported count at or below that cap, and always at least 1.
    #[must_use]
    pub fn new(
        adapter: &'a wgpu::Adapter,
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        shader_cache: &'a WgslModuleCache,
        scene_renderer: &'a Arc<crate::gpu::shared_context::SharedSceneRenderer>,
        max_samples: u32,
        redraw_handle: RedrawHandle,
    ) -> Self {
        Self {
            adapter,
            device,
            queue,
            surface_format,
            shader_cache,
            scene_renderer,
            msaa_samples: supported_msaa_samples(adapter, surface_format, max_samples),
            redraw_handle,
        }
    }

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
            .map(|p| (p.x / u32_to_f32(width), p.y / u32_to_f32(height)))
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
type RedrawWaker = dyn Fn() + Send + Sync + 'static;

struct RedrawState {
    dirty: AtomicBool,
    waker: RwLock<Option<alloc::sync::Arc<RedrawWaker>>>,
}

impl fmt::Debug for RedrawState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedrawState")
            .field("dirty", &self.dirty.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// A thread-safe handle that schedules another frame for its `GpuSurface`.
///
/// Clones share one coalesced dirty bit and one backend wake target.
#[derive(Clone, Debug)]
pub struct RedrawHandle {
    state: alloc::sync::Arc<RedrawState>,
}

impl RedrawHandle {
    /// Creates a new redraw handle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: alloc::sync::Arc::new(RedrawState {
                dirty: AtomicBool::new(false),
                waker: RwLock::new(None),
            }),
        }
    }

    /// Mark the surface as needing a redraw.
    pub fn request_redraw(&self) {
        if self.state.dirty.swap(true, Ordering::AcqRel) {
            return;
        }

        let waker = self.state.waker.read().clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    /// Returns whether a redraw is pending without consuming the request.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.state.dirty.load(Ordering::Acquire)
    }

    /// Check and clear the dirty flag. Returns `true` if a redraw was requested.
    #[must_use]
    pub fn take_dirty(&self) -> bool {
        self.state.dirty.swap(false, Ordering::AcqRel)
    }

    /// Installs the platform wake callback used to leave the idle state.
    ///
    /// This is backend infrastructure. Application renderers receive a clone of
    /// this handle through [`GpuContext`] and only call [`Self::request_redraw`].
    #[doc(hidden)]
    pub fn set_waker(&self, waker: Option<alloc::sync::Arc<RedrawWaker>>) {
        *self.state.waker.write() = waker;

        if self.is_dirty() {
            let waker = self.state.waker.read().clone();
            if let Some(waker) = waker {
                waker();
            }
        }
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
    /// Elapsed animation time since the surface started rendering.
    elapsed: Duration,
    /// Time advanced since the previous frame.
    delta: Duration,
    /// Internal: set to true when `request_redraw()` is called.
    redraw_requested: bool,
}

impl fmt::Debug for GpuFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    #[expect(
        clippy::too_many_arguments,
        reason = "a frame payload carries the complete render target, input, and timing context"
    )]
    pub const fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        texture: &'a wgpu::Texture,
        view: wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        pointer: PointerState,
        gesture: GestureState,
        elapsed: Duration,
        delta: Duration,
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
            elapsed,
            delta,
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

    /// Returns accumulated animation time for the surface.
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Returns the frame-to-frame time step.
    #[must_use]
    pub const fn delta(&self) -> Duration {
        self.delta
    }

    /// Request that `render()` be called again on the next frame.
    ///
    /// Use this for animations. If not called, the surface stays idle
    /// until an external event triggers a redraw via [`RedrawHandle`].
    pub const fn request_redraw(&mut self) {
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
pub trait GpuView: 'static {
    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, buffers, bind groups, and other
    /// GPU resources that persist across frames.
    ///
    /// `ctx.redraw_handle` can be cloned here for external redraw triggers.
    /// `env` provides access to the `WaterUI` environment (theme, fonts, etc.).
    ///
    /// Async setup hook for GPU resources.
    #[expect(
        async_fn_in_trait,
        reason = "GPU setup is deliberately a main-thread, potentially non-Send future"
    )]
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
    /// - `None`: follow the backend's surrounding platform style.
    ///
    /// This is evaluated by native backends during surface initialization.
    fn preferred_surface_hdr(&self) -> Option<bool> {
        None
    }

    /// Measure the view for a layout proposal.
    ///
    /// GPU views default to filling the proposed size (stretch). Override for
    /// content-sized rendering (e.g. images respecting an aspect ratio).
    /// This is layout-only and must not touch GPU/render state, so it runs as a
    /// pure measurement; `GpuSurface` confines the view to the main thread anyway.
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    /// Which axis (or axes) this view stretches to fill. Defaults to both.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    /// Layout priority for space distribution. Defaults to `0`.
    fn priority(&self) -> i32 {
        0
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
    ///
    /// # Errors
    ///
    /// Returns [`OffscreenRenderError::InvalidSize`] when either dimension is zero.
    pub const fn try_from_pixels(width: u32, height: u32) -> Result<Self, OffscreenRenderError> {
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
    ///
    /// # Errors
    ///
    /// Returns the PNG encoding error when the raw RGBA buffer is invalid.
    pub fn into_png(self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_png(self.width, self.height, self.rgba8)
    }

    /// Encodes the RGBA data as PNG without consuming output.
    ///
    /// # Errors
    ///
    /// Returns the PNG encoding error when the raw RGBA buffer is invalid.
    pub fn to_png(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_png(self.width, self.height, self.rgba8.clone())
    }

    /// Saves the rendered image as a PNG file.
    ///
    /// # Errors
    ///
    /// Returns PNG encoding or filesystem write errors.
    pub fn save_png<P: AsRef<Path>>(&self, path: P) -> Result<(), OffscreenRenderError> {
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
    /// Encodes as PNG with automatic dynamic-range handling.
    ///
    /// - If HDR headroom is detected (`RGB > 1.0`), emits a standards-based HDR PNG
    ///   (PQ-coded 16-bit RGBA + `cICP` chunk).
    /// - Otherwise emits SDR PNG (16-bit sRGB transfer).
    ///
    /// # Errors
    ///
    /// Returns PNG encoding errors or validation errors for invalid HDR buffers.
    pub fn to_png(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_auto_png(self.width, self.height, &self.rgba16f)
    }

    /// Encodes PNG with automatic dynamic-range handling without consuming output.
    ///
    /// # Errors
    ///
    /// Returns PNG encoding errors or validation errors for invalid HDR buffers.
    pub fn into_png(self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_auto_png(self.width, self.height, &self.rgba16f)
    }

    /// Encodes as SDR PNG using automatic tone mapping.
    ///
    /// # Errors
    ///
    /// Returns PNG encoding errors or validation errors for invalid HDR buffers.
    pub fn to_sdr_png(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_sdr_tonemapped_png(self.width, self.height, &self.rgba16f)
    }

    /// Encodes as SDR PNG without consuming output.
    ///
    /// # Errors
    ///
    /// Returns PNG encoding errors or validation errors for invalid HDR buffers.
    pub fn into_sdr_png(self) -> Result<Vec<u8>, OffscreenRenderError> {
        encode_sdr_tonemapped_png(self.width, self.height, &self.rgba16f)
    }

    /// Converts to SDR `RGBA8` bytes using automatic tone mapping.
    ///
    /// # Errors
    ///
    /// Returns validation errors for invalid HDR buffers.
    pub fn to_sdr_rgba8(&self) -> Result<Vec<u8>, OffscreenRenderError> {
        decode_sdr_tonemapped_rgba8(self.width, self.height, &self.rgba16f)
    }

    /// Converts to SDR `RGBA8` bytes using automatic tone mapping.
    ///
    /// # Errors
    ///
    /// Returns validation errors for invalid HDR buffers.
    pub fn into_sdr_rgba8(self) -> Result<Vec<u8>, OffscreenRenderError> {
        decode_sdr_tonemapped_rgba8(self.width, self.height, &self.rgba16f)
    }

    /// Saves the rendered image as PNG with automatic dynamic-range handling.
    ///
    /// # Errors
    ///
    /// Returns PNG encoding or filesystem write errors.
    pub fn save_png<P: AsRef<Path>>(&self, path: P) -> Result<(), OffscreenRenderError> {
        let png = self.to_png()?;
        std::fs::write(path, png).map_err(|e| OffscreenRenderError::PngWriteFailed(e.to_string()))
    }

    /// Saves the rendered image as SDR PNG using automatic tone mapping.
    ///
    /// # Errors
    ///
    /// Returns PNG encoding or filesystem write errors.
    pub fn save_sdr_png<P: AsRef<Path>>(&self, path: P) -> Result<(), OffscreenRenderError> {
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
    /// GPU readback buffer mapping failed.
    ReadbackMapFailed(String),
    /// PNG encoding failed.
    PngEncodingFailed(String),
    /// Writing PNG to disk failed.
    PngWriteFailed(String),
}

impl fmt::Display for OffscreenRenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            Self::ReadbackMapFailed(error) => write!(f, "GPU readback mapping failed: {error}"),
            Self::PngEncodingFailed(error) => write!(f, "PNG encoding failed: {error}"),
            Self::PngWriteFailed(error) => write!(f, "PNG write failed: {error}"),
        }
    }
}

impl Error for OffscreenRenderError {}

/// Private object-safe trait for type-erased GPU views.
trait GpuViewImpl: 'static {
    fn setup<'a>(
        &'a mut self,
        ctx: &'a GpuContext<'a>,
        env: &'a mut waterui_core::Environment,
    ) -> ErasedSetupFuture<'a>;
    fn render(&mut self, frame: &mut GpuFrame);
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;
    fn stretch_axis(&self) -> StretchAxis;
    fn priority(&self) -> i32;
    fn preferred_surface_hdr(&self) -> Option<bool>;
}

impl<T: GpuView> GpuViewImpl for T {
    fn setup<'a>(
        &'a mut self,
        ctx: &'a GpuContext<'a>,
        env: &'a mut waterui_core::Environment,
    ) -> ErasedSetupFuture<'a> {
        Box::pin(GpuView::setup(self, ctx, env))
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        GpuView::render(self, frame);
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        GpuView::measure(self, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        GpuView::stretch_axis(self)
    }

    fn priority(&self) -> i32 {
        GpuView::priority(self)
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
    // The GPU view is `!Send` (it owns wgpu/render state and may capture reactive
    // signals). `measure` is the only `SubView` method touched during layout, so the
    // surface confines the view to the main thread.
    renderer: MainThreadBound<Box<dyn GpuViewImpl>>,
    /// Preferred maximum MSAA sample count for this surface.
    ///
    /// Backends use this as the cap when selecting a supported sample count.
    msaa_max_samples: NonZeroU32,
    /// Optional per-surface HDR preference override.
    ///
    /// `None` follows the backend's surrounding platform style.
    /// `Some(true)` prefers HDR, `Some(false)` prefers SDR.
    surface_prefers_hdr: Option<bool>,
    /// Optional picture-in-picture host id for backend registration.
    picture_in_picture_host_id: Option<u64>,
}

const DEFAULT_MSAA_MAX_SAMPLES: NonZeroU32 =
    NonZeroU32::new(4).expect("default MSAA sample count is non-zero");

impl fmt::Debug for GpuSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuSurface").finish_non_exhaustive()
    }
}

impl GpuSurface {
    /// Creates a new GPU surface with the provided GPU view.
    ///
    /// # Arguments
    ///
    /// * `view` - An implementation of `GpuView` that handles setup and rendering.
    ///
    /// # Panics
    ///
    /// Panics when constructed outside the main UI thread.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let surface = GpuSurface::new(MyRenderer::default());
    /// ```
    #[must_use]
    pub fn new<R: GpuView>(view: R) -> Self {
        Self {
            renderer: MainThreadBound::new(Box::new(view)),
            msaa_max_samples: DEFAULT_MSAA_MAX_SAMPLES,
            surface_prefers_hdr: None,
            picture_in_picture_host_id: None,
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
    pub const fn msaa_sample_limit(&self) -> NonZeroU32 {
        self.msaa_max_samples
    }

    /// Prefer HDR swapchain formats for this surface when available.
    ///
    /// This overrides the surrounding platform style for this surface only.
    #[must_use]
    pub const fn prefer_hdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(true);
        self
    }

    /// Prefer SDR swapchain formats for this surface even when HDR is available.
    ///
    /// This overrides the surrounding platform style for this surface only.
    #[must_use]
    pub const fn prefer_sdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(false);
        self
    }

    /// Resolves this surface's explicit or renderer-provided HDR preference.
    ///
    /// `None` means follow the surrounding platform style.
    #[must_use]
    pub fn resolved_hdr_preference(&self) -> Option<bool> {
        self.surface_prefers_hdr
            .or_else(|| self.renderer.preferred_surface_hdr())
    }

    /// Associates this surface with a picture-in-picture host id.
    #[must_use]
    pub const fn picture_in_picture_host_id(mut self, host_id: u64) -> Self {
        self.picture_in_picture_host_id = Some(host_id);
        self
    }

    /// Returns the picture-in-picture host id, if any.
    #[must_use]
    pub const fn picture_in_picture_host(&self) -> Option<u64> {
        self.picture_in_picture_host_id
    }

    /// Renders this surface once into an offscreen texture and asynchronously reads back RGBA8 pixels.
    ///
    /// This is intended for visual regression checks and snapshot generation
    /// without launching a full app window.
    ///
    /// # Errors
    ///
    /// Returns format validation, MSAA validation, or readback errors.
    #[expect(
        clippy::future_not_send,
        reason = "offscreen GpuView setup is UI-local and borrows the main-thread Environment"
    )]
    pub async fn render_offscreen(
        self,
        runtime: &GpuRuntime,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        self.render_offscreen_frames(runtime, config, env, NonZeroU32::MIN)
            .await
    }

    /// Renders this surface into an offscreen texture for `frame_count` frames and asynchronously reads back RGBA8 pixels.
    ///
    /// # Errors
    ///
    /// Returns format validation, MSAA validation, or readback errors.
    #[expect(
        clippy::future_not_send,
        reason = "offscreen GpuView setup is UI-local and borrows the main-thread Environment"
    )]
    pub async fn render_offscreen_frames(
        mut self,
        runtime: &GpuRuntime,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
        frame_count: NonZeroU32,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        validate_rgba_readback_format(config.format)?;
        let shared = runtime.context();
        let width = config.size.width();
        let height = config.size.height();
        let msaa_samples = resolve_offscreen_msaa(
            &shared.adapter,
            config.format,
            config.msaa_samples,
            self.msaa_max_samples,
        )?;
        let device = shared.device.as_ref();
        let queue = shared.queue.as_ref();
        let context = GpuContext {
            adapter: &shared.adapter,
            device,
            queue,
            surface_format: config.format,
            shader_cache: shared.shader_cache.as_ref(),
            scene_renderer: shared.scene_renderer(),
            msaa_samples,
            redraw_handle: RedrawHandle::new(),
        };
        self.setup(&context, env).await;

        let mut texture_usage =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        if config.format == wgpu::TextureFormat::Rgba8Unorm {
            texture_usage |= wgpu::TextureUsages::STORAGE_BINDING;
        }
        let texture = create_offscreen_texture(
            device,
            "waterui_offscreen_surface",
            width,
            height,
            config.format,
            texture_usage,
        );
        render_offscreen_frames_to_texture(&mut self, device, queue, &texture, config, frame_count);
        let rgba8 = readback_texture(
            runtime,
            &texture,
            width,
            height,
            4,
            "waterui_offscreen_readback",
            "waterui_offscreen_readback_encoder",
        )
        .await?;
        Ok(OffscreenRenderOutput {
            width,
            height,
            rgba8,
        })
    }

    /// Renders this surface into an HDR offscreen texture and asynchronously reads back `RGBA16F` pixels.
    ///
    /// # Errors
    ///
    /// Returns format validation, MSAA validation, or readback errors.
    #[expect(
        clippy::future_not_send,
        reason = "offscreen GpuView setup is UI-local and borrows the main-thread Environment"
    )]
    pub async fn render_offscreen_hdr(
        self,
        runtime: &GpuRuntime,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        self.render_offscreen_hdr_frames(runtime, config, env, NonZeroU32::MIN)
            .await
    }

    /// Renders this surface into an HDR offscreen texture for `frame_count` frames and asynchronously reads back `RGBA16F` pixels.
    ///
    /// # Errors
    ///
    /// Returns format validation, MSAA validation, or readback errors.
    #[expect(
        clippy::future_not_send,
        reason = "offscreen GpuView setup is UI-local and borrows the main-thread Environment"
    )]
    pub async fn render_offscreen_hdr_frames(
        mut self,
        runtime: &GpuRuntime,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
        frame_count: NonZeroU32,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        if config.format != wgpu::TextureFormat::Rgba16Float {
            return Err(OffscreenRenderError::UnsupportedReadbackFormat(
                config.format,
            ));
        }
        let shared = runtime.context();
        let width = config.size.width();
        let height = config.size.height();
        let msaa_samples = resolve_offscreen_msaa(
            &shared.adapter,
            config.format,
            config.msaa_samples,
            self.msaa_max_samples,
        )?;
        let device = shared.device.as_ref();
        let queue = shared.queue.as_ref();
        let context = GpuContext {
            adapter: &shared.adapter,
            device,
            queue,
            surface_format: config.format,
            shader_cache: shared.shader_cache.as_ref(),
            scene_renderer: shared.scene_renderer(),
            msaa_samples,
            redraw_handle: RedrawHandle::new(),
        };
        self.setup(&context, env).await;

        let texture = create_offscreen_texture(
            device,
            "waterui_offscreen_surface_hdr",
            width,
            height,
            config.format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        render_offscreen_frames_to_texture(&mut self, device, queue, &texture, config, frame_count);
        let rgba16f = readback_texture(
            runtime,
            &texture,
            width,
            height,
            8,
            "waterui_offscreen_readback_hdr",
            "waterui_offscreen_readback_hdr_encoder",
        )
        .await?;
        Ok(OffscreenRenderOutputHdr {
            width,
            height,
            rgba16f,
        })
    }

    /// Calls `setup` on the GPU view, returning a future that completes when ready.
    #[expect(
        clippy::future_not_send,
        reason = "GpuView setup is driven by the UI-local GPU host executor and intentionally accepts non-Send renderers"
    )]
    pub fn setup<'a>(
        &'a mut self,
        ctx: &'a GpuContext<'a>,
        env: &'a mut waterui_core::Environment,
    ) -> impl Future<Output = ()> + 'a {
        self.renderer.setup(ctx, env)
    }

    /// Calls `render` on the GPU view.
    pub fn render(&mut self, frame: &mut GpuFrame) {
        self.renderer.render(frame);
    }

    /// Returns this surface's measured size for the given proposal.
    #[must_use]
    pub fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        self.measure(proposal).size
    }

    /// Returns this surface's complete layout dimensions for the proposal.
    #[must_use]
    pub fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.renderer.measure(proposal)
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

}

fn resolve_offscreen_msaa(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    requested: Option<NonZeroU32>,
    default_maximum: NonZeroU32,
) -> Result<u32, OffscreenRenderError> {
    let supported = preferred_msaa_samples(adapter, format, requested.unwrap_or(default_maximum));
    if let Some(requested) = requested
        && requested.get() != supported
    {
        return Err(OffscreenRenderError::UnsupportedMsaaSamples {
            requested: requested.get(),
            format,
        });
    }
    Ok(supported)
}

fn create_offscreen_texture(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn render_offscreen_frames_to_texture(
    surface: &mut GpuSurface,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    config: OffscreenRenderConfig,
    frame_count: NonZeroU32,
) {
    let width = config.size.width();
    let height = config.size.height();
    let frame_delta = Duration::from_secs_f32(1.0 / 60.0);
    let mut frame = GpuFrame {
        device,
        queue,
        texture,
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format: config.format,
        width,
        height,
        pointer: config.pointer,
        gesture: config.gesture,
        elapsed: frame_delta,
        delta: frame_delta,
        redraw_requested: false,
    };
    for frame_index in 0..frame_count.get() {
        frame.elapsed = frame_delta.saturating_mul(frame_index + 1);
        frame.delta = frame_delta;
        surface.render(&mut frame);
    }
}

const fn validate_rgba_readback_format(
    format: wgpu::TextureFormat,
) -> Result<(), OffscreenRenderError> {
    if matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
    ) {
        Ok(())
    } else {
        Err(OffscreenRenderError::UnsupportedReadbackFormat(format))
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

async fn readback_texture(
    runtime: &GpuRuntime,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
    label: &'static str,
    encoder_label: &'static str,
) -> Result<Vec<u8>, OffscreenRenderError> {
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let context = runtime.context();
    let device = context.device.as_ref();
    let queue = context.queue.as_ref();
    let unpadded_bpr = width * bytes_per_pixel;
    let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
    let copy_size = u64::from(padded_bpr) * u64::from(height);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: copy_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some(encoder_label),
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
    let submission = queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    context
        .submission_completion_driver()
        .on_complete(submission, || {});
    receiver
        .await
        .expect("GPU readback callback dropped before reporting its result")
        .map_err(|error| OffscreenRenderError::ReadbackMapFailed(error.to_string()))?;

    let mapped = slice.get_mapped_range();
    let mut output = vec![0_u8; (width * height * bytes_per_pixel) as usize];
    for row in 0..height as usize {
        let src_start = row * padded_bpr as usize;
        let src_end = src_start + unpadded_bpr as usize;
        let dst_start = row * unpadded_bpr as usize;
        let dst_end = dst_start + unpadded_bpr as usize;
        output[dst_start..dst_end].copy_from_slice(&mapped[src_start..src_end]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(output)
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

const SDR_WHITE_NITS: f32 = 203.0;

fn encode_auto_png(
    width: u32,
    height: u32,
    rgba16f: &[u8],
) -> Result<Vec<u8>, OffscreenRenderError> {
    let (max_rgb, hdr_ratio) = analyze_hdr_headroom(rgba16f);
    if max_rgb > 1.0 && hdr_ratio > 0.0 {
        encode_hdr_pq_png(width, height, rgba16f)
    } else {
        encode_sdr_png_linear(width, height, rgba16f)
    }
}

fn encode_sdr_tonemapped_png(
    width: u32,
    height: u32,
    rgba16f: &[u8],
) -> Result<Vec<u8>, OffscreenRenderError> {
    let expected = validate_rgba16f_buffer(width, height, rgba16f)?;
    let white_point = compute_tonemap_white_point(rgba16f);
    let png16 = rgba16f_to_sdr_srgb16_bytes(rgba16f, white_point, expected);
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
    rgba16f: &[u8],
) -> Result<Vec<u8>, OffscreenRenderError> {
    let expected = validate_rgba16f_buffer(width, height, rgba16f)?;
    let png16 = rgba16f_to_sdr_srgb16_bytes(rgba16f, 1.0, expected);
    encode_png16(width, height, &png16, None)
}

fn encode_hdr_pq_png(
    width: u32,
    height: u32,
    rgba16f: &[u8],
) -> Result<Vec<u8>, OffscreenRenderError> {
    let expected = validate_rgba16f_buffer(width, height, rgba16f)?;
    // Treat linear 1.0 as HDR reference white (203 nits), then encode absolute PQ.
    // This keeps >1.0 headroom while producing standards-based HDR signaling.
    let mut png16 = Vec::with_capacity(expected);
    for px in rgba16f.as_chunks::<8>().0 {
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
        let r16 = unit_f32_to_u16(r);
        let g16 = unit_f32_to_u16(g);
        let b16 = unit_f32_to_u16(b);
        let a16 = unit_f32_to_u16(a);
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
    for px in rgba16f.as_chunks::<8>().0 {
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
        let r16 = unit_f32_to_u16(r);
        let g16 = unit_f32_to_u16(g);
        let b16 = unit_f32_to_u16(b);
        let a16 = unit_f32_to_u16(a);
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
    for px in rgba16f.as_chunks::<8>().0 {
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
        rgba8.push(unit_f32_to_u8(r));
        rgba8.push(unit_f32_to_u8(g));
        rgba8.push(unit_f32_to_u8(b));
        rgba8.push(unit_f32_to_u8(a));
    }
    rgba8
}

fn compute_tonemap_white_point(rgba16f: &[u8]) -> f32 {
    let mut frame_max = Vec::with_capacity(rgba16f.len() / 8);
    for px in rgba16f.as_chunks::<8>().0 {
        let r = f16_to_f32(u16::from_le_bytes([px[0], px[1]])).max(0.0);
        let g = f16_to_f32(u16::from_le_bytes([px[2], px[3]])).max(0.0);
        let b = f16_to_f32(u16::from_le_bytes([px[4], px[5]])).max(0.0);
        frame_max.push(r.max(g).max(b));
    }
    if frame_max.is_empty() {
        return 1.0;
    }
    frame_max.sort_unstable_by(f32::total_cmp);
    let idx = (((frame_max.len() - 1) * 995) + 500) / 1000;
    frame_max[idx].max(1.0)
}

fn analyze_hdr_headroom(rgba16f: &[u8]) -> (f32, f32) {
    let mut max_rgb = 0.0f32;
    let mut total = 0usize;
    let mut hdr = 0usize;
    for px in rgba16f.as_chunks::<8>().0 {
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
        usize_to_f32(hdr) / usize_to_f32(total)
    };
    (max_rgb, ratio)
}

#[inline]
fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055f32.mul_add(x.powf(1.0 / 2.4), -0.055)
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
    let num = f32::mul_add(c2, ym1, c1);
    let den = f32::mul_add(c3, ym1, 1.0);
    (num / den).powf(m2)
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from((bits >> 15) & 0x1);
    let exp = u32::from((bits >> 10) & 0x1f);
    let frac = u32::from(bits & 0x03ff);

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
            let exp32 = (e + 127).cast_unsigned();
            (sign << 31) | (exp32 << 23) | (frac_norm << 13)
        }
    } else if exp == 0x1f {
        (sign << 31) | 0x7f80_0000 | (frac << 13)
    } else {
        let exp32 = (i32::try_from(exp).expect("half-float exponent fits in i32") - 15 + 127)
            .cast_unsigned();
        (sign << 31) | (exp32 << 23) | (frac << 13)
    };
    f32::from_bits(f32_bits)
}

fn u32_to_f32(value: u32) -> f32 {
    value
        .to_f32()
        .expect("gpu_surface: u32 value must be representable as f32")
}

fn usize_to_f32(value: usize) -> f32 {
    value
        .to_f32()
        .expect("gpu_surface: usize value must be representable as f32")
}

fn unit_f32_to_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0)
        .round()
        .to_u16()
        .expect("gpu_surface: unit float must convert to u16")
}

fn unit_f32_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0)
        .round()
        .to_u8()
        .expect("gpu_surface: unit float must convert to u8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        num::NonZeroU32,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    fn rgba16f_pixel_le(r: u16, g: u16, b: u16, a: u16) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&r.to_le_bytes());
        bytes.extend_from_slice(&g.to_le_bytes());
        bytes.extend_from_slice(&b.to_le_bytes());
        bytes.extend_from_slice(&a.to_le_bytes());
        bytes
    }

    #[test]
    fn redraw_handle_coalesces_wakes_until_consumed() {
        let handle = RedrawHandle::new();
        let wakes = Arc::new(AtomicUsize::new(0));
        let callback_wakes = Arc::clone(&wakes);
        handle.set_waker(Some(Arc::new(move || {
            callback_wakes.fetch_add(1, Ordering::Relaxed);
        })));

        handle.request_redraw();
        handle.request_redraw();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert!(handle.take_dirty());

        handle.request_redraw();
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn redraw_handle_wakes_when_callback_is_installed_after_request() {
        let handle = RedrawHandle::new();
        handle.request_redraw();

        let wakes = Arc::new(AtomicUsize::new(0));
        let callback_wakes = Arc::clone(&wakes);
        handle.set_waker(Some(Arc::new(move || {
            callback_wakes.fetch_add(1, Ordering::Relaxed);
        })));

        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert!(handle.take_dirty());
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

    #[test]
    fn offscreen_frames_advance_elapsed_and_delta() {
        fn assert_duration_near(actual: Duration, expected: Duration, label: &str) {
            let diff = actual.abs_diff(expected);
            assert!(
                diff <= Duration::from_nanos(10),
                "{label} mismatch: got {actual:?}, expected {expected:?}, diff {diff:?}"
            );
        }

        #[derive(Clone)]
        struct FrameProbe {
            records: Rc<RefCell<Vec<(Duration, Duration)>>>,
        }

        impl GpuView for FrameProbe {
            #[expect(
                clippy::future_not_send,
                reason = "GpuView setup runs on the UI-local GPU executor with non-Send test state"
            )]
            async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
            }

            fn render(&mut self, frame: &mut GpuFrame) {
                self.records
                    .borrow_mut()
                    .push((frame.elapsed(), frame.delta()));
            }
        }

        let records = Rc::new(RefCell::new(Vec::new()));
        let size = OffscreenSize::try_from_pixels(4, 4).expect("probe size must be valid");
        let config = OffscreenRenderConfig::new(size);
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("offscreen frame test requires a working GPU runtime");
        let mut env = waterui_core::Environment::new();
        pollster::block_on(
            GpuSurface::new(FrameProbe {
                records: Rc::clone(&records),
            })
            .render_offscreen_frames(
                &runtime,
                config,
                &mut env,
                NonZeroU32::new(3).expect("non-zero literal"),
            ),
        )
        .expect("offscreen frame probe should render");

        let records = records.borrow();
        assert_eq!(records.len(), 3, "expected one record per offscreen frame");
        assert_duration_near(
            records[0].0,
            Duration::from_secs_f32(1.0 / 60.0),
            "frame 1 elapsed",
        );
        assert_duration_near(
            records[0].1,
            Duration::from_secs_f32(1.0 / 60.0),
            "frame 1 delta",
        );
        assert_duration_near(
            records[1].0,
            Duration::from_secs_f32(2.0 / 60.0),
            "frame 2 elapsed",
        );
        assert_duration_near(
            records[1].1,
            Duration::from_secs_f32(1.0 / 60.0),
            "frame 2 delta",
        );
        assert_duration_near(
            records[2].0,
            Duration::from_secs_f32(3.0 / 60.0),
            "frame 3 elapsed",
        );
        assert_duration_near(
            records[2].1,
            Duration::from_secs_f32(1.0 / 60.0),
            "frame 3 delta",
        );
    }
}
