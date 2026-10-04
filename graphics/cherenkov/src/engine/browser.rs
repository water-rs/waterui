//! The engine: owns the render thread and every resource's identity.
//! `!Send`, lives on the UI thread.

use super::{SurfaceWaker, Waker, thread};

use crate::local::Sender;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::ShaderId;
use crate::backend::{Backend, Renderer};
use crate::capability::{
    DrainedProducer, Effects, Filters, GpuContent, Runs, ShaderPaint, ShaderSource, Uploads,
};
use crate::config::{MemoryUsage, Pressure};
use crate::error::{EngineError, RenderError, ResourceError, SurfaceError};
use crate::frame::{FrameStats, FrameTime, FrameTiming, Next};
use crate::glyph::FontId;
use crate::image::{Format, ImageData};
use crate::message::{
    ChangeSet, FontData, Message, ProducerId, RegisterOp, RenderReply, SurfaceId,
};
use crate::paint::ImageId;
use crate::resource::{
    Filter, Font, FontSource, FrameSink, GpuProducer, Image, ReplaceImage, ResourceId, Shader,
};
use crate::style::FilterId;
use crate::surface::{Shared, Surface};

/// Owns the device and a serial executor on the creating JS thread.
///
/// `Engine`, its resources and futures are deliberately `!Send`. Operations
/// awaiting WebGPU yield without moving browser objects to another thread.
/// Native builds expose the synchronous render-thread API instead.
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<cherenkov::Engine<cherenkov::testing::Null>>();
/// ```
pub struct Engine<B: Backend> {
    tx: Sender<Message<B>>,
    info: B::Info,
    stats: RefCell<FrameStats>,
    commits: RefCell<Vec<(SurfaceId, ChangeSet<B>)>>,
    /// The live surfaces' shared queues, drained into one `Render` message
    /// per frame.
    surfaces: RefCell<Vec<std::rc::Weak<RefCell<Shared<B>>>>>,
    next_surface: Cell<u64>,
    next_font: Cell<u64>,
    next_image: Cell<u64>,
    next_shader: Cell<u64>,
    next_filter: Cell<u64>,
    next_backdrop_shader: Cell<u64>,
    next_producer: Cell<u64>,
    /// The type-erased sender resource drops use; an executor that is
    /// gone has nothing left to release.
    post: Rc<dyn Fn(Message<B>)>,
    /// The `Message::ReplaceImage` sender every image handle shares.
    replace_image: ReplaceImage,
    waker: Arc<Waker>,
    // `!Send`: the engine lives on the UI thread.
    _not_send: PhantomData<Rc<()>>,
}

impl<B: Backend> std::fmt::Debug for Engine<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

/// The ownership of a surface creation while its request is in flight
/// (#150).
///
/// Constructed before the request is enqueued, so dropping the
/// [`Engine::surface`] future — before the backend processed it, after it
/// committed, or after the reply — still destroys what was committed.
/// `disarm` hands the surface to its handle, or is the rejected path,
/// where nothing exists to destroy.
struct Registration {
    release: Option<Box<dyn FnOnce()>>,
}

impl Registration {
    /// The surface handle owns the destruction now, or nothing was
    /// committed: no destruction is enqueued.
    fn disarm(&mut self) {
        self.release = None;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl<B: Backend> Engine<B> {
    /// Initializes the backend and its serial executor on the current JS thread.
    /// Await this future from the browser event loop; do not block on it.
    ///
    /// # Errors
    /// [`EngineError`] when the backend fails to initialize or the render
    /// thread cannot start.
    #[expect(
        clippy::arc_with_non_send_sync,
        reason = "everything here is single-threaded; `Arc` matches the \
            shared surface/record field types, and the callback never \
            leaves this thread"
    )]
    pub async fn new(config: B::Config) -> Result<Self, EngineError> {
        let (tx, info) = thread::local::<B>(config).await?;
        let post_tx = tx.clone();
        let waker = Arc::new(Waker::new());
        let replace_image = {
            let tx = tx.clone();
            // The executor wakes the host through every visible surface
            // that draws the image, once it knows which surfaces do.
            Rc::new(move |id, image| {
                tx.send(Message::ReplaceImage { id, image })
                    .map_err(|_| ResourceError::Lost)
            }) as ReplaceImage
        };
        Ok(Self {
            tx,
            info,
            stats: RefCell::new(FrameStats::default()),
            commits: RefCell::new(Vec::new()),
            surfaces: RefCell::new(Vec::new()),
            next_surface: Cell::new(0),
            next_font: Cell::new(1),
            next_image: Cell::new(1),
            next_shader: Cell::new(1),
            next_filter: Cell::new(1),
            next_backdrop_shader: Cell::new(1),
            next_producer: Cell::new(1),
            post: Rc::new(move |message| {
                let _ = post_tx.send(message);
            }),
            replace_image,
            waker,
            _not_send: PhantomData,
        })
    }

    /// The backend's provenance (`B::Info`).
    #[must_use]
    pub const fn info(&self) -> &B::Info {
        &self.info
    }

    /// Statistics of the last [`Engine::render`]. GPU timings are kept by
    /// the render thread and returned by [`Engine::finish_timings`] instead.
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.stats.borrow().clone()
    }

    /// Returns all GPU timings accumulated since the previous call,
    /// oldest first. Awaits frames still in flight without blocking
    /// JavaScript, so this marks the end of a measured window and belongs
    /// to tooling, never to a frame path. Empty when the backend reports
    /// no GPU timing or has nothing outstanding.
    ///
    /// # Errors
    /// [`RenderError::Timeout`] when the GPU does not finish in time,
    /// [`RenderError::Readback`] when a timing buffer cannot be read, and
    /// [`RenderError::Thread`] when the render thread is gone.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn finish_timings(&self) -> Result<Vec<FrameTiming>, RenderError> {
        let (reply, rx) = crate::local::channel();
        self.tx
            .send(Message::FinishTimings { reply })
            .map_err(|_| RenderError::Thread)?;
        rx.recv().await.map_err(|_| RenderError::Thread)?
    }

    /// The engine's current memory usage.
    ///
    /// # Panics
    /// Panics if the reply channel drops without answering, which cannot
    /// happen while the render thread is alive.
    #[must_use]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn memory(&self) -> MemoryUsage {
        let (reply, rx) = crate::local::channel();
        if self.tx.send(Message::Memory { reply }).is_err() {
            return MemoryUsage::default();
        }
        rx.recv()
            .await
            .map_or_else(|_| MemoryUsage::default(), |reply| reply.usage)
    }

    /// Reports system memory pressure. `Critical` drops every cache.
    pub fn trim(&self, pressure: Pressure) {
        let _ = self.tx.send(Message::Trim(pressure));
    }

    /// Registers the host wake-up callback.
    ///
    /// Changes made outside a frame (a `surface.update`, a layer drop, a
    /// bound signal firing) are queued, not sent. When the display link is
    /// paused after `Next::Idle`, the host must learn that a frame is
    /// needed: the engine calls `f` at most once between two
    /// [`Engine::render`]s, the first time something is queued on a
    /// visible surface, and once when a surface becomes visible (see
    /// [`Surface::visibility`]). A hidden surface never calls it.
    ///
    /// # Panics
    /// Panics if the engine's callback slot is poisoned by a prior panic.
    pub fn set_waker(&self, f: impl Fn() + 'static) {
        *self.waker.callback.lock().expect("waker poisoned") = Some(Arc::new(f));
    }

    fn alloc(cell: &Cell<u64>) -> u64 {
        let id = cell.get();
        cell.set(id + 1);
        id
    }

    /// Registers a font. The data is validated here, before anything is
    /// queued, so the backend cannot reject it later.
    ///
    /// # Errors
    /// [`ResourceError::Font`] for empty or unparseable data,
    /// [`ResourceError::Unsupported`] for a font format the backend cannot
    /// draw, [`ResourceError::Lost`] when the executor is gone.
    pub fn font(&self, source: FontSource) -> Result<Font, ResourceError> {
        if source.data.is_empty() {
            return Err(ResourceError::Font("empty font data".into()));
        }
        let font = <B::Renderer as Renderer>::prepare_font(FontData {
            data: source.data,
            index: source.index,
        })?;
        let id = FontId::new(Self::alloc(&self.next_font));
        self.tx
            .send(Message::Resource(Box::new(move |r: &mut B::Renderer| {
                r.add_font(id, font);
            })))
            .map_err(|_| ResourceError::Lost)?;
        Ok(Font::new(
            id,
            self.on_release(ResourceId::Font(id), move |r| r.remove_font(id)),
        ))
    }

    /// Registers an image. [`Image::replace`] later swaps its pixels
    /// behind the same id.
    ///
    /// `image` is validated by [`ImageData::new`] before it is passed here.
    /// The upload is queued in order with every render and does not wait
    /// for the backend. A rejection only the backend can detect (a device
    /// limit) fails every render that draws the image with
    /// [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::Lost`] when the executor is gone.
    pub fn image<F: Format>(&self, image: ImageData<F>) -> Result<Image<F>, ResourceError>
    where
        B: Uploads<F>,
    {
        let id = ImageId::new(Self::alloc(&self.next_image));
        let upload = image.into_upload();
        let resource = ResourceId::Image(id);
        self.register(
            resource,
            Box::new(move |r: &mut B::Renderer| {
                Box::pin(std::future::ready(r.add_image(id, upload)))
            }),
        )?;
        Ok(Image::new(
            id,
            Rc::clone(&self.replace_image),
            self.on_release(resource, move |r| r.remove_image(id)),
        ))
    }

    /// Creates a surface over `target`: an [`Offscreen`](crate::Offscreen)
    /// texture or an interop window target.
    ///
    /// # Errors
    /// [`SurfaceError`] when the backend cannot draw the target, or
    /// [`SurfaceError::Lost`] when the render thread is gone.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    #[expect(
        clippy::arc_with_non_send_sync,
        reason = "everything here is single-threaded; `Arc` matches the native \
            surface waker the render loop shares, and the callback never \
            leaves this thread"
    )]
    pub async fn surface(&self, target: impl Into<B::Target>) -> Result<Surface<B>, SurfaceError> {
        let id = SurfaceId::new(Self::alloc(&self.next_surface));
        let waker = Arc::new(SurfaceWaker::new(Arc::clone(&self.waker)));
        let (reply, rx) = crate::local::channel();
        // The guard owns the surface id from the enqueue on: dropping the
        // future still destroys what `create_surface` committed (#150).
        let tx = self.tx.clone();
        let mut registration = Registration {
            release: Some(Box::new(move || {
                let _ = tx.send(Message::DestroySurface { id });
            })),
        };
        self.tx
            .send(Message::CreateSurface {
                id,
                target: target.into(),
                waker: Arc::clone(&waker),
                reply,
            })
            .map_err(|_| SurfaceError::Lost)?;
        match rx.recv().await {
            Ok(Ok(info)) => {
                registration.disarm();
                let surface = Surface::new(id, info, self.tx.clone(), waker);
                self.surfaces
                    .borrow_mut()
                    .push(Rc::downgrade(&surface.shared));
                Ok(surface)
            }
            Ok(Err(error)) => {
                registration.disarm();
                Err(error)
            }
            Err(_) => Err(SurfaceError::Lost),
        }
    }

    /// The number of surfaces still alive, for leak testing.
    #[doc(hidden)]
    pub fn live_surfaces(&self) -> usize {
        let mut surfaces = self.surfaces.borrow_mut();
        surfaces.retain(|weak| weak.strong_count() > 0);
        surfaces.len()
    }

    /// Applies queued commits, samples animations and renders every dirty
    /// visible surface on this JS thread. Browser work yields to the event
    /// loop. Mutations while awaiting this call belong to the next frame and
    /// still wake the host. A hidden surface is neither sampled nor drawn;
    /// its changes were applied as it made them.
    ///
    /// # Errors
    /// [`RenderError::Hidden`] when every surface is hidden, before
    /// anything is drained. Any other [`RenderError`] fails this call; a
    /// surface that failed to render is left in its previous state.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn render(&self, time: FrameTime) -> Result<Next, RenderError> {
        if super::all_hidden(&self.surfaces.borrow()) {
            return Err(RenderError::Hidden);
        }
        let mut commits = std::mem::take(&mut *self.commits.borrow_mut());
        commits.clear();
        super::drain_visible(&mut self.surfaces.borrow_mut(), time, &mut commits);
        // Re-arm before yielding: a signal fired during browser work must
        // request the next frame, even if the current frame returns Idle.
        self.waker.arm();
        let (reply, rx) = crate::local::channel::<RenderReply<B>>();
        self.tx
            .send(Message::Render {
                time,
                commits,
                reply,
            })
            .map_err(|_| RenderError::Thread)?;
        let mut reply = rx.recv().await.map_err(|_| RenderError::Thread)?;
        self.recycle_commits(&mut reply.commits);
        reply.commits.clear();
        *self.commits.borrow_mut() = reply.commits;
        let (next, stats) = reply.result?;
        *self.stats.borrow_mut() = stats;
        Ok(next)
    }

    /// Queues a registration the backend may reject; the executor records
    /// a rejection against `resource`.
    fn register(&self, resource: ResourceId, op: RegisterOp<B>) -> Result<(), ResourceError> {
        self.tx
            .send(Message::Register { resource, op })
            .map_err(|_| ResourceError::Lost)
    }

    fn recycle_commits(&self, commits: &mut [(SurfaceId, ChangeSet<B>)]) {
        let surfaces = self.surfaces.borrow();
        for (id, changes) in commits {
            let Some(shared) = surfaces
                .iter()
                .filter_map(std::rc::Weak::upgrade)
                .find(|shared| shared.borrow().id == *id)
            else {
                continue;
            };
            shared
                .borrow_mut()
                .recycle(std::mem::take(&mut changes.ops), &mut changes.recycled);
        }
    }

    fn on_drop(
        &self,
        op: impl FnOnce(&mut B::Renderer) + crate::RenderTransfer + 'static,
    ) -> impl FnOnce() + 'static {
        let post = Rc::clone(&self.post);
        move || post(Message::Resource(Box::new(op)))
    }

    /// The drop of a registered resource's last handle: the render loop
    /// runs `remove` once no surface's installed content draws the
    /// resource, and only when the backend holds it.
    fn on_release(
        &self,
        resource: ResourceId,
        remove: impl FnOnce(&mut B::Renderer) + crate::RenderTransfer + 'static,
    ) -> impl FnOnce() + 'static {
        let post = Rc::clone(&self.post);
        move || {
            post(Message::Release {
                resource,
                op: Box::new(remove),
            });
        }
    }
}

impl<B: ShaderPaint> Engine<B> {
    /// Registers a WGSL shader. The source is validated here, before
    /// anything is queued; pipeline creation is queued in order with every
    /// render and does not wait for the browser. A pipeline the browser
    /// cannot create fails every render that draws the shader with
    /// [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the source fails validation,
    /// [`ResourceError::Lost`] when the executor is gone.
    pub fn shader(&self, source: ShaderSource) -> Result<Shader, ResourceError> {
        B::validate_shader(&source)?;
        let id = ShaderId::new(Self::alloc(&self.next_shader));
        let resource = ResourceId::Shader(id);
        self.register(
            resource,
            Box::new(move |r: &mut B::Renderer| Box::pin(B::add_shader(r, id, source))),
        )?;
        Ok(Shader::new(
            id,
            self.on_release(resource, move |r| B::remove_shader(r, id)),
        ))
    }
}

impl<B: crate::BackdropShaders> Engine<B> {
    /// Registers a backdrop effect shader. The source is validated here,
    /// before anything is queued; pipeline creation is queued in order
    /// with every render and does not wait for the browser. A pipeline the
    /// browser cannot create fails every render that samples the shader
    /// with [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the source fails validation or its
    /// `reach` is not a finite non-negative number;
    /// [`ResourceError::Lost`] when the executor is gone.
    pub fn backdrop_shader(
        &self,
        source: crate::BackdropShaderSource,
    ) -> Result<crate::BackdropShader, ResourceError> {
        if !(source.reach.is_finite() && source.reach >= 0.0) {
            return Err(ResourceError::Shader(
                "backdrop shader reach must be a finite non-negative number".into(),
            ));
        }
        B::validate_backdrop_shader(&source)?;
        let id = crate::BackdropShaderId::new(Self::alloc(&self.next_backdrop_shader));
        let reach = source.reach;
        let resource = ResourceId::BackdropShader(id);
        self.register(
            resource,
            Box::new(move |r: &mut B::Renderer| Box::pin(B::add_backdrop_shader(r, id, source))),
        )?;
        Ok(crate::BackdropShader::new(
            id,
            reach,
            self.on_release(resource, move |r| B::remove_backdrop_shader(r, id)),
        ))
    }
}

impl<B: Filters> Engine<B> {
    /// Registers a [`filtrate_core::Filter`] run on the render thread.
    #[must_use]
    pub fn filter<F: filtrate_core::Filter + crate::RenderTransfer>(&self, filter: F) -> Filter
    where
        B: Runs<F>,
    {
        let id = FilterId::new(Self::alloc(&self.next_filter));
        (self.post)(Message::Resource(Box::new(move |r: &mut B::Renderer| {
            B::add_filter(r, id, filter);
        })));
        Filter::new(id, self.on_drop(move |r| B::remove_filter(r, id)))
    }

    /// Registers a custom effect run on the render thread.
    #[must_use]
    pub fn effect(&self, effect: impl Into<B::Effect>) -> Filter
    where
        B: Effects,
    {
        let id = FilterId::new(Self::alloc(&self.next_filter));
        let effect = effect.into();
        (self.post)(Message::Resource(Box::new(move |r: &mut B::Renderer| {
            B::add_effect(r, id, effect);
        })));
        Filter::new(id, self.on_drop(move |r| B::remove_filter(r, id)))
    }
}

impl<B: GpuContent> Engine<B> {
    /// Registers a GPU producer the engine's surfaces bind with
    /// [`GpuProducer::at`]. The handle is `Clone`; every clone of the one
    /// view instance's producer shares its renderer state, and there is
    /// no cache keyed by content — a second call is a second producer.
    /// The last drop retires it through the producer's own queue.
    #[must_use]
    pub fn gpu_producer(&self, content: impl Into<B::Content>) -> GpuProducer<B> {
        let id = ProducerId::new(Self::alloc(&self.next_producer));
        let content = content.into();
        (self.post)(Message::Resource(Box::new(move |r: &mut B::Renderer| {
            B::add_gpu_producer(r, id, content);
        })));
        GpuProducer::new(id, self.tx.clone())
    }

    /// Creates a submitted-frame producer — one whose frames come from
    /// the returned [`FrameSink`] instead of a `GpuContent` render — and
    /// its [`GpuProducer`] for binding it to layers with
    /// [`GpuProducer::at`].
    ///
    /// The producer has no setup and holds no content: its current frame
    /// is whatever the sink last submitted, on the device it was
    /// submitted to. A device replacement drops that frame; the next
    /// [`FrameSink::submit`] supplies one on the new device.
    #[must_use]
    pub fn frame_producer(&self) -> (GpuProducer<B>, FrameSink<B>) {
        let id = ProducerId::new(Self::alloc(&self.next_producer));
        let dirty = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gate = Arc::new(crate::WakeGate::default());
        (self.post)(Message::Resource(Box::new({
            let dirty = Arc::clone(&dirty);
            let gate = Arc::clone(&gate);
            move |r: &mut B::Renderer| {
                B::add_frame_producer(r, id, dirty, gate);
            }
        })));
        let engine_waker = Arc::clone(&self.waker);
        (
            GpuProducer::new(id, self.tx.clone()),
            FrameSink::new(
                id,
                self.tx.clone(),
                dirty,
                gate,
                Rc::new(move || engine_waker.wake()),
            ),
        )
    }

    /// The device-replacement contract: drains every live producer, the
    /// render loop dropping their device resources — current frames and
    /// frame rings — and releasing their bindings. A device replacement
    /// re-registers each rendered producer's content on a fresh renderer
    /// ([`gpu_producer`](Self::gpu_producer)) and rebinds; the first drawn
    /// binding then runs the producer's `setup` again on the new device.
    /// A frame producer's frame is device state and drops with the old
    /// device — recreate the pair with [`frame_producer`](Self::frame_producer)
    /// and submit again.
    ///
    /// # Errors
    /// [`RenderError::Thread`] when the executor is gone.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn drain_gpu_producers(
        &self,
    ) -> Result<Vec<(ProducerId, DrainedProducer<B>)>, RenderError> {
        let (reply, rx) = crate::local::channel();
        self.tx
            .send(Message::Resource(Box::new(move |r: &mut B::Renderer| {
                let _ = reply.send(B::drain_gpu_producers(r));
            })))
            .map_err(|_| RenderError::Thread)?;
        rx.recv().await.map_err(|_| RenderError::Thread)
    }
}

impl<B: Backend> Drop for Engine<B> {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
    }
}
