//! User GPU work inside the scene: [`GpuContent`](crate::GpuContent),
//! external frames, and the views that host them.
//!
//! The engine owns the device. Content that draws with `wgpu` directly — a
//! particle system, a 3D viewport — implements [`GpuContent`](crate::GpuContent)
//! and the backend installs it on a layer of an engine whose backend hosts GPU
//! content, where it renders into a texture the engine composites like any
//! other layer. The content runs on the render thread: it is `Send`, and
//! reactive UI parameters cross through
//! [`GpuContent::update`](crate::GpuContent::update) and
//! [`GpuView::params`](crate::GpuView::params).
//!
//! Frames that already exist in GPU memory — a video decoder's or a camera's
//! output — do not draw at all: an [`ExternalFrameSource`] publishes them to
//! an [`ExternalFrameView`], whose layer samples their planes in place.

extern crate alloc;

pub(crate) mod content;
#[cfg(target_os = "android")]
pub mod device;
pub mod external;
pub(crate) mod leaf;
mod legacy;
pub mod runtime;
pub use external::{
    ExternalFrameSource, ExternalFrameStream, ExternalFrameView, FrameOutput, FrameReceiver,
    PresentedSize, RetiredOutput,
};
// Temporary compatibility shim for split crates (`waterui-particle`,
// `waterui-map-gpu`, `waterui-video-gpu`) during the `#4419` transition;
// removed once all three crates migrate to `waterui_graphics::{GpuContent, GpuView}`.
#[doc(hidden)]
pub use legacy::{GpuContent, GpuView};
pub use runtime::{
    DeviceLoss, ExternalFrameRenderer, GpuContentRenderer, GpuRuntime, GpuRuntimeError,
    HostedLayerError, SharedGpuContext, preferred_surface_format,
};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::{HostedFrame, RebuildHold, RenderedFrame, Unrecoverable};

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
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;

use cherenkov::Instant;
use nami::watcher::BoxWatcherGuard;

use std::sync::Mutex;

use waterui_core::layout::{ProposalSize, Size, ViewDimensions};
use waterui_frame_rate::FrameRate;
use wgpu::{Adapter, Device, Instance, Queue, Texture, TextureFormat, TextureView};

use crate::draw::kurbo;
use crate::gpu::content::ErasedGpuContent;
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
/// [`GpuView`](crate::GpuView) that a parent rebuild tears down takes its
/// content with it, and a later request through its handle redraws nothing.
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

/// The engine's device, handed to [`GpuContent::setup`](crate::GpuContent::setup).
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

/// Pending source-rate change recorded on [`Frame::set_source_rate`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRateUpdate {
    /// Leave the producer's latched source rate as-is.
    Unchanged,
    /// Clear the producer's latched source rate.
    Clear,
    /// Latch `FrameRate` as the producer's source cadence.
    Set(waterui_frame_rate::FrameRate),
}

/// Per-frame context passed into [`GpuContent::render`](crate::GpuContent::render).
#[derive(Debug)]
pub struct Frame<'a> {
    /// Engine-owned device.
    device: &'a Device,
    /// Engine-owned queue.
    queue: &'a Queue,
    /// Offscreen render target for this frame.
    texture: &'a Texture,
    /// Color attachment view of the render target texture.
    view: &'a TextureView,
    /// Format of the render target texture.
    format: TextureFormat,
    /// Width of the render target texture in physical pixels.
    width: u32,
    /// Height of the render target texture in physical pixels.
    height: u32,
    /// Display scale factor.
    scale: f32,
    /// Elapsed time since the producer's first rendered frame.
    elapsed: Duration,
    /// Time delta since the previous frame.
    delta: Duration,
    /// The platform's target presentation time for this frame, when available.
    presentation_time: Option<Instant>,
    redraw_rate: Option<FrameRate>,
    source_rate_update: SourceRateUpdate,
}

impl<'a> Frame<'a> {
    /// A frame that has not yet requested a redraw.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "constructs every per-frame render target parameter in one const constructor"
    )]
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
            redraw_rate: None,
            source_rate_update: SourceRateUpdate::Unchanged,
        }
    }

    /// Engine-owned device.
    #[must_use]
    pub const fn device(&self) -> &'a Device {
        self.device
    }

    /// Engine-owned queue.
    #[must_use]
    pub const fn queue(&self) -> &'a Queue {
        self.queue
    }

    /// Offscreen render target for this frame.
    #[must_use]
    pub const fn texture(&self) -> &'a Texture {
        self.texture
    }

    /// Color attachment view of [`Self::texture`].
    #[must_use]
    pub const fn view(&self) -> &'a TextureView {
        self.view
    }

    /// Format of [`Self::texture`].
    #[must_use]
    pub const fn format(&self) -> TextureFormat {
        self.format
    }

    /// Width of [`Self::texture`] in physical pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height of [`Self::texture`] in physical pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Physical pixel dimensions `(width, height)` of [`Self::texture`].
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Display scale factor.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// Elapsed time since the producer's first rendered frame.
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Time delta since the previous frame.
    #[must_use]
    pub const fn delta(&self) -> Duration {
        self.delta
    }

    /// The platform's target presentation time for this frame, when available.
    #[must_use]
    pub const fn presentation_time(&self) -> Option<Instant> {
        self.presentation_time
    }

    /// Asks for another frame after this one at [`FrameRate::FAST`].
    pub const fn request_redraw(&mut self) {
        self.request_redraw_at(FrameRate::FAST);
    }

    /// Asks for another frame after this one at `rate`, combining repeated
    /// requests within one pass by field-wise max.
    pub const fn request_redraw_at(&mut self, rate: FrameRate) {
        self.redraw_rate = Some(match self.redraw_rate {
            Some(current) => current.combine_with(rate),
            None => rate,
        });
    }

    /// Declares or clears this producer's non-redrawing source cadence
    /// (latched across subsequent frames until replaced or cleared with
    /// `None`). Unlike [`Self::request_redraw_at`], this never requests
    /// another frame on its own.
    pub const fn set_source_rate(&mut self, rate: Option<FrameRate>) {
        self.source_rate_update = match rate {
            Some(rate) => SourceRateUpdate::Set(rate),
            None => SourceRateUpdate::Clear,
        };
    }

    /// Whether the content asked for another frame.
    #[must_use]
    pub const fn redraw_requested(&self) -> bool {
        self.redraw_rate.is_some()
    }

    /// The refresh-rate vote requested for the next frame, if any.
    #[must_use]
    pub const fn redraw_rate(&self) -> Option<FrameRate> {
        self.redraw_rate
    }

    /// The source-cadence update recorded in this frame.
    #[must_use]
    pub const fn source_rate_update(&self) -> SourceRateUpdate {
        self.source_rate_update
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

/// A UI-side handler for input the backend routes to a [`GpuView`](crate::GpuView)
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
/// routing, the per-frame pump, reactive parameter watcher guards, and the
/// caret query, shared by [`GpuView`](crate::GpuView) and [`ExternalFrameView`].
#[derive(Default)]
struct SurfaceHooks {
    input: Option<InputHandler>,
    frame: Option<FrameHook>,
    caret: Option<CaretQuery>,
    guards: Vec<BoxWatcherGuard>,
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

/// Shared access to a [`GpuContent`](crate::GpuContent) after the engine took it.
///
/// The engine drives the content through this handle; the view keeps a clone
/// so UI-side queries — layout measurement, the HDR preference — still reach
/// the live content, and so a rebuilt engine can re-install the same object
/// after device loss.
pub struct GpuContentHandle(Arc<Mutex<Box<dyn ErasedGpuContent>>>);

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
    fn new(content: Box<dyn ErasedGpuContent>) -> Self {
        Self(Arc::new(Mutex::new(content)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn ErasedGpuContent>> {
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
        if let Some(rate) = frame.redraw_rate() {
            gpu.request_redraw_at(rate);
        }
        match frame.source_rate_update() {
            SourceRateUpdate::Unchanged => {}
            SourceRateUpdate::Clear => gpu.set_source_rate(None),
            SourceRateUpdate::Set(rate) => gpu.set_source_rate(Some(rate)),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::{NativeView, Str};

    use super::leaf::GpuContentLeaf;
    use super::{
        Arc, Context, Display, Frame, FrameTime, GpuContentRenderer, GpuRuntime, Mutex,
        ProposalSize, Rc, RedrawHandle, Size, Vec,
    };
    use crate::{GpuContent, GpuView};

    /// Content that reports an intrinsic size.
    struct Sized;

    impl GpuContent for Sized {
        type Params = ();

        fn setup(&mut self, _gpu: &Context<'_>) {}

        fn render(&mut self, _frame: &mut Frame<'_>) {}

        fn intrinsic_size(&self) -> Option<Size> {
            Some(Size::new(4.0, 4.0))
        }
    }

    #[test]
    fn engine_content_returns_shared_handle_across_reinstalls() {
        let mut leaf = GpuContentLeaf::from(GpuView::new(Sized));
        let first = leaf.engine_content();
        let second = leaf.engine_content();
        assert_eq!(
            first.measure(ProposalSize::UNSPECIFIED).size,
            Size::new(4.0, 4.0)
        );
        assert_eq!(
            second.measure(ProposalSize::UNSPECIFIED).size,
            Size::new(4.0, 4.0)
        );
    }

    #[test]
    fn gpu_view_lowers_to_leaf_preserving_metadata_and_measurement() {
        let leaf = GpuContentLeaf::from(
            GpuView::new(Sized)
                .offered_label(Str::from("Chart"))
                .prefer_hdr_surface(),
        );
        assert_eq!(leaf.accessibility_label().as_deref(), Some("Chart"));
        assert_eq!(leaf.resolved_hdr_preference(), Some(true));
        assert_eq!(
            leaf.measure(ProposalSize::UNSPECIFIED).size,
            Size::new(4.0, 4.0)
        );
        assert_eq!(NativeView::stretch_axis(&leaf), StretchAxis::None);
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ParamEvent {
        Setup,
        Update(u32),
        Render(u32),
    }

    struct RecordingContent {
        current: u32,
        events: Arc<Mutex<Vec<ParamEvent>>>,
    }

    impl GpuContent for RecordingContent {
        type Params = u32;
        fn setup(&mut self, _gpu: &Context<'_>) {
            self.events
                .lock()
                .expect("event log lock poisoned")
                .push(ParamEvent::Setup);
        }

        fn update(&mut self, params: u32) {
            self.current = params;
            self.events
                .lock()
                .expect("event log lock poisoned")
                .push(ParamEvent::Update(params));
        }

        fn render(&mut self, _frame: &mut Frame<'_>) {
            self.events
                .lock()
                .expect("event log lock poisoned")
                .push(ParamEvent::Render(self.current));
        }

        fn is_opaque(&self) -> bool {
            true
        }

        fn intrinsic_size(&self) -> Option<Size> {
            Some(Size::new(16.0, 16.0))
        }

        fn preferred_surface_hdr(&self) -> Option<bool> {
            Some(true)
        }
    }

    #[test]
    fn parameterized_view_preserves_metadata_and_chains_on_frame_in_both_orders() {
        let param = Binding::new(7_u32);
        let before_count = Rc::new(Cell::new(0_usize));
        let before_hook = Rc::clone(&before_count);
        let leaf_before = GpuContentLeaf::from(
            GpuView::new(RecordingContent {
                current: 0,
                events: Arc::new(Mutex::new(Vec::new())),
            })
            .offered_label(Str::from("Before"))
            .offered_value(Str::from("Val"))
            .on_frame(move || before_hook.set(before_hook.get() + 1))
            .params(param.clone()),
        );
        assert!(leaf_before.is_opaque());
        assert_eq!(leaf_before.intrinsic_size(), Some(Size::new(16.0, 16.0)));
        assert_eq!(leaf_before.resolved_hdr_preference(), Some(true));
        assert_eq!(leaf_before.accessibility_label().as_deref(), Some("Before"));
        assert_eq!(leaf_before.accessibility_value().as_deref(), Some("Val"));
        leaf_before.frame();
        assert_eq!(before_count.get(), 1);

        let after_count = Rc::new(Cell::new(0_usize));
        let after_hook = Rc::clone(&after_count);
        let leaf_after = GpuContentLeaf::from(
            GpuView::new(RecordingContent {
                current: 0,
                events: Arc::new(Mutex::new(Vec::new())),
            })
            .params(param)
            .on_frame(move || after_hook.set(after_hook.get() + 1)),
        );
        leaf_after.frame();
        assert_eq!(after_count.get(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parameterized_view_delivers_initial_and_updated_params_and_wakes_idle_host() {
        use cherenkov::Next;
        use cherenkov_gpu::interop::GpuContentBox;

        use crate::OffscreenSize;

        let runtime =
            pollster::block_on(GpuRuntime::new()).expect("GPU runtime opens on test hardware");
        let param = Binding::new(10_u32);
        let events = Arc::new(Mutex::new(Vec::new()));
        let hook_runs = Rc::new(Cell::new(0_usize));
        let hook_runs_clone = Rc::clone(&hook_runs);

        let mut leaf = GpuContentLeaf::from(
            GpuView::new(RecordingContent {
                current: 0,
                events: Arc::clone(&events),
            })
            .params(param.clone())
            .on_frame(move || hook_runs_clone.set(hook_runs_clone.get() + 1)),
        );

        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_clone = Arc::clone(&wakes);
        let size = OffscreenSize::try_from_pixels(16, 16).expect("non-zero test size");
        let display = Display {
            scale: 1.0,
            headroom: 1.0,
        };
        let mut renderer = pollster::block_on(GpuContentRenderer::new(
            &runtime,
            runtime.context(),
            leaf.take_engine_content(),
            size,
            RedrawHandle::new(move || {
                wakes_clone.fetch_add(1, Ordering::SeqCst);
            }),
        ))
        .expect("GPU content renderer mounts");

        let take_events = || core::mem::take(&mut *events.lock().expect("event log lock poisoned"));

        // First frame delivers initial params after setup and before render, then idles.
        leaf.frame();
        let frame1 = pollster::block_on(renderer.render(size, display, FrameTime::now()))
            .expect("first hosted frame renders");
        assert!(matches!(frame1, Next::Idle), "idle content stays idle");
        assert_eq!(hook_runs.get(), 1);
        assert_eq!(
            take_events(),
            vec![
                ParamEvent::Setup,
                ParamEvent::Update(10),
                ParamEvent::Render(10),
            ]
        );

        // Multiple updates before the next frame wake the idle host and coalesce to the latest value.
        let wakes_before = wakes.load(Ordering::SeqCst);
        param.set(20);
        param.set(30);
        assert!(
            wakes.load(Ordering::SeqCst) > wakes_before,
            "signal change must wake an idle GPU host"
        );

        leaf.frame();
        let frame2 = pollster::block_on(renderer.render(size, display, FrameTime::now()))
            .expect("second hosted frame renders");
        assert!(matches!(frame2, Next::Idle));
        assert_eq!(hook_runs.get(), 2);
        assert_eq!(
            take_events(),
            vec![ParamEvent::Update(30), ParamEvent::Render(30)]
        );

        // While quiet, the idle producer does not re-render; a size change renders without re-running `update`.
        leaf.frame();
        let _ = pollster::block_on(renderer.render(size, display, FrameTime::now()))
            .expect("quiet hosted frame renders");
        assert!(
            take_events().is_empty(),
            "idle producer skips render while quiet"
        );
        let resized = OffscreenSize::try_from_pixels(24, 24).expect("non-zero resized test size");
        let _ = pollster::block_on(renderer.render(resized, display, FrameTime::now()))
            .expect("resized hosted frame renders");
        assert_eq!(take_events(), vec![ParamEvent::Render(30)]);

        // Re-installing the same handle (simulating device-loss re-setup) re-delivers the latest params.
        let mut rebuilt = pollster::block_on(GpuContentRenderer::new(
            &runtime,
            runtime.context(),
            GpuContentBox::new(leaf.engine_content()),
            size,
            RedrawHandle::new(|| {}),
        ))
        .expect("rebuilt GPU content renderer mounts");
        let _ = pollster::block_on(rebuilt.render(size, display, FrameTime::now()))
            .expect("rebuilt hosted frame renders");
        assert_eq!(
            take_events(),
            vec![
                ParamEvent::Setup,
                ParamEvent::Update(30),
                ParamEvent::Render(30),
            ]
        );
    }
}
