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
use crate::message::{ContentOp, FontData, LayerId, SurfaceId};
use crate::paint::ImageId;
use crate::resource::ResourceId;
use crate::tree::SurfaceTree;

/// The render-thread contract. Implemented by a zero-sized marker type
/// (`Gpu`, `Vello`, `Raster`).
pub trait Backend: Sized + 'static {
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

    /// Creates the render-side state for surface `id`. `waker` is the
    /// surface's host wake-up for render-side completions that land after
    /// a render (a promoted plane's attach on the main queue); it wakes
    /// nothing while the surface is hidden. A source the backend drives on
    /// its own that wakes the host through another callback (a GPU
    /// producer, a filter) gates that wake with a
    /// [`WakeGate`](crate::WakeGate) over the
    /// [`visibility`](crate::CompletionWaker::visibility) of the surfaces
    /// it draws into.
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
    /// [`Frame`], and [`Redraw`] counts only visible surfaces: custom GPU
    /// content and filters on a hidden surface want no redraw. Their host
    /// wakes stop earlier, through their [`WakeGate`](crate::WakeGate),
    /// the moment the host hides the surface. Content ops, installs and
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
    /// returns whether a backend-side source (custom GPU content, an
    /// animated shader) wants another frame.
    ///
    /// # Errors
    /// [`RenderError`] fails the whole `render` call.
    #[cfg(not(target_arch = "wasm32"))]
    fn render(&mut self, frame: &Frame<'_>, stats: &mut FrameStats) -> Result<Redraw, RenderError>;

    /// Executes on the owning JS thread, yielding for browser operations.
    ///
    /// # Errors
    /// Returns the corresponding render or readback error.
    #[cfg(target_arch = "wasm32")]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> impl core::future::Future<Output = Result<Redraw, RenderError>>;

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

/// Whether a backend wants another frame after the current one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Redraw {
    /// Nothing backend-side is animated.
    None,
    /// A backend source (custom GPU content, an animated shader paint)
    /// wants the next frame.
    Wanted {
        /// Inclusive display refresh range in hertz.
        rate: crate::RefreshRange,
    },
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
