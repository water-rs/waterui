//! User GPU work inside the scene: [`GpuContent`], external frames, and the
//! views that host them.
//!
//! The engine owns the device. Content that draws with `wgpu` directly — a
//! particle system, a 3D viewport — implements [`GpuContent`] and the backend
//! installs it on a layer of an engine whose backend hosts GPU content, where
//! it renders into a texture the engine composites like any other layer. The
//! content runs on the render thread: it is `Send`, and whatever it shares
//! with the UI (a pointer position, a simulation parameter) crosses through
//! its own synchronised state.
//!
//! Frames that already exist in GPU memory — a video decoder's or a camera's
//! output — do not draw at all: an [`ExternalFrameSource`] publishes them to
//! an [`ExternalFrameView`], whose layer samples their planes in place.

extern crate alloc;

#[cfg(target_os = "android")]
pub mod device;
pub mod external;
pub mod runtime;
pub use external::{
    ExternalFrameSource, ExternalFrameStream, ExternalFrameView, FrameOutput, FrameReceiver,
    PresentedSize, RetiredOutput,
};
pub use runtime::{
    DeviceLoss, ExternalFrameRenderer, GpuContentRenderer, GpuRuntime, GpuRuntimeError,
    HostedLayerError, SharedGpuContext, preferred_surface_format,
};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::{HostedFrame, RebuildHold, RenderedFrame};

/// The rectangle an IME caret query ([`CaretQuery`]) answers with.
pub use crate::draw::kurbo::Rect;
/// The display a frame is composited for and the presentation time it
/// targets, which [`GpuContentRenderer`] and [`ExternalFrameRenderer`] take
/// per frame, and the offscreen target and readback a content author renders
/// a frame into to read it back.
pub use cherenkov::{Display, FrameTime, Offscreen, OffscreenFormat, Readback};
/// The GPU backend [`OffscreenRenderer`](crate::OffscreenRenderer) and
/// [`GpuRuntime`]'s engines run on.
pub use cherenkov_gpu::Gpu;
/// Importing frames that already live in GPU memory — a decoder's, a
/// camera's or a browser compositor's planes, shader modules, platform
/// textures — onto the engine's device, with the colour description each
/// frame carries.
pub use cherenkov_gpu::interop;

/// The exact `wgpu` this build links: [`Context`] and [`Frame`] hand content
/// its device, queue and target in its types.
pub use wgpu;

/// Plain-bytes casts for the uniform and vertex data content writes to its
/// buffers, so a content author uses the version the framework links.
pub use bytemuck;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;
use core::time::Duration;

use cherenkov::Instant;

use std::sync::Mutex;

use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_core::{Environment, Native, NativeView, View, contract_violation};
use wgpu::{Adapter, Device, Instance, Queue, Texture, TextureFormat, TextureView};

use crate::draw::kurbo;

use crate::input::SurfaceInputEvent;

/// The GPU layer's shared pointer: `Arc` where device handles and frames
/// really travel between threads, `Rc` on WebGPU, whose wgpu handles are
/// `!Send` and where no second thread exists to share them with anyway.
#[cfg(not(target_arch = "wasm32"))]
type Shared<T> = Arc<T>;
/// The GPU layer's shared pointer on WebGPU (see the `Arc` alias).
#[cfg(target_arch = "wasm32")]
type Shared<T> = Rc<T>;

/// Wakes the host for another frame from anywhere: a decoder thread, a
/// browser's compositor callback, a network task.
///
/// The backend supplies it in [`Context`]; requesting a redraw while a frame
/// is already scheduled is a no-op. It wakes the same view instance only: a
/// [`GpuContentView`] that a parent rebuild tears down takes its content with
/// it, and a later request through its handle redraws nothing.
#[derive(Clone)]
pub struct RedrawHandle(Arc<dyn Fn() + Send + Sync>);

impl fmt::Debug for RedrawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RedrawHandle")
    }
}

impl RedrawHandle {
    /// A handle that runs `wake` on every request.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    /// Asks the host for another frame.
    pub fn request_redraw(&self) {
        (self.0)();
    }
}

/// The engine's device, handed to [`GpuContent::setup`].
#[derive(Debug)]
#[non_exhaustive]
pub struct Context<'a> {
    /// The instance the adapter was created from.
    instance: &'a Instance,
    /// The adapter the device was created from.
    adapter: &'a Adapter,
    /// The device the engine renders with.
    device: &'a Device,
    /// Its queue.
    queue: &'a Queue,
    /// The format of the texture the content renders into.
    format: TextureFormat,
    /// Wakes the host for another frame from any thread.
    redraw: RedrawHandle,
}

impl<'a> Context<'a> {
    /// A context over the engine's device: the instance and adapter it was
    /// created from, the device and its queue, the format content renders
    /// into, and the handle that wakes the host for another frame.
    #[must_use]
    pub const fn new(
        instance: &'a Instance,
        adapter: &'a Adapter,
        device: &'a Device,
        queue: &'a Queue,
        format: TextureFormat,
        redraw: RedrawHandle,
    ) -> Self {
        Self {
            instance,
            adapter,
            device,
            queue,
            format,
            redraw,
        }
    }

    /// The instance the adapter was created from.
    #[must_use]
    pub const fn instance(&self) -> &'a Instance {
        self.instance
    }

    /// The adapter the device was created from.
    #[must_use]
    pub const fn adapter(&self) -> &'a Adapter {
        self.adapter
    }

    /// The device the engine renders with.
    #[must_use]
    pub const fn device(&self) -> &'a Device {
        self.device
    }

    /// Its queue.
    #[must_use]
    pub const fn queue(&self) -> &'a Queue {
        self.queue
    }

    /// The format of the texture the content renders into.
    #[must_use]
    pub const fn format(&self) -> TextureFormat {
        self.format
    }

    /// Wakes the host for another frame from any thread.
    #[must_use]
    pub const fn redraw(&self) -> &RedrawHandle {
        &self.redraw
    }
}

/// One frame of a [`GpuContent`]: the texture to draw into and the clock.
#[derive(Debug)]
pub struct Frame<'a> {
    /// The device the engine renders with.
    pub device: &'a Device,
    /// Its queue.
    pub queue: &'a Queue,
    /// The texture the content draws into.
    pub texture: &'a Texture,
    /// A view of the whole texture.
    pub view: &'a TextureView,
    /// The texture's format.
    pub format: TextureFormat,
    /// The texture's width in pixels.
    pub width: u32,
    /// The texture's height in pixels.
    pub height: u32,
    /// Pixels per logical point.
    pub scale: f32,
    /// Time since the content was set up.
    pub elapsed: Duration,
    /// Time since the previous frame.
    pub delta: Duration,
    /// When the platform will present this frame, where it reports a target
    /// presentation time: a display link's target timestamp, a frame
    /// scheduler's expected presentation time. `None` on platforms that
    /// report none.
    pub presentation_time: Option<Instant>,
    redraw: bool,
}

impl<'a> Frame<'a> {
    /// A frame that has not yet requested a redraw.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        device: &'a Device,
        queue: &'a Queue,
        texture: &'a Texture,
        view: &'a TextureView,
        format: TextureFormat,
        (width, height): (u32, u32),
        scale: f32,
        (elapsed, delta): (Duration, Duration),
        presentation_time: Option<Instant>,
    ) -> Self {
        Self {
            device,
            queue,
            texture,
            view,
            format,
            width,
            height,
            scale,
            elapsed,
            delta,
            presentation_time,
            redraw: false,
        }
    }

    /// Asks for another frame after this one.
    pub const fn request_redraw(&mut self) {
        self.redraw = true;
    }

    /// Whether the content asked for another frame.
    #[must_use]
    pub const fn redraw_requested(&self) -> bool {
        self.redraw
    }
}

/// GPU work the engine composites as a layer.
pub trait GpuContent: Send + 'static {
    /// Creates pipelines and resources against the engine's device.
    ///
    /// Called on the render thread before the first [`render`](Self::render),
    /// and again on each new device after the GPU runtime replaces a lost one.
    /// Each call replaces every device resource the previous call created:
    /// resources from the old device must not be used after it.
    fn setup(&mut self, gpu: &Context<'_>);

    /// Draws one frame into `frame`'s texture.
    ///
    /// Call [`Frame::request_redraw`] when the content animates and needs
    /// the next frame too.
    fn render(&mut self, frame: &mut Frame<'_>);

    /// Whether every pixel the content draws is opaque, letting the engine
    /// skip blending underneath it.
    fn is_opaque(&self) -> bool {
        false
    }

    /// The size the content is naturally, in logical points; `None` takes
    /// whatever the layout gives.
    ///
    /// Content whose natural size only resolves asynchronously — a decoder
    /// that learns the stream dimensions after setup — leaves this `None` and
    /// answers [`measure`](Self::measure) instead.
    fn intrinsic_size(&self) -> Option<Size> {
        None
    }

    /// Measures the content against a layout proposal.
    ///
    /// The default answers [`intrinsic_size`](Self::intrinsic_size) when it
    /// knows one and fills the proposal otherwise. Override it for content
    /// whose size resolves at runtime: the measurement is read again for every
    /// layout pass, so an asynchronous source reports its size as soon as it
    /// has one — announce the change through [`Context::redraw`].
    ///
    /// This is layout-only and must not touch GPU or render state.
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_by_intrinsic_size(self.intrinsic_size(), proposal)
    }

    /// Which dynamic range the content prefers its presentation target to
    /// carry.
    ///
    /// - `Some(true)`: prefer an HDR target.
    /// - `Some(false)`: prefer an SDR target.
    /// - `None` — the default — means follow the host's surrounding policy.
    ///
    /// The view's own [`prefer_hdr_surface`](GpuContentView::prefer_hdr_surface)
    /// overrides this answer.
    fn preferred_surface_hdr(&self) -> Option<bool> {
        None
    }
}

/// The default measurement of GPU-backed views: the intrinsic size when one
/// is known, the proposal filled otherwise.
fn measure_by_intrinsic_size(intrinsic: Option<Size>, proposal: ProposalSize) -> ViewDimensions {
    ViewDimensions::new(intrinsic.unwrap_or_else(|| {
        Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        )
    }))
}

/// A UI-side handler for input the backend routes to a [`GpuContentView`]
/// or an [`ExternalFrameView`].
pub type InputHandler = Rc<dyn Fn(&SurfaceInputEvent)>;

/// A UI-side hook the backend runs once per frame, before the content renders.
///
/// Producers confined to the UI thread — a browser engine whose objects are
/// not `Send` — pump their work here and hand the results to the content
/// through their own channels.
pub type FrameHook = Rc<dyn Fn()>;

/// A UI-side query for where the content's text caret is, in logical
/// view-local coordinates, so a host can place its input-method panel.
pub type CaretQuery = Rc<dyn Fn() -> Option<kurbo::Rect>>;

/// The UI-thread hooks a GPU-backed view carries for its host: input
/// routing, the per-frame pump and the caret query, shared by
/// [`GpuContentView`] and [`ExternalFrameView`].
#[derive(Default)]
struct SurfaceHooks {
    input: Option<InputHandler>,
    frame: Option<FrameHook>,
    caret: Option<CaretQuery>,
}

impl SurfaceHooks {
    const fn wants_input_events(&self) -> bool {
        self.input.is_some()
    }

    fn input(&self, event: &SurfaceInputEvent) {
        if let Some(handler) = &self.input {
            handler(event);
        }
    }

    fn frame(&self) {
        if let Some(hook) = &self.frame {
            hook();
        }
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.caret.as_ref().and_then(|query| query())
    }
}

/// A view whose pixels come from [`GpuContent`].
///
/// # Content lifetime
///
/// The view owns one [`GpuContent`] instance for its whole lifetime, and
/// [`GpuContent::setup`] is where that instance creates its persistent GPU
/// resources. When a parent rebuild tears the view down, the content goes
/// with it and the rebuilt view starts a new instance. Content state does not
/// move into hidden shared caches to survive that.
///
/// # Layout Behavior
///
/// Stretches on both axes unless the content reports an intrinsic size, in
/// which case it is that size and does not stretch.
pub struct GpuContentView {
    content: Option<Box<dyn GpuContent>>,
    content_handle: Option<GpuContentHandle>,
    intrinsic_size: Option<Size>,
    opaque: bool,
    surface_prefers_hdr: Option<bool>,
    hooks: SurfaceHooks,
    label: Option<String>,
    value: Option<String>,
}

impl fmt::Debug for GpuContentView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContentView")
            .field("intrinsic_size", &self.intrinsic_size)
            .field("opaque", &self.opaque)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl GpuContentView {
    /// A view drawing `content`.
    #[must_use]
    pub fn new(content: impl GpuContent) -> Self {
        Self {
            intrinsic_size: content.intrinsic_size(),
            opaque: content.is_opaque(),
            content: Some(Box::new(content)),
            content_handle: None,
            surface_prefers_hdr: None,
            hooks: SurfaceHooks::default(),
            label: None,
            value: None,
        }
    }

    /// Receives pointer, keyboard and gesture events the backend routes to
    /// this view.
    #[must_use]
    pub fn on_input(mut self, handler: impl Fn(&SurfaceInputEvent) + 'static) -> Self {
        self.hooks.input = Some(Rc::new(handler));
        self
    }

    /// Runs `hook` on the UI thread once per frame before the content renders.
    #[must_use]
    pub fn on_frame(mut self, hook: impl Fn() + 'static) -> Self {
        self.hooks.frame = Some(Rc::new(hook));
        self
    }

    /// Answers where the content's text caret is, for input-method panels.
    #[must_use]
    pub fn on_ime_caret(mut self, query: impl Fn() -> Option<kurbo::Rect> + 'static) -> Self {
        self.hooks.caret = Some(Rc::new(query));
        self
    }

    /// The content's text caret, if it has one right now.
    #[must_use]
    pub fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.hooks.ime_caret()
    }

    /// Runs the per-frame UI hook, if any.
    pub fn frame(&self) {
        self.hooks.frame();
    }

    /// The accessibility name of the content.
    #[must_use]
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The accessibility value of the content.
    #[must_use]
    pub fn described(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Whether this view takes input events.
    #[must_use]
    pub const fn wants_input_events(&self) -> bool {
        self.hooks.wants_input_events()
    }

    /// Routes an input event to the content's handler.
    pub fn input(&self, event: &SurfaceInputEvent) {
        self.hooks.input(event);
    }

    /// The natural size of the content, if it has one.
    #[must_use]
    pub const fn intrinsic_size(&self) -> Option<Size> {
        self.intrinsic_size
    }

    /// Measures this view's content against a layout proposal.
    ///
    /// Unlike [`intrinsic_size`](Self::intrinsic_size) — the snapshot taken at
    /// [`GpuContentView::new`] — this asks the live content, so a source whose
    /// size resolves asynchronously reports it once it has one.
    #[must_use]
    pub fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.content_handle.as_ref().map_or_else(
            || {
                self.content.as_ref().map_or_else(
                    || measure_by_intrinsic_size(None, proposal),
                    |content| content.measure(proposal),
                )
            },
            |handle| handle.measure(proposal),
        )
    }

    /// Whether the content is opaque.
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        self.opaque
    }

    /// Prefer HDR presentation formats for this content even when the
    /// surrounding platform style is SDR.
    ///
    /// This overrides the surrounding platform style for this surface only.
    #[must_use]
    pub const fn prefer_hdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(true);
        self
    }

    /// Prefer SDR presentation formats for this content even when HDR is
    /// available.
    ///
    /// This overrides the surrounding platform style for this surface only.
    #[must_use]
    pub const fn prefer_sdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(false);
        self
    }

    /// Resolves this view's explicit or content-provided HDR preference.
    ///
    /// `None` means follow the surrounding platform style.
    #[must_use]
    pub fn resolved_hdr_preference(&self) -> Option<bool> {
        self.surface_prefers_hdr.or_else(|| {
            self.content_handle
                .as_ref()
                .and_then(GpuContentHandle::preferred_surface_hdr)
                .or_else(|| {
                    self.content
                        .as_ref()
                        .and_then(|content| content.preferred_surface_hdr())
                })
        })
    }

    /// The accessibility name.
    #[must_use]
    pub fn accessibility_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The accessibility value.
    #[must_use]
    pub fn accessibility_value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Transfers this producer to a Cherenkov GPU layer.
    ///
    /// Asynchronous producer work wakes the host through the wake of each
    /// engine surface that draws the layer. The UI hooks and input handlers
    /// stay in this view.
    ///
    /// Content already taken with [`take_content`](Self::take_content) is a
    /// contract violation: the layer draws nothing.
    #[must_use]
    pub fn take_engine_content(&mut self) -> cherenkov_gpu::interop::GpuContentBox {
        cherenkov_gpu::interop::GpuContentBox::new(self.engine_content())
    }

    /// A shareable handle to this view's content for the engine.
    ///
    /// Every call answers a handle to the *same* content object: a host that
    /// rebuilds its engine layer after device loss re-installs a fresh handle
    /// so the engine's setup runs the one content on the new device, keeping
    /// the state the content had accumulated.
    ///
    /// Content already taken with [`take_content`] is a contract violation:
    /// the handle is to content that draws nothing.
    ///
    /// [`take_content`]: Self::take_content
    pub fn engine_content(&mut self) -> GpuContentHandle {
        let content = &mut self.content;
        self.content_handle
            .get_or_insert_with(|| GpuContentHandle::new(take_once(content)))
            .clone()
    }

    /// Takes the content out for installation on a layer.
    ///
    /// Taking it twice is a contract violation, a view being installed once:
    /// the second call answers content that draws nothing.
    #[must_use]
    pub fn take_content(&mut self) -> Box<dyn GpuContent> {
        take_once(&mut self.content)
    }
}

/// The content in `slot`, or [`Vacant`] when it was already taken.
fn take_once(slot: &mut Option<Box<dyn GpuContent>>) -> Box<dyn GpuContent> {
    slot.take().unwrap_or_else(|| {
        contract_violation!("GpuContentView content installed twice; the layer draws nothing");
        Box::new(Vacant)
    })
}

/// Content that draws nothing: what a view's second installation gets.
struct Vacant;

impl GpuContent for Vacant {
    fn setup(&mut self, _gpu: &Context<'_>) {}

    fn render(&mut self, _frame: &mut Frame<'_>) {}
}

impl NativeView for GpuContentView {
    fn stretch_axis(&self) -> StretchAxis {
        if self.intrinsic_size.is_some() {
            StretchAxis::None
        } else {
            StretchAxis::Both
        }
    }
}

impl View for GpuContentView {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        NativeView::stretch_axis(self)
    }
}

/// Shared access to a [`GpuContent`] after the engine took it.
///
/// The engine drives the content through this handle; the view keeps a clone
/// so UI-side queries — layout measurement, the HDR preference — still reach
/// the live content, and so a rebuilt engine can re-install the same object
/// after device loss.
pub struct GpuContentHandle(Arc<Mutex<Box<dyn GpuContent>>>);

impl fmt::Debug for GpuContentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContentHandle").finish_non_exhaustive()
    }
}

impl Clone for GpuContentHandle {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl GpuContentHandle {
    fn new(content: Box<dyn GpuContent>) -> Self {
        Self(Arc::new(Mutex::new(content)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn GpuContent>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Measures the content against a layout proposal, from the UI thread.
    #[must_use]
    pub fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.lock().measure(proposal)
    }

    /// The content's HDR preference for its presentation target.
    #[must_use]
    pub fn preferred_surface_hdr(&self) -> Option<bool> {
        self.lock().preferred_surface_hdr()
    }
}

/// Projects `WaterUI`'s producer contract onto the engine's render context.
impl cherenkov_gpu::interop::GpuContent for GpuContentHandle {
    fn setup(
        &mut self,
        gpu: &cherenkov_gpu::interop::wgpu::Context<'_>,
    ) -> impl core::future::Future<Output = ()> {
        let redraw = gpu.redraw.clone();
        self.lock().setup(&Context {
            instance: gpu.instance,
            adapter: gpu.adapter,
            device: gpu.device,
            queue: gpu.queue,
            format: gpu.format,
            redraw: RedrawHandle::new(move || redraw.request_redraw()),
        });
        core::future::ready(())
    }

    fn render(&mut self, gpu: &mut cherenkov_gpu::interop::wgpu::Frame<'_>) {
        let mut frame = Frame::new(
            gpu.device,
            gpu.queue,
            gpu.texture,
            gpu.view,
            gpu.format,
            (gpu.width, gpu.height),
            gpu.scale,
            (gpu.elapsed, gpu.delta),
            gpu.presentation_time,
        );
        self.lock().render(&mut frame);
        if frame.redraw_requested() {
            gpu.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Content that reports an intrinsic size, so it differs from [`Vacant`].
    struct Sized;

    impl GpuContent for Sized {
        fn setup(&mut self, _gpu: &Context<'_>) {}

        fn render(&mut self, _frame: &mut Frame<'_>) {}

        fn intrinsic_size(&self) -> Option<Size> {
            Some(Size::new(4.0, 4.0))
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "GpuContentView content installed twice")
    )]
    fn content_taken_twice_draws_nothing() {
        let mut view = GpuContentView::new(Sized);
        assert!(view.take_content().intrinsic_size().is_some());
        assert!(view.take_content().intrinsic_size().is_none());
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "GpuContentView content installed twice")
    )]
    fn an_engine_handle_after_taking_the_content_draws_nothing() {
        let mut view = GpuContentView::new(Sized);
        let _ = view.take_content();
        let handle = view.engine_content();
        assert!(handle.lock().intrinsic_size().is_none());
    }
}
