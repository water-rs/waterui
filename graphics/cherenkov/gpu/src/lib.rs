//! `cherenkov-gpu`: the wgpu backend for the Cherenkov 2D rendering
//! engine.
//!
//! The shared front end lives in the [`cherenkov`] crate:
//! [`Engine`](cherenkov::Engine), [`Surface`](cherenkov::Surface),
//! [`Layer`](cherenkov::Layer), the layer tree and the render thread's loop
//! are all generic over [`Backend`]. This crate supplies the render side
//! only — [`Gpu`]'s [`Backend`] implementation drives the wgpu device on the
//! render thread.
//!
//! ```no_run
//! use cherenkov::{Draw, Engine, Offscreen, OffscreenFormat, WorkingColor};
//! use cherenkov::kurbo::Rect;
//! use cherenkov_gpu::{Gpu, GpuConfig};
//!
//! let engine = Engine::<Gpu>::new(GpuConfig::default())?;
//! let surface = engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {})?;
//! surface.update(|tx| {
//!     tx[surface.root()].content(
//!         surface.record(|c| c.fill(Rect::new(0., 0., 64., 64.), WorkingColor::WHITE)),
//!     );
//! });
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[cfg(feature = "bench")]
pub mod bench;
pub mod interop;
mod names;
mod render;

/// The allocation-event diagnostic sink (issue #169).
pub use render::diag;
/// The registered-effect module text the renderer compiles; exposed for
/// `tests/shader.rs`, which translates it through every naga backend.
#[doc(hidden)]
pub use render::shaders::backdrop_effect_text;

use std::path::PathBuf;

use cherenkov::{Backend, EngineError, Offscreen, Rgba8, Rgba16F, Uploads};

/// Adapter information for provenance.
#[derive(Clone, Debug)]
pub struct GpuInfo {
    /// Adapter name.
    pub name: String,
    /// Backend (e.g. `Vulkan`).
    pub backend: String,
    /// PCI vendor id.
    pub vendor: u32,
    /// PCI device id.
    pub device: u32,
    /// Device class (e.g. `IntegratedGpu`).
    pub device_type: String,
    /// Driver name.
    pub driver: String,
    /// Driver version detail.
    pub driver_info: String,
    /// Supported GPU timestamp positions.
    pub timestamps: TimestampSupport,
}

/// One engine-creation boundary a [`CreationProbe`] observes — the phases
/// of issue #170's creation ledger, in the order `init` reaches them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreationPhase {
    /// `wgpu::Instance` created.
    Instance,
    /// Adapter selected for the working format.
    Adapter,
    /// Logical device and queue opened.
    Device,
    /// Engine bind group layouts created.
    Layouts,
    /// The fixed engine shader modules created.
    ShaderModules,
    /// The closed core pipeline set created.
    CorePipelines,
    /// Frame-wide buffers (globals, instances, stops) created.
    Buffers,
    /// Coverage atlas storage created.
    Atlas,
    /// The group-0 bind group and dummy views bound.
    BindGroups,
    /// Timestamp query resources created; absent when timestamps are off.
    Timestamps,
    /// The silhouette-blur pipeline created.
    ShadowBlur,
    /// The Vulkan external-frame context created.
    ExternalNative,
    /// The renderer is fully constructed.
    Complete,
}

/// The boundary a [`CreationProbe`] observes.
#[derive(Debug)]
pub struct CreationPoint<'a> {
    /// The phase that just completed.
    pub phase: CreationPhase,
    /// The adapter once selected.
    pub adapter: Option<&'a wgpu::Adapter>,
    /// The device once opened; allocator accounting is available from
    /// this phase on.
    pub device: Option<&'a wgpu::Device>,
}

/// A synchronous observer of engine creation (issue #170).
///
/// `cherenkov-bench creation` installs one through
/// [`GpuConfig::creation_probe`] to snapshot process and driver memory at
/// each phase boundary. The observer runs on the thread performing the
/// phase — the engine's render thread for `init` — and returning `true`
/// aborts creation just past it with [`EngineError::Backend`], the
/// ledger's one-factor ablation handle.
///
/// Diagnostics only, like [`GpuConfig::alloc_diag`].
#[derive(Clone)]
pub struct CreationProbe {
    observe: std::sync::Arc<dyn for<'a> Fn(&CreationPoint<'a>) -> bool + Send + Sync>,
}

impl CreationProbe {
    /// An observer called at every phase boundary; `true` aborts creation.
    pub fn new(
        observe: impl for<'a> Fn(&CreationPoint<'a>) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            observe: std::sync::Arc::new(observe),
        }
    }

    /// Runs the observer; `true` asks the caller to abort creation.
    pub(crate) fn fire(&self, point: &CreationPoint<'_>) -> bool {
        (self.observe)(point)
    }
}

impl std::fmt::Debug for CreationProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreationProbe").finish_non_exhaustive()
    }
}

/// Where the adapter lets the renderer sample GPU timestamps.
///
/// The renderer samples only at pass boundaries, the one position every
/// adapter with timestamp queries honours. Encoder-level sampling is not
/// a level here: Metal on Apple GPUs advertises it but samples only at
/// stage boundaries, through a dummy blit encoder wgpu documents as
/// unreliable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimestampSupport {
    /// No timestamp queries; `finish_timings` returns no GPU timings.
    Unsupported,
    /// At render and compute pass boundaries.
    PassBoundaries,
}

/// The texture format used for intermediate (isolation) render targets.
///
/// The surface itself stays `Rgba16Float` regardless; this only picks the
/// precision of offscreen layers the compositor reads back in the same
/// frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScratchFormat {
    /// `Rgba16Float`: linear, no banding, 8 bytes per texel.
    #[default]
    LinearF16,
    /// `Rgba8Unorm`: half the bandwidth, 8-bit precision; intermediate
    /// results are clamped to `0..=1`.
    Rgba8Unorm,
}

/// Configuration for the GPU engine.
#[derive(Clone, Debug)]
pub struct GpuConfig {
    /// Uses an existing host device. Handles must share one creation chain.
    /// Device limits and enabled features govern engine capabilities.
    pub device: Option<interop::SharedDevice>,
    /// Which wgpu backends may be used. Defaults to all.
    pub backends: wgpu::Backends,
    /// Adapter power preference. Defaults to high performance.
    pub power_preference: wgpu::PowerPreference,
    /// When true and the adapter supports it,
    /// [`Engine::render`](cherenkov::Engine::render) measures GPU time
    /// with timestamp queries resolved after completion and reported on a
    /// later render, never stalling the frame on GPU idle.
    pub timestamps: bool,
    /// Window of a native GPU wait, and the deadline of a browser one.
    /// A native wait opens a new window every time the queue retires any
    /// submission — a slow adapter keeps draining — and fails with
    /// [`RenderError::Timeout`](cherenkov::RenderError::Timeout) only when
    /// a whole window passes with nothing retired: a deadline on progress,
    /// not on duration. On wasm32, where a wait resolves on the page's event
    /// loop, it is a hard timeout.
    pub wait_timeout: std::time::Duration,
    /// Memory budgets.
    pub budget: cherenkov::Budget,
    /// When set and the adapter supports pipeline caches, the closed
    /// pipeline set is persisted at this path, best effort.
    pub pipeline_cache: Option<PathBuf>,
    /// The isolation (scratch) texture format. Defaults to
    /// [`ScratchFormat::LinearF16`].
    pub scratch_format: ScratchFormat,
    /// When set, the renderer records an allocation-event trace into
    /// this sink. Diagnostics only; the per-event allocator snapshot is
    /// deliberately expensive, so keep it out of timed runs.
    pub alloc_diag: Option<diag::Sink>,
    /// When set, creation reports each [`CreationPhase`] boundary to the
    /// observer — issue #170's creation ledger. Diagnostics only.
    pub creation_probe: Option<CreationProbe>,
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            device: None,
            backends: wgpu::Backends::all(),
            power_preference: wgpu::PowerPreference::HighPerformance,
            timestamps: false,
            wait_timeout: std::time::Duration::from_secs(30),
            budget: cherenkov::Budget::default(),
            pipeline_cache: None,
            scratch_format: ScratchFormat::default(),
            alloc_diag: None,
            creation_probe: None,
        }
    }
}

/// The surface targets [`Gpu`] draws into, retaining linear Display P3 output.
#[derive(Debug)]
pub enum GpuTarget {
    /// An offscreen texture.
    Offscreen(Offscreen),
    /// A native window. On Apple platforms the engine builds system-compositor
    /// planes under the view's layer and the system composites the surface,
    /// which is then not readable; elsewhere it presents one swapchain and
    /// retains readable working-space pixels before presentation.
    Window(WindowTarget),
    /// Engine-owned working-space texture shared with a native host.
    Texture(interop::TextureTarget),
    /// Child surface controls of a host's parent, with system compositor
    /// planes for eligible layers.
    #[cfg(target_os = "android")]
    SurfaceControl(interop::android::SurfaceControlTarget),
    /// A Linux target presenting through a bounded pool of exportable
    /// DMA-BUF images with sync-file acquire/release fences (#1687).
    #[cfg(target_os = "linux")]
    Dmabuf(interop::dmabuf::DmabufTarget),
    /// Canvases and the host's elements under a host element of the page,
    /// with DOM planes for hosted content.
    #[cfg(target_arch = "wasm32")]
    Dom(interop::web::DomTarget),
}

/// A window the engine presents on: a raw window handle and the drawable
/// size in pixels.
///
/// On Apple platforms the view's backing layer is the system-compositor
/// parent: eligible layers (video frames) are promoted onto their own system
/// layers, and the engine's own composition is split into parts presented
/// through metal layers around them (#90). The system composites the
/// result, so such a surface is not readable. Elsewhere the engine renders
/// into its own linear f16 target and blits it onto one swapchain, so the
/// surface stays readable.
pub struct WindowTarget {
    #[cfg(not(target_vendor = "apple"))]
    handle: Box<dyn wgpu::WindowHandle>,
    /// The view's backing layer, captured on the caller's thread.
    #[cfg(target_vendor = "apple")]
    parent: render::planes::apple::Parent,
    size: (u32, u32),
    refresh: cherenkov::RefreshRange,
    output: render::present::OutputRequest,
    probe: Option<std::sync::mpsc::Sender<interop::DisplayProbe>>,
}

impl WindowTarget {
    /// Wraps `handle` (any `raw-window-handle` window, e.g. an
    /// `Arc<winit::window::Window>`) at `size` device pixels.
    ///
    /// # Panics
    /// On Apple platforms, off the main thread: the view's layer is captured
    /// here and a view is main-thread state. Also when the handle is
    /// unavailable or is not an `AppKit` or `UIKit` view there.
    pub fn new(handle: impl wgpu::WindowHandle + 'static, size: (u32, u32)) -> Self {
        Self {
            #[cfg(not(target_vendor = "apple"))]
            handle: Box::new(handle),
            #[cfg(target_vendor = "apple")]
            parent: render::planes::apple::Parent::capture(Box::new(handle)),
            size,
            refresh: cherenkov::DEFAULT_REFRESH,
            output: render::present::OutputRequest {
                transparent: false,
                color_space: render::present::ColorSpaceRequest::Best,
                sync: DisplaySync::Synchronized,
            },
            probe: None,
        }
    }

    /// Presents with a composite alpha mode the compositor sees through
    /// (premultiplied, else postmultiplied). Surface creation
    /// fails when the adapter offers none: an opaque composite would present
    /// every pixel with no alpha.
    #[must_use]
    pub const fn transparent(mut self, transparent: bool) -> Self {
        self.output.transparent = transparent;
        self
    }

    /// Sets the swapchain's colour-space request (#98, #2445). The
    /// default, [`ColorSpaceRequest::Best`](interop::ColorSpaceRequest::Best),
    /// negotiates the surface's best advertised pair — an extended or HDR
    /// space where offered, otherwise a reported SDR selection.
    /// [`Range`](interop::ColorSpaceRequest::Range) takes the best pair
    /// inside a [`ColorRangeInterval`](interop::ColorRangeInterval) of
    /// classes, and [`Exact`](interop::ColorSpaceRequest::Exact) one space.
    /// The surface fails with [`SurfaceError::UnsupportedTarget`] when
    /// nothing it advertises meets the request, never substituting a pair
    /// — at creation, or on Apple, where the engine creates its layers on
    /// the main queue, as the first render's [`RenderError::Render`].
    ///
    /// [`SurfaceError::UnsupportedTarget`]: cherenkov::SurfaceError::UnsupportedTarget
    /// [`RenderError::Render`]: cherenkov::RenderError::Render
    #[must_use]
    pub const fn color_space(mut self, request: interop::ColorSpaceRequest) -> Self {
        self.output.color_space = request;
        self
    }

    /// Chooses how presentation is paced against the display (#214).
    /// The default is [`DisplaySync::Synchronized`], which every surface
    /// supports. A surface that cannot present as requested fails with
    /// [`SurfaceError::UnsupportedTarget`] — at creation, or on Apple as
    /// the first render's [`RenderError::Render`] — and never substitutes
    /// another mode. The present mode the request resolved to is reported
    /// as [`OutputSelection::present_mode`](interop::OutputSelection::present_mode).
    ///
    /// [`SurfaceError::UnsupportedTarget`]: cherenkov::SurfaceError::UnsupportedTarget
    /// [`RenderError::Render`]: cherenkov::RenderError::Render
    #[must_use]
    pub const fn display_sync(mut self, sync: DisplaySync) -> Self {
        self.output.sync = sync;
        self
    }

    /// Registers a channel that receives the surface's
    /// [`interop::DisplayProbe`] at creation. The host samples the probe
    /// on the main thread — required on Apple — and feeds the reported
    /// headroom back through
    /// [`Surface::display`](cherenkov::Surface::display).
    pub fn output_probe(&mut self) -> std::sync::mpsc::Receiver<interop::DisplayProbe> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.probe = Some(sender);
        receiver
    }

    /// Sets the refresh range for backend animation and presentation retries.
    ///
    /// # Panics
    /// When the range is empty or includes zero.
    #[must_use]
    pub fn rate(mut self, rate: cherenkov::RefreshRange) -> Self {
        assert!(
            *rate.start() > 0 && !rate.is_empty(),
            "refresh range must be positive and ordered"
        );
        self.refresh = rate;
        self
    }

    /// The drawable size the swapchain is configured to.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// How a window's presentation is paced against the display's refresh —
/// [`WindowTarget::display_sync`] (#214).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplaySync {
    /// Every frame waits for the display's vertical blank and is shown
    /// whole, in order: presentation paces the producer and nothing
    /// tears (FIFO). Every surface supports it.
    #[default]
    Synchronized,
    /// Presentation never waits for the display: mailbox, where the
    /// newest frame replaces a queued one and is shown whole at the next
    /// vertical blank, where the surface supports it; otherwise
    /// immediate, where a frame is shown at once and may tear. For
    /// input-latency measurement and benchmarks. A surface that supports
    /// neither cannot satisfy the request.
    Unsynchronized,
}

impl core::fmt::Debug for WindowTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WindowTarget")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl From<WindowTarget> for GpuTarget {
    fn from(window: WindowTarget) -> Self {
        Self::Window(window)
    }
}

impl From<interop::TextureTarget> for GpuTarget {
    fn from(target: interop::TextureTarget) -> Self {
        Self::Texture(target)
    }
}

impl From<Offscreen> for GpuTarget {
    fn from(offscreen: Offscreen) -> Self {
        Self::Offscreen(offscreen)
    }
}

/// The wgpu backend: renders the shared front end's layer trees through a
/// closed instanced-quad pipeline.
#[derive(Clone, Copy, Debug, Default)]
pub struct Gpu;

impl cherenkov::Target for Gpu {
    type Queue = cherenkov::EngineQueue<Self>;
    type Install = cherenkov::InstallOp<Self>;
}

impl Backend for Gpu {
    type Config = GpuConfig;
    type Info = GpuInfo;
    type Target = GpuTarget;
    type Renderer = render::GpuRenderer;

    #[cfg(not(target_arch = "wasm32"))]
    fn init(config: GpuConfig) -> Result<(Self::Renderer, Self::Info), EngineError> {
        render::init(config)
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn init(config: GpuConfig) -> Result<(Self::Renderer, Self::Info), EngineError> {
        render::init(config).await
    }
}

impl Uploads<Rgba8> for Gpu {}
impl Uploads<Rgba16F> for Gpu {}

// HDR output (#97): `LinearDisplayP3` texture output and window
// presentation tone-map to the display's `Display::headroom` in the
// present shader — see `render::present`.
impl cherenkov::HdrOutput for Gpu {}

// System-compositor planes (#90): eligible layers of an Apple window surface
// are promoted to Core Animation layers, of an Android surface-control
// surface to child surface controls, and of a DOM surface to DOM elements —
// see `render::planes`.
#[cfg(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32"))]
impl cherenkov::Planes for Gpu {}

// Hosted system layers (#2199): the host's `NSView` (macOS), `CALayer` (iOS),
// `SurfaceControl` or DOM element (web) is placed on a plane of its own, never composited —
// see `render::planes`.
#[cfg(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32"))]
impl cherenkov::HostedLayers for Gpu {
    type Object = render::planes::Hosted;

    fn bind_hosted(
        r: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        layer: cherenkov::LayerId,
        object: Self::Object,
        size: kurbo::Size,
    ) {
        r.bind_hosted(surface, layer, object, size);
    }
}

impl cherenkov::BackdropSampling for Gpu {}

impl cherenkov::GpuInstalls for Gpu {}

impl cherenkov::ProjectiveLayers for Gpu {}

// GPU producers (#268): a `GpuProducer` lives at renderer scope, so its
// bindings can sit on any surface of the engine — a persistent surface
// and a transient capture target share the one current frame. Rendered
// and submitted pixels take one path: either kind's bindings sample the
// producer's current `ExternalFrame` through the external pipeline
// (`render::external`, `render::external.wgsl`).
impl cherenkov::GpuContent for Gpu {
    type Content = interop::GpuContentBox;
    type Frame = interop::ExternalFrame;

    fn frame_opaque(frame: &Self::Frame) -> bool {
        frame.alpha() == interop::RgbAlpha::Opaque
    }

    fn add_gpu_producer(r: &mut Self::Renderer, id: cherenkov::ProducerId, content: Self::Content) {
        r.add_gpu_producer(id, content);
    }

    fn add_frame_producer(r: &mut Self::Renderer, id: cherenkov::ProducerId) {
        r.add_frame_producer(id);
    }

    fn bind_gpu_producer(
        r: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        layer: cherenkov::LayerId,
        producer: &cherenkov::GpuProducer<Self>,
        size: (u32, u32),
    ) -> Option<bool> {
        r.bind_gpu_producer(surface, layer, producer, size)
    }

    fn submit_frame(
        r: &mut Self::Renderer,
        id: cherenkov::ProducerId,
        frame: Self::Frame,
    ) -> Vec<(cherenkov::SurfaceId, cherenkov::LayerId)> {
        r.submit_frame(id, frame)
    }

    fn retire_gpu_producer(r: &mut Self::Renderer, id: cherenkov::ProducerId) {
        r.retire_gpu_producer(id);
    }

    fn drain_gpu_producers(
        r: &mut Self::Renderer,
    ) -> Vec<(cherenkov::ProducerId, cherenkov::DrainedProducer<Self>)> {
        r.drain_gpu_producers()
    }
}

impl cherenkov::ShaderPaintCapability for Gpu {
    fn validate_shader(source: &cherenkov::ShaderSource) -> Result<(), cherenkov::ResourceError> {
        render::GpuRenderer::validate_shader(source)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn add_shader(
        r: &mut Self::Renderer,
        id: cherenkov::ShaderId,
        source: cherenkov::ShaderSource,
    ) -> Result<(), cherenkov::ResourceError> {
        r.add_shader(id, &source)
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn add_shader(
        r: &mut Self::Renderer,
        id: cherenkov::ShaderId,
        source: cherenkov::ShaderSource,
    ) -> Result<(), cherenkov::ResourceError> {
        r.add_shader(id, &source).await
    }
    fn remove_shader(r: &mut Self::Renderer, id: cherenkov::ShaderId) {
        r.remove_shader(id);
    }
}

impl cherenkov::Filters for Gpu {
    fn remove_filter(r: &mut Self::Renderer, id: cherenkov::FilterId) {
        r.remove_filter(id);
    }
}
impl<F: filtrate_core::Filter + cherenkov::RenderTransfer> cherenkov::Runs<F> for Gpu {
    fn add_filter(r: &mut Self::Renderer, id: cherenkov::FilterId, filter: F) {
        r.add_filter(id, Box::new(render::filter::FromFilter(filter)));
    }
}
impl cherenkov::Effects for Gpu {
    type Effect = interop::EffectBox;
    fn add_effect(r: &mut Self::Renderer, id: cherenkov::FilterId, effect: Self::Effect) {
        r.add_filter(id, effect.0);
    }
}
impl cherenkov::Backdrop for Gpu {
    fn add_backdrop_group(
        r: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        id: cherenkov::BackdropId,
        spec: cherenkov::BackdropSpec,
    ) {
        r.add_backdrop_group(surface, id, None, spec);
    }
    fn remove_backdrop_group(
        r: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        id: cherenkov::BackdropId,
    ) {
        r.remove_backdrop_group(surface, id);
    }
}
impl<K, F> cherenkov::BackdropRuns<K, F> for Gpu
where
    K: filtrate_core::kind::Kind,
    F: cherenkov::BackdropChain<K> + cherenkov::RenderTransfer,
{
    fn add_filtered_backdrop_group(
        r: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        id: cherenkov::BackdropId,
        filter: F,
        spec: cherenkov::BackdropSpec,
    ) {
        r.add_backdrop_group(
            surface,
            id,
            Some(Box::new(render::filter::FromBackdropChain::<K, F>(
                filter,
                std::marker::PhantomData,
            ))),
            spec,
        );
    }
}
impl cherenkov::BackdropShaders for Gpu {
    fn validate_backdrop_shader(
        source: &cherenkov::BackdropShaderSource,
    ) -> Result<(), cherenkov::ResourceError> {
        render::GpuRenderer::validate_backdrop_shader(source)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn add_backdrop_shader(
        r: &mut Self::Renderer,
        id: cherenkov::BackdropShaderId,
        source: cherenkov::BackdropShaderSource,
    ) -> Result<(), cherenkov::ResourceError> {
        r.add_backdrop_shader(id, &source)
    }
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn add_backdrop_shader(
        r: &mut Self::Renderer,
        id: cherenkov::BackdropShaderId,
        source: cherenkov::BackdropShaderSource,
    ) -> Result<(), cherenkov::ResourceError> {
        r.add_backdrop_shader(id, &source).await
    }
    fn remove_backdrop_shader(r: &mut Self::Renderer, id: cherenkov::BackdropShaderId) {
        r.remove_backdrop_shader(id);
    }
}
