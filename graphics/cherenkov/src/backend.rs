//! The render-thread contract every backend implements.
//!
//! The `cherenkov` crate owns the whole front end and the render thread's
//! loop. A backend crate supplies the render side only: a config type, a
//! [`Backend`] implementation, its capability implementations and an
//! `interop` module.

use crate::WorkingColor;

/// Values crossing the native render-thread boundary. On wasm32 the render
/// executor is local to the creating JS thread, so transfer is not required.
#[cfg(not(target_arch = "wasm32"))]
pub trait RenderTransfer: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> RenderTransfer for T {}

/// Values retained by the local browser executor; they need not be `Send`.
#[cfg(target_arch = "wasm32")]
pub trait RenderTransfer {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> RenderTransfer for T {}

use rustc_hash::FxHashSet;

use crate::Picture;
use crate::config::{MemoryUsage, Pressure};
use crate::error::{EngineError, RenderError, ResourceError, SurfaceError};
use crate::frame::{FrameId, FrameStats, FrameTime, FrameTiming, Readback};
use crate::glyph::FontId;
use crate::image::ImageUpload;
use crate::message::FontData;
use crate::paint::ImageId;
use cherenkov_record::{ContentOp, LayerId, SurfaceId};
use cherenkov_record::{ResourceId, SurfaceTree};

/// The render-thread contract, a zero-sized marker type (`Gpu`, `Raster`).
///
/// The [`cherenkov_record::Target`] the layer tree is generic over: an
/// engine backend's queue is the engine's
/// [`EngineQueue`](crate::EngineQueue) and its install payload the
/// render-side [`InstallOp`](crate::message::InstallOp), which is
/// render-thread transferable like every other render op.
pub trait Backend:
    Sized
    + cherenkov_record::Target<
        Queue = crate::surface::EngineQueue<Self>,
        Install = crate::message::InstallOp<Self>,
    > + 'static
{
    /// The backend's configuration type.
    type Config: RenderTransfer + 'static;
    /// Provenance for reports.
    type Info: Clone + Send + 'static;
    /// A surface target: [`Offscreen`](crate::Offscreen) or an interop
    /// window target.
    type Target: From<crate::Offscreen> + RenderTransfer + 'static;
    /// The render-thread state; never leaves that thread.
    type Renderer: Renderer<Target = Self::Target>;

    /// Runs on the render thread, once. Creates the device or worker pool.
    ///
    /// # Errors
    /// [`EngineError`] when the device or pool cannot be created.
    #[cfg(not(target_arch = "wasm32"))]
    fn init(config: Self::Config) -> Result<(Self::Renderer, Self::Info), EngineError>;

    /// Initializes on the owning JS thread without blocking its event loop.
    ///
    /// # Errors
    /// Returns a backend initialization error.
    #[cfg(target_arch = "wasm32")]
    fn init(
        config: Self::Config,
    ) -> impl core::future::Future<Output = Result<(Self::Renderer, Self::Info), EngineError>>;
}

/// Everything the render loop asks of a backend. Every method runs on the
/// render thread.
pub trait Renderer: 'static {
    /// A surface target.
    type Target;
    /// A font [`Renderer::prepare_font`] validated, ready for
    /// [`Renderer::add_font`].
    type Font: RenderTransfer + 'static;
    /// The part of a rendered frame the render thread hands to the
    /// frame's awaiting caller: on platforms whose system compositor owns
    /// presentation, the work that must land on the platform's main
    /// thread for the frame's parts to present (Apple window surfaces:
    /// the `CATransaction` of layer geometry and drawable presents).
    /// [`Engine::render`](crate::Engine::render) applies it on the
    /// awaiting thread right after the reply arrives, so a frame's
    /// present is ordered before the next frame's acquire. `()` where a
    /// frame commits nothing off the render thread.
    type FrameCommit: RenderTransfer + 'static;

    /// Applies the frame's [`Self::FrameCommit`] on the awaiting caller's
    /// thread. Called by [`Engine::render`](crate::Engine::render) before
    /// it returns. A backend whose commits are main-thread-bound asserts
    /// the caller is on that thread here: a render whose surfaces present
    /// on the platform main thread must be awaited on it.
    #[cfg(not(target_arch = "wasm32"))]
    fn apply_frame_commit(_commit: Self::FrameCommit) {}

    /// Creates the render-side state for surface `id`. `waker` is the
    /// surface's host wake-up: for render-side completions that land after
    /// a render (a promoted plane's attach on the main queue), and for the
    /// redraw requests of the sources the backend drives on its own (a GPU
    /// producer, a filter), which keep the wakers of the surfaces they draw
    /// into in a [`SurfaceWakes`](crate::SurfaceWakes). It wakes nothing
    /// while the surface is hidden.
    ///
    /// # Errors
    /// [`SurfaceError`] when the target cannot be drawn.
    fn create_surface(
        &mut self,
        id: SurfaceId,
        target: Self::Target,
        waker: crate::CompletionWaker,
    ) -> Result<SurfaceInfo, SurfaceError>;

    /// Resizes a surface's target.
    fn resize_surface(&mut self, id: SurfaceId, size: (u32, u32));

    /// The host announced surface `id`'s [`Visibility`]; called only when
    /// it changes.
    ///
    /// While a surface is hidden the render loop leaves it out of every
    /// [`Frame`], and [`FrameRedraw`] counts only visible surfaces: custom
    /// GPU content and filters on a hidden surface want no redraw. Their
    /// host wakes stop earlier, through the surface's own waker, the moment
    /// the host hides the surface. Content ops, installs and
    /// resource changes still arrive while it is hidden. When it becomes
    /// visible again the next frame lists it, and a producer or filter
    /// that asked for a redraw while it was hidden is drawn then.
    fn set_visibility(&mut self, id: SurfaceId, visibility: Visibility);

    /// Destroys a surface's render-side state.
    fn destroy_surface(&mut self, id: SurfaceId);

    /// Validates font data on the caller thread, before anything is
    /// queued, and prepares it for [`Renderer::add_font`]. Every check a
    /// font needs runs here, so registration cannot fail later.
    ///
    /// # Errors
    /// [`ResourceError`] when the data cannot be used.
    fn prepare_font(font: FontData) -> Result<Self::Font, ResourceError>;

    /// Registers a font [`Renderer::prepare_font`] validated.
    fn add_font(&mut self, id: FontId, font: Self::Font);

    /// Unregisters a font no installed content draws any more.
    fn remove_font(&mut self, id: FontId);

    /// The largest image this renderer admits, in each dimension and in
    /// total texels: the device's texture limit, or the per-image share
    /// of the backend's memory budget. The engine reads it once, right
    /// after [`Backend::init`] returns the renderer, and it must not
    /// change afterwards: [`Engine::image`](crate::Engine::image) and
    /// [`Image::replace`](crate::Image::replace) check it on the calling
    /// thread and reject what it does not admit before anything is
    /// queued.
    fn image_limits(&self) -> crate::ImageLimits;

    /// Registers an image. A rejection fails every later render that
    /// draws the image with [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError`] when the upload cannot be used.
    fn add_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError>;

    /// Replaces a registered image's pixels behind the same id: every
    /// recording naming `id` samples the new pixels from the next render
    /// on. Same dimensions reuse the backing storage; different dimensions
    /// reallocate it and refresh every cache that referenced the old one.
    /// After a replacement the render loop marks changed exactly the
    /// surfaces for which [`Renderer::samples`] answers true.
    ///
    /// # Errors
    /// [`ResourceError`] when the upload cannot be used; the image keeps
    /// its previous pixels, and renders that draw it fail with
    /// [`RenderError::Rejected`] until a replacement succeeds.
    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError>;

    /// Whether any layer content on `surface`, with its slot updates
    /// applied and nested pictures included, samples `resource`: a font,
    /// an image or a shader. Content never names a backdrop shader; the
    /// render loop finds those in the layer tree.
    ///
    /// The render loop frees a released resource only once this answers
    /// false for every surface, so content installed on a surface never
    /// names a resource its `remove_*` already ran for.
    fn samples(&self, surface: SurfaceId, resource: ResourceId) -> bool;

    /// Unregisters an image no installed content draws any more.
    fn remove_image(&mut self, id: ImageId);

    /// Replaces or updates a layer's content, or clears it. Returns the
    /// previous picture when replaced or cleared, and `None` for updates or
    /// when the layer held no picture.
    fn set_content(
        &mut self,
        surface: SurfaceId,
        layer: LayerId,
        content: Option<ContentOp>,
    ) -> Option<Picture>;

    /// The layer is gone: drop every cache keyed on it.
    fn remove_layer(&mut self, surface: SurfaceId, layer: LayerId);

    /// Renders every surface in `frame` whose tree or content changed;
    /// returns the surfaces a backend-side source (custom GPU content, an
    /// animated shader, a pending presentation) wants the next frame for,
    /// each with its refresh class.
    ///
    /// # Errors
    /// [`RenderError`] fails the whole `render` call.
    #[cfg(not(target_arch = "wasm32"))]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<(FrameRedraw, Self::FrameCommit), RenderError>;

    /// Executes on the owning JS thread, yielding for browser operations.
    ///
    /// # Errors
    /// Returns the corresponding render or readback error.
    #[cfg(target_arch = "wasm32")]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> impl core::future::Future<Output = Result<(FrameRedraw, Self::FrameCommit), RenderError>>;

    /// Returns all GPU timings accumulated since the previous call,
    /// oldest first. Timings stay on the renderer rather than being
    /// returned by `render`; this tooling call waits for frames still on
    /// the GPU. A backend that times frames synchronously, or not at all,
    /// may have nothing outstanding.
    ///
    /// # Errors
    /// [`RenderError::Timeout`] when the GPU makes no progress for a
    /// wait window, and [`RenderError::Readback`] when the timing buffers
    /// cannot be read.
    #[cfg(not(target_arch = "wasm32"))]
    fn finish_timings(&mut self) -> Result<Vec<FrameTiming>, RenderError> {
        Ok(Vec::new())
    }

    /// Awaits outstanding GPU timing readbacks without blocking JavaScript
    /// and returns all accumulated timings, oldest first. Render calls
    /// never return GPU timings.
    ///
    /// # Errors
    /// Returns a timeout or readback error.
    #[cfg(target_arch = "wasm32")]
    fn finish_timings(
        &mut self,
    ) -> impl core::future::Future<Output = Result<Vec<FrameTiming>, RenderError>> {
        core::future::ready(Ok(Vec::new()))
    }

    /// Reads back a surface's pixels.
    ///
    /// # Errors
    /// [`RenderError::NotReadable`] for non-readable surfaces,
    /// [`RenderError::Readback`] on failure.
    #[cfg(not(target_arch = "wasm32"))]
    fn readback(&mut self, surface: SurfaceId) -> Result<Readback, RenderError>;

    /// Executes on the owning JS thread, yielding for browser operations.
    ///
    /// # Errors
    /// Returns the corresponding render or readback error.
    #[cfg(target_arch = "wasm32")]
    fn readback(
        &mut self,
        surface: SurfaceId,
    ) -> impl core::future::Future<Output = Result<Readback, RenderError>>;

    /// Layers whose last successful render handed every running property
    /// track to the system compositor. The backend must withdraw
    /// ownership on demotion, an unsupported track, or failed presentation.
    /// Owned tracks remain in the tree for sampling and retargeting, but do
    /// not request display-link frames. Recorded operand animations are
    /// independent and always remain engine-driven.
    fn owned_animations(&self, _surface: SurfaceId) -> &[LayerId] {
        &[]
    }

    /// The backend's current memory usage.
    fn memory(&self) -> MemoryUsage;

    /// Releases memory under system `pressure`.
    fn trim(&mut self, pressure: Pressure);

    /// Submits the native external-frame releases retirement work has
    /// queued since the last submission. An external frame's release
    /// must be signalled even when no frame is being drawn, so the
    /// render loop calls this after every retirement drain — including
    /// one a retirement itself woke — and an idle engine still releases
    /// the lease. A renderer that imports no native frames queues
    /// nothing to submit.
    fn submit_native_releases(&mut self);
}

/// The render-side facts of a surface, answered by
/// [`Renderer::create_surface`].
#[derive(Clone, Copy, Debug)]
pub struct SurfaceInfo {
    /// Largest supported surface dimension.
    pub max_dimension: u32,
    /// The drawable size in pixels.
    pub size: (u32, u32),
    /// Whether [`Renderer::readback`] works on the surface.
    pub readable: bool,
    /// Whether the surface presents to a display — a window with a
    /// swapchain. Only a presenting surface carries pending-presentation
    /// state: a [`Display`] update on any other target never marks a
    /// frame for presentation.
    pub presents: bool,
}

/// The per-surface refresh requests a backend reported for a frame.
///
/// A backend-side source — a custom GPU content, an animated shader
/// paint, a filter, a pending native presentation — wants the next frame
/// for the surface it draws into, never for the frame at large: the
/// requests keep that identity so the engine can publish each surface's
/// own deadline through [`Surface::next_frame`](crate::Surface::next_frame).
/// Surfaces carrying no request do not appear in the collection.
///
/// Backed by a keyed map: recording, combining and reading back a
/// surface's request all stay O(1), so per-frame bookkeeping is linear in
/// the surface count. A renderer builds a fresh collection for every
/// render.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameRedraw {
    /// The unioned request per surface that asked for the next frame.
    requests: rustc_hash::FxHashMap<SurfaceId, crate::RefreshRange>,
}

impl FrameRedraw {
    /// Records that `surface` wants the next frame at `rate`, unioning
    /// repeated requests for the same surface into one entry.
    pub fn request(&mut self, surface: SurfaceId, rate: crate::RefreshRange) {
        self.requests
            .entry(surface)
            .and_modify(|kept| *kept = union_rate(kept.clone(), rate.clone()))
            .or_insert(rate);
    }

    /// Removes every request.
    pub fn clear(&mut self) {
        self.requests.clear();
    }

    /// The request `surface` made, when it asked for the next frame.
    #[must_use]
    pub fn for_surface(&self, surface: SurfaceId) -> Option<&crate::RefreshRange> {
        self.requests.get(&surface)
    }

    /// The requests, one per surface — unordered.
    pub fn iter(&self) -> impl Iterator<Item = (&SurfaceId, &crate::RefreshRange)> {
        self.requests.iter()
    }

    /// The union of every request's refresh range — the frame's aggregate
    /// backend demand.
    #[must_use]
    pub fn rate(&self) -> Option<crate::RefreshRange> {
        self.requests.values().fold(None, |rate, next| {
            Some(rate.map_or_else(|| next.clone(), |rate| union_rate(rate, next.clone())))
        })
    }
}

/// The union of two refresh ranges: the slowest floor, the fastest
/// ceiling — a frame serving both rates ticks at the covered range.
pub fn union_rate(a: crate::RefreshRange, b: crate::RefreshRange) -> crate::RefreshRange {
    (*a.start()).min(*b.start())..=(*a.end()).max(*b.end())
}

/// One frame's render input: every live surface with its sampled tree.
#[derive(Debug)]
pub struct Frame<'a> {
    /// The render's id; a backend that draws any surface reports it in
    /// [`FrameStats::frame`] and tags the frame's [`FrameTiming`] with it.
    pub id: FrameId,
    /// The frame's presentation time.
    pub time: FrameTime,
    /// The surfaces to consider; render those with `changed` set.
    pub surfaces: &'a [SurfaceFrame<'a>],
}

/// One surface's input to [`Renderer::render`].
#[derive(Debug)]
pub struct SurfaceFrame<'a> {
    /// The surface id.
    pub id: SurfaceId,
    /// The drawable size in pixels.
    pub size: (u32, u32),
    /// The display properties.
    pub display: Display,
    /// The clear colour.
    pub clear: WorkingColor,
    /// Whether a property op, a content op or an animation step touched the
    /// surface since the last render.
    pub changed: bool,
    /// The layers that received a new external frame this frame, when those
    /// installs are the surface's only change: `Some` with a non-empty set
    /// only then — `changed` still reports them — and `None` when anything
    /// else changed or nothing did (#90). A planes-capable backend can
    /// present the new frames to the layers' system planes without
    /// re-rendering the surface; every other backend ignores this.
    pub plane_frames: Option<&'a FxHashSet<LayerId>>,
    /// Whether the window should present this frame even when `changed` is
    /// false — a headroom update reaches the swapchain without touching the
    /// layer tree or any content cache (#98).
    pub present_pending: bool,
    /// Whether the host announced the surface moved to another display
    /// since the previous frame ([`Surface::display_moved`]). A presenting
    /// backend re-enumerates the surface's capabilities on it — a
    /// headroom-only [`Display`] update never triggers re-enumeration
    /// (#98).
    ///
    /// [`Surface::display_moved`]: crate::Surface::display_moved
    pub display_moved: bool,
    /// The sampled layer tree.
    pub tree: &'a SurfaceTree,
}

/// Whether the user can see a surface's output.
///
/// The host announces it with
/// [`Surface::visibility`](crate::Surface::visibility) from the platform's
/// visibility signal: a minimized or fully occluded window, a backgrounded
/// app, a view detached from its window or a hidden document is
/// [`Hidden`](Self::Hidden).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// The surface is drawn and presented as usual.
    #[default]
    Visible,
    /// The surface is neither drawn nor presented, and nothing on it asks
    /// for a frame; its state changes are still accepted.
    Hidden,
}

/// The properties of the display a surface presents on. Headroom and scale
/// belong to the display, so the host sets them when they change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    /// Device pixels per logical pixel.
    pub scale: f64,
    /// HDR headroom: the ratio of peak white to SDR white.
    pub headroom: f32,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            scale: 1.0,
            headroom: 1.0,
        }
    }
}
