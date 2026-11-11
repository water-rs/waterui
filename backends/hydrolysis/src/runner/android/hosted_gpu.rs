//! GPU content on the HWUI painter (water-rs/waterui#1812).
//!
//! HWUI draws the retained tree into `RenderNode`s and has no engine layer
//! tree a GPU producer could join. Each `GpuContentView` and
//! `ExternalFrameView` therefore gets a surface of its own: a Kotlin
//! `TextureView` mounted as a platform view at the node's frame. The
//! painter places it exactly like any other platform view — in session
//! z-order, inside its owner's clip and transform — so GPU content
//! overlaps, clips and stacks with ordinary views.
//!
//! The pieces:
//!
//! - [`HostedGpu`], the window's owner of the shared [`GpuRuntime`] (created
//!   on its own thread when the first view needs it, so an app without GPU
//!   content never opens a device) and of the surface slots the host's
//!   JNI callbacks address by placement id.
//! - [`SurfaceSlot`], what the host reports about one `TextureView`'s
//!   `Surface`: the native window, its size, and the wgpu surface the pump
//!   created over it. The host's detach drops the wgpu surface at once — the
//!   `Surface` must be released before the platform reclaims it.
//! - [`HostedLayer`], one view's pump. Each commit it submits an engine
//!   frame to the hosted renderer (`GpuContentRenderer` or
//!   `ExternalFrameRenderer`), polls it, and composites a resolved frame
//!   into the surface's next texture.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};

use ndk::native_window::NativeWindow;
use rustc_hash::FxHashMap;
use waterui::Environment;
use waterui_graphics::OffscreenSize;
use waterui_graphics::gpu::interop::GpuContentBox;
use waterui_graphics::gpu::{
    Display, ExternalFrameRenderer, ExternalFrameView, FrameTime, GpuContentRenderer,
    GpuContentView, GpuRuntime, GpuRuntimeError, HostedFrame, HostedLayerError, RedrawHandle,
    RenderedFrame, SharedGpuContext, preferred_surface_format, wgpu,
};

use super::native_window::AndroidWindow;
use crate::platform_view::{
    PlatformViewPlacement, PlatformViewSink, PlatformViewSource, PlatformViewTable,
};
use crate::renderer::ProducerWake;

/// The platform-view kind the Kotlin HWUI painter registers a `TextureView`
/// factory under. Application kinds are free vocabulary; this one belongs to
/// the backend.
const HOSTED_SURFACE_KIND: &str = "dev.waterui.hydrolysis.hosted-gpu-surface";

/// The wake of a startup that finished: views waiting for the runtime
/// register here, and the runtime's thread drains them once.
enum StartupWakes {
    Waiting(Vec<ProducerWake>),
    Done,
}

enum RuntimeState {
    /// No view has asked for a device yet.
    Idle,
    /// The runtime is being created on its thread.
    Starting {
        result: Receiver<Result<GpuRuntime, GpuRuntimeError>>,
        wakes: Arc<Mutex<StartupWakes>>,
    },
    Ready(GpuRuntime),
}

/// The window's GPU-content host: the shared device runtime and the surface
/// slots of its views.
pub struct HostedGpu {
    sink: PlatformViewSink,
    /// The display density surfaces present at; the window reports it
    /// every frame ahead of the commit.
    scale: Cell<f64>,
    runtime: RefCell<RuntimeState>,
    slots: RefCell<FxHashMap<u64, Weak<RefCell<SurfaceSlot>>>>,
}

impl HostedGpu {
    /// A host minting placement ids from `sink`, the table the window's
    /// platform views publish through.
    pub(crate) fn new(sink: PlatformViewSink) -> Self {
        Self {
            sink,
            scale: Cell::new(1.0),
            runtime: RefCell::new(RuntimeState::Idle),
            slots: RefCell::new(FxHashMap::default()),
        }
    }

    /// Reports the display density the next commit presents at.
    pub(crate) fn set_scale(&self, scale: f64) {
        self.scale.set(scale);
    }

    /// The runtime when it is ready. The first call starts creating it on
    /// its own thread — adapter and device requests do not run on the UI
    /// thread — and registers `wake` to fire when it lands.
    ///
    /// # Panics
    ///
    /// When the device cannot be created: a device without the required
    /// Vulkan support is an unsupported device, never a degraded one.
    fn runtime(&self, wake: &ProducerWake) -> Option<GpuRuntime> {
        let mut state = self.runtime.borrow_mut();
        loop {
            match &*state {
                RuntimeState::Ready(runtime) => return Some(runtime.clone()),
                RuntimeState::Idle => {
                    let (sender, result) = channel();
                    let wakes = Arc::new(Mutex::new(StartupWakes::Waiting(vec![wake.clone()])));
                    let thread_wakes = Arc::clone(&wakes);
                    std::thread::Builder::new()
                        .name("hydrolysis-gpu-runtime".to_owned())
                        .spawn(move || {
                            let created = pollster::block_on(GpuRuntime::new());
                            // The receiver is dropped only with the window.
                            let _ = sender.send(created);
                            let done = std::mem::replace(
                                &mut *thread_wakes.lock().unwrap_or_else(PoisonError::into_inner),
                                StartupWakes::Done,
                            );
                            if let StartupWakes::Waiting(wakes) = done {
                                for wake in wakes {
                                    wake.request_redraw();
                                }
                            }
                        })
                        .expect("hydrolysis HWUI: failed to start the GPU runtime thread");
                    *state = RuntimeState::Starting { result, wakes };
                    return None;
                }
                RuntimeState::Starting { result, wakes } => match result.try_recv() {
                    Ok(Ok(runtime)) => *state = RuntimeState::Ready(runtime),
                    Ok(Err(error)) => {
                        panic!("hydrolysis HWUI: GPU content needs a GPU device: {error}")
                    }
                    Err(TryRecvError::Empty) => {
                        let mut wakes = wakes.lock().unwrap_or_else(PoisonError::into_inner);
                        match &mut *wakes {
                            StartupWakes::Waiting(waiting) => waiting.push(wake.clone()),
                            // The thread finished between the receive above
                            // and this lock: the result is waiting now.
                            StartupWakes::Done => continue,
                        }
                        drop(wakes);
                        return None;
                    }
                    Err(TryRecvError::Disconnected) => {
                        panic!("hydrolysis HWUI: the GPU runtime thread ended without a result")
                    }
                },
            }
        }
    }

    /// The host reports the `Surface` of placement `id`'s `TextureView`. A
    /// view that retired first has no slot any more: the report is the host
    /// catching up, and the window lease drops with it.
    pub(crate) fn attach(&self, id: u64, window: NativeWindow, width: u32, height: u32) {
        if let Some(slot) = self.slot(id) {
            slot.borrow_mut().attach(window, width, height);
        }
    }

    /// The host resized placement `id`'s `Surface`.
    pub(crate) fn resize(&self, id: u64, width: u32, height: u32) {
        if let Some(slot) = self.slot(id) {
            slot.borrow_mut().resize(width, height);
        }
    }

    /// The host is releasing placement `id`'s `Surface`.
    pub(crate) fn detach(&self, id: u64) {
        if let Some(slot) = self.slot(id) {
            slot.borrow_mut().detach();
        }
    }

    fn slot(&self, id: u64) -> Option<Rc<RefCell<SurfaceSlot>>> {
        self.slots.borrow().get(&id).and_then(Weak::upgrade)
    }
}

/// What the host holds for one view's `TextureView`.
struct SurfaceSlot {
    window: Option<AttachedWindow>,
    target: Option<Target>,
    /// The owning node's wake, so a host callback marks it for a frame.
    wake: Option<ProducerWake>,
}

struct AttachedWindow {
    window: NativeWindow,
    width: u32,
    height: u32,
}

/// The wgpu surface over the attached window, configured for `size` on the
/// device generation it was created under.
struct Target {
    surface: wgpu::Surface<'static>,
    size: (u32, u32),
    generation: u64,
}

impl SurfaceSlot {
    fn attach(&mut self, window: NativeWindow, width: u32, height: u32) {
        // A new `Surface` replaces whatever the old one backed.
        self.target = None;
        self.window = Some(AttachedWindow {
            window,
            width,
            height,
        });
        self.request_frame();
    }

    fn resize(&mut self, width: u32, height: u32) {
        if let Some(attached) = &mut self.window {
            attached.width = width;
            attached.height = height;
            self.request_frame();
        }
    }

    fn detach(&mut self) {
        // The swapchain holds the producer end of the window's queue: it is
        // released before the platform reclaims the `Surface`.
        self.target = None;
        self.window = None;
    }

    fn request_frame(&self) {
        if let Some(wake) = &self.wake {
            wake.request_redraw();
        }
    }
}

/// A frame's presentation target: configured for `size` and `generation`,
/// recreated when the window or the device changed under it.
impl SurfaceSlot {
    fn target(&mut self, context: &SharedGpuContext) -> Option<&Target> {
        let attached = self.window.as_ref()?;
        let size = (attached.width.max(1), attached.height.max(1));
        let stale = self
            .target
            .as_ref()
            .is_some_and(|target| target.generation != context.generation());
        if stale {
            self.target = None;
        }
        if self.target.is_none() {
            let surface = context
                .instance()
                .create_surface(AndroidWindow(attached.window.clone()))
                .unwrap_or_else(|error| {
                    panic!("hydrolysis HWUI: GPU content surface creation failed: {error}")
                });
            self.target = Some(Target {
                surface,
                size: (0, 0),
                generation: context.generation(),
            });
        }
        let target = self.target.as_mut()?;
        if target.size != size {
            let capabilities = target.surface.get_capabilities(context.adapter());
            let mut configuration = target
                .surface
                .get_default_config(context.adapter(), size.0, size.1)
                .expect("hydrolysis HWUI: the adapter cannot present to the GPU content surface");
            configuration.format = preferred_surface_format(&capabilities, false);
            configuration.alpha_mode = [
                wgpu::CompositeAlphaMode::PreMultiplied,
                wgpu::CompositeAlphaMode::Inherit,
            ]
            .into_iter()
            .find(|mode| capabilities.alpha_modes.contains(mode))
            .unwrap_or(configuration.alpha_mode);
            target.surface.configure(context.device(), &configuration);
            target.size = size;
        }
        self.target.as_ref()
    }
}

/// The engine layers a hosted view renders through:
/// [`GpuContentRenderer`] and [`ExternalFrameRenderer`].
pub trait LayerRenderer {
    fn generation(&self) -> u64;

    fn submit(
        &mut self,
        size: OffscreenSize,
        display: Display,
        target_time: FrameTime,
    ) -> Result<HostedFrame, HostedLayerError>;

    fn composite(&mut self, frame: &RenderedFrame, target: &wgpu::Texture, display: Display);
}

macro_rules! layer_renderer {
    ($renderer:ty) => {
        impl LayerRenderer for $renderer {
            fn generation(&self) -> u64 {
                Self::generation(self)
            }

            fn submit(
                &mut self,
                size: OffscreenSize,
                display: Display,
                target_time: FrameTime,
            ) -> Result<HostedFrame, HostedLayerError> {
                Self::submit(self, size, display, target_time)
            }

            fn composite(
                &mut self,
                frame: &RenderedFrame,
                target: &wgpu::Texture,
                display: Display,
            ) {
                Self::composite(self, frame, target, display);
            }
        }
    };
}

layer_renderer!(GpuContentRenderer);
layer_renderer!(ExternalFrameRenderer);

type RendererCreation =
    Pin<Box<dyn Future<Output = Result<Box<dyn LayerRenderer>, HostedLayerError>>>>;

/// A view the pump renders: how its hosted renderer is built, and the UI
/// hook that runs ahead of every frame.
pub trait HostedSource {
    /// Builds the hosted renderer on `context`, waking `redraw` whenever
    /// the content wants a frame of its own.
    fn renderer(
        &mut self,
        runtime: GpuRuntime,
        context: Arc<SharedGpuContext>,
        size: OffscreenSize,
        redraw: RedrawHandle,
    ) -> RendererCreation;

    /// The view's per-frame UI hook.
    fn before_frame(&self);
}

impl HostedSource for GpuContentView {
    fn renderer(
        &mut self,
        runtime: GpuRuntime,
        context: Arc<SharedGpuContext>,
        size: OffscreenSize,
        redraw: RedrawHandle,
    ) -> RendererCreation {
        // The same content object answers every call, so a renderer rebuilt
        // after device loss installs it again with its accumulated state.
        let producer = GpuContentBox::new(self.engine_content());
        Box::pin(async move {
            let layer = GpuContentRenderer::new(&runtime, context, producer, size, redraw).await?;
            Ok(Box::new(layer) as Box<dyn LayerRenderer>)
        })
    }

    fn before_frame(&self) {
        self.frame();
    }
}

impl HostedSource for ExternalFrameView {
    fn renderer(
        &mut self,
        runtime: GpuRuntime,
        context: Arc<SharedGpuContext>,
        size: OffscreenSize,
        redraw: RedrawHandle,
    ) -> RendererCreation {
        let stream = self.stream();
        Box::pin(async move {
            let layer =
                ExternalFrameRenderer::new(&runtime, context, &stream, size, redraw).await?;
            Ok(Box::new(layer) as Box<dyn LayerRenderer>)
        })
    }

    fn before_frame(&self) {
        self.frame();
    }
}

/// Wakes the owning node when the render thread resolves a frame or a
/// renderer; the next commit polls again.
struct FrameWaker(ProducerWake);

impl Wake for FrameWaker {
    fn wake(self: Arc<Self>) {
        self.0.request_redraw();
    }
}

enum Renderer {
    None,
    Creating(RendererCreation),
    Ready(Box<dyn LayerRenderer>),
}

/// One GPU view's presentation: its placement, surface slot and the engine
/// renderer and frame it has in flight.
pub struct HostedLayer {
    id: u64,
    table: Rc<RefCell<PlatformViewTable>>,
    gpu: Rc<HostedGpu>,
    slot: Rc<RefCell<SurfaceSlot>>,
    renderer: Renderer,
    frame: Option<Pin<Box<HostedFrame>>>,
    /// The pixel size the frame in flight was submitted at.
    submitted: (u32, u32),
    /// Set by the content's own redraw requests, and by a frame that must
    /// render again: a frame is owed.
    owed: Arc<AtomicBool>,
    /// The engine asked for another frame when it resolved the last one.
    engine_wants_frame: bool,
}

impl HostedLayer {
    /// A layer in the window environment's [`HostedGpu`].
    ///
    /// # Panics
    ///
    /// When the environment carries none: the Android runner installs it at
    /// session create, so a missing host is a runner bug.
    pub(crate) fn in_environment(env: &Environment) -> Self {
        let gpu = env.get::<Rc<HostedGpu>>().unwrap_or_else(|| {
            panic!(
                "hydrolysis HWUI: GPU content needs the window's hosted-GPU owner, which the \
                 Android runner installs at session create"
            )
        });
        Self::new(gpu)
    }

    /// A layer with a fresh placement id and surface slot in `gpu`.
    fn new(gpu: &Rc<HostedGpu>) -> Self {
        let id = gpu.sink.next_id();
        let slot = Rc::new(RefCell::new(SurfaceSlot {
            window: None,
            target: None,
            wake: None,
        }));
        gpu.slots.borrow_mut().insert(id, Rc::downgrade(&slot));
        Self {
            id,
            table: Rc::clone(gpu.sink.table()),
            gpu: Rc::clone(gpu),
            slot,
            renderer: Renderer::None,
            frame: None,
            submitted: (0, 0),
            owed: Arc::new(AtomicBool::new(true)),
            engine_wants_frame: false,
        }
    }

    /// The platform-view sink table the placement publishes through.
    pub(crate) const fn table(&self) -> &Rc<RefCell<PlatformViewTable>> {
        &self.table
    }

    /// The placement the host mounts a `TextureView` for.
    pub(crate) fn placement(&self) -> PlatformViewPlacement {
        PlatformViewPlacement::new(
            self.id,
            PlatformViewSource::Factory {
                kind: HOSTED_SURFACE_KIND.into(),
            },
        )
    }

    /// One commit's step: build the renderer once its device and surface
    /// exist, resolve the frame in flight into the surface, and submit the
    /// next when one is owed.
    ///
    pub(crate) fn pump(&mut self, source: &mut impl HostedSource, wake: &ProducerWake) {
        source.before_frame();
        let slot = Rc::clone(&self.slot);
        let mut slot = slot.borrow_mut();
        if slot.wake.is_none() {
            slot.wake = Some(wake.clone());
        }
        let Some(runtime) = self.gpu.runtime(wake) else {
            return;
        };
        let context = runtime.context();
        // A rebuilt device invalidates the renderer and its frame; the
        // surface follows through its own generation check.
        if matches!(&self.renderer, Renderer::Ready(renderer) if renderer.generation() != context.generation())
        {
            self.renderer = Renderer::None;
            self.frame = None;
        }
        let Some(size) = slot
            .window
            .as_ref()
            .and_then(|attached| OffscreenSize::try_from_pixels(attached.width, attached.height))
        else {
            // No `Surface`: nothing to draw into, nothing in flight.
            self.frame = None;
            return;
        };
        let display = Display {
            scale: self.gpu.scale.get(),
            headroom: 1.0,
        };
        let waker = Waker::from(Arc::new(FrameWaker(wake.clone())));
        let mut cx = Context::from_waker(&waker);

        if matches!(self.renderer, Renderer::None) {
            let owed = Arc::clone(&self.owed);
            let content_wake = wake.clone();
            let redraw = RedrawHandle::new(move || {
                owed.store(true, Ordering::Release);
                content_wake.request_redraw();
            });
            self.renderer =
                Renderer::Creating(source.renderer(runtime, Arc::clone(&context), size, redraw));
        }
        if let Renderer::Creating(creation) = &mut self.renderer {
            match creation.as_mut().poll(&mut cx) {
                Poll::Pending => return,
                Poll::Ready(Ok(renderer)) => self.renderer = Renderer::Ready(renderer),
                Poll::Ready(Err(error)) => {
                    self.renderer = Renderer::None;
                    return fail(&error);
                }
            }
        }
        let pixels = (size.width(), size.height());
        if self.frame.is_some() && !self.resolve(&mut cx, &mut slot, &context, display) {
            return;
        }
        let owed =
            self.owed.swap(false, Ordering::AcqRel) || std::mem::take(&mut self.engine_wants_frame);
        if owed {
            let Renderer::Ready(renderer) = &mut self.renderer else {
                unreachable!("the renderer was made ready above");
            };
            match renderer.submit(size, display, FrameTime::now()) {
                Ok(frame) => {
                    self.submitted = pixels;
                    self.frame = Some(Box::pin(frame));
                    // Polling registers the waker the render thread fires.
                    self.resolve(&mut cx, &mut slot, &context, display);
                }
                Err(error) => fail(&error),
            }
        }
        // The engine wants frames at its own pace (a playing video): the
        // host's vsync pump is that pace.
        if self.engine_wants_frame {
            wake.request_redraw();
        }
    }

    /// Polls the frame in flight; when it resolved, composites it into the
    /// surface's next texture and presents. Returns whether the frame
    /// resolved (successfully or not), leaving none in flight.
    fn resolve(
        &mut self,
        cx: &mut Context<'_>,
        slot: &mut SurfaceSlot,
        context: &SharedGpuContext,
        display: Display,
    ) -> bool {
        let frame = self
            .frame
            .as_mut()
            .expect("a frame is in flight when it is resolved");
        let rendered = match frame.as_mut().poll(cx) {
            Poll::Pending => return false,
            Poll::Ready(Ok(rendered)) => rendered,
            Poll::Ready(Err(error)) => {
                self.frame = None;
                fail(&error);
                return true;
            }
        };
        self.frame = None;
        self.engine_wants_frame = !matches!(rendered.next(), cherenkov::Next::Idle);
        let Renderer::Ready(engine) = &mut self.renderer else {
            return true;
        };
        let attached = slot
            .window
            .as_ref()
            .map(|window| (window.width, window.height));
        // A frame for another size than the surface now has is stale: the
        // next one renders at the new size.
        if attached != Some(self.submitted) {
            self.owed.store(true, Ordering::Release);
            return true;
        }
        let Some(target) = slot.target(context) else {
            return true;
        };
        match target.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                engine.composite(&rendered, &output.texture, display);
                context.queue().present(output);
            }
            // The swapchain changed under the frame: it is rebuilt and the
            // frame renders again.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                slot.target = None;
                self.owed.store(true, Ordering::Release);
            }
            // Not presentable right now: the frame is owed again.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.owed.store(true, Ordering::Release);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                panic!("hydrolysis HWUI: the GPU content surface failed validation on acquire")
            }
        }
        true
    }
}

impl Drop for HostedLayer {
    fn drop(&mut self) {
        self.gpu.slots.borrow_mut().remove(&self.id);
    }
}

/// A hosted renderer or frame failed: a contract violation panics in a
/// debug build and is logged in a release build, where the view stops
/// producing frames.
fn fail(error: &HostedLayerError) {
    debug_assert!(false, "hydrolysis HWUI: GPU content failed: {error}");
    tracing::error!(%error, "hydrolysis HWUI: GPU content failed");
}
