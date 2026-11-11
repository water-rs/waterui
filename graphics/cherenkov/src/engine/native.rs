//! The engine: owns the render thread and every resource's identity.
//! `!Send`, lives on the UI thread.

use super::{SurfaceWaker, thread};

use crate::transport::{FrameSlot, ReplyRx, SlotPool};
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
use crate::image::{Format, ImageData, ImageUpload};
use cherenkov_record::{ChangeSet, SurfaceId};

use crate::message::{FontData, Message, ProducerId, RegisterOp, RenderReply, ResOp};
use crate::paint::ImageId;
use cherenkov_record::ResourceId;

use crate::resource::{
    Filter, Font, FontSource, FrameSink, GpuProducer, Image, ImageBudget, ReplaceImage, Shader,
};
use crate::style::FilterId;
use crate::surface::Surface;

/// The engine: owns the device and the render thread. `!Send`, lives on
/// the UI thread.
///
/// The render thread owns every backend object. `Engine` is deliberately
/// `!Send` — everything it hands out (`Surface`, `Layer`, resource
/// handles) may only live on the UI thread that created the engine.
///
/// The native UI-to-render message queue is unbounded: an `Engine::Drop`,
/// a cancelled async surface registration or a resource drop can post a
/// `Shutdown`, `DestroySurface` or retirement while the render thread is
/// parked — a send must never be able to block on a full queue.
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<cherenkov::Engine<cherenkov::testing::Null>>();
/// ```
pub struct Engine<B: Backend> {
    tx: crossbeam_channel::Sender<Message<B>>,
    /// A producer's last `Arc` drop posts its retirement here — an
    /// unbounded queue the render loop waits on alongside `tx` and
    /// drains after each applied message, so a drop on the render
    /// thread itself never waits behind queued transactions and a
    /// retirement wakes the loop by itself.
    retire: crossbeam_channel::Sender<ResOp<B>>,
    info: B::Info,
    /// The largest image the backend admits, read once after init.
    /// The image budget every registration and replacement is admitted
    /// against on the calling thread, read once after init.
    images: Rc<ImageBudget>,
    stats: RefCell<FrameStats>,
    /// The memory query's persistent reply channel, reused across calls:
    /// the worker answers through the same request guard a frame's reply
    /// uses, so a dropped request closes it and a successful one leaves
    /// it reusable.
    memory_reply: crate::transport::Reply<crate::message::MemoryReply>,
    memory_reply_rx: crate::transport::ReplyRx<crate::message::MemoryReply>,
    /// The unique reusable frame slot — one `Render` in flight at a
    /// time, with its persistent response channel and commit buffers.
    /// The slot is the protocol's frame backpressure: a second `render`
    /// call waits for the first's lease, and a cancelled `render_async`
    /// hands it back with `pending` still set so the next owner drains
    /// the stale response before submitting.
    frame: SlotPool<FrameSlot<B>>,
    /// The live surfaces' shared queues, drained into one `Render` message
    /// per frame.
    surfaces: RefCell<super::Surfaces<B>>,
    next_surface: Cell<u64>,
    next_font: Cell<u64>,
    next_image: Cell<u64>,
    next_shader: Cell<u64>,
    next_filter: Cell<u64>,
    next_backdrop_shader: Cell<u64>,
    next_producer: Cell<u64>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The type-erased sender resource drops use; a render thread that is
    /// gone has nothing left to release.
    post: Rc<dyn Fn(Message<B>)>,
    /// The `Message::ReplaceImage` sender every image handle shares.
    replace_image: ReplaceImage,
    // `!Send`: the engine lives on the UI thread.
    _not_send: PhantomData<Rc<()>>,
}

impl<B: Backend> std::fmt::Debug for Engine<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl<B: Backend> Engine<B> {
    /// Spawns the render thread and runs `B::init` on it, blocking until it
    /// reports success or failure.
    ///
    /// # Errors
    /// [`EngineError`] when the backend fails to initialize or the render
    /// thread cannot start.
    pub fn new(config: B::Config) -> Result<Self, EngineError> {
        let spawn = Self::spawn(config)?;
        let (info, image_limits) = spawn
            .init_rx
            .recv_blocking()
            .map_err(|_| EngineError::Thread("render thread died during init".into()))??;
        Ok(Self::from_spawn(spawn, info, image_limits))
    }

    /// The asynchronous engine constructor the final interface uses:
    /// the render thread's init reply resolves the future, and a
    /// cancellation before it arrives posts `Shutdown` to the nascent
    /// worker without joining it on the caller's thread.
    ///
    /// # Errors
    /// [`EngineError`] when the backend fails to initialize or the render
    /// thread cannot start.
    pub async fn new_async(config: B::Config) -> Result<Self, EngineError> {
        let mut spawn = Self::spawn(config)?;
        // The guard owns the nascent connection until init resolves:
        // dropping the future releases it, which posts Shutdown and
        // detaches the worker — it exits after its ordered shutdown
        // without the caller's thread joining it.
        let guard = InitGuard::<B> {
            tx: Some(spawn.tx.clone()),
            thread: spawn.thread.take(),
            _engine: PhantomData,
        };
        let result = spawn
            .init_rx
            .recv()
            .await
            .map_err(|_| EngineError::Thread("render thread died during init".into()));
        let (info, image_limits) = match result {
            Ok(result) => result,
            Err(error) => {
                drop(guard);
                return Err(error);
            }
        }?;
        spawn.thread = guard.disarm();
        Ok(Self::from_spawn(spawn, info, image_limits))
    }

    /// Spawns the render thread and returns the pieces either
    /// constructor's wait then turns into an `Engine`.
    fn spawn(config: B::Config) -> Result<Spawn<B>, EngineError> {
        let (tx, rx) = crossbeam_channel::unbounded::<Message<B>>();
        let (retire_tx, retire_rx) = crossbeam_channel::unbounded::<ResOp<B>>();
        let (init_tx, init_rx) = crate::transport::reply();
        let render_thread = std::thread::Builder::new()
            .name("cherenkov-render".into())
            .spawn(move || thread::run::<B>(config, &rx, &retire_rx, &init_tx))
            .map_err(|e| EngineError::Thread(format!("spawn failed: {e}")))?;
        Ok(Spawn {
            tx,
            retire_tx,
            init_rx,
            thread: Some(render_thread),
        })
    }

    /// Assembles the engine once the render thread reports its
    /// capabilities — the shared body of `new` and `new_async`.
    fn from_spawn(spawn: Spawn<B>, info: B::Info, image_limits: crate::ImageLimits) -> Self {
        let Spawn {
            tx,
            retire_tx,
            thread: render_thread,
            ..
        } = spawn;
        let post_tx = tx.clone();
        let (memory_reply, memory_reply_rx) = crate::transport::reply();
        let replace_image = {
            let tx = tx.clone();
            // The render loop wakes the host through every visible surface
            // that draws the image, once it knows which surfaces do.
            Rc::new(move |id: ImageId, image: ImageUpload| {
                tx.send(Message::ReplaceImage { id, image })
                    .map_err(|_| ResourceError::Lost)
            }) as ReplaceImage
        };
        Self {
            tx,
            retire: retire_tx,
            info,
            images: ImageBudget::new(image_limits),
            stats: RefCell::new(FrameStats::default()),
            memory_reply,
            memory_reply_rx,
            frame: SlotPool::new(FrameSlot::new()),
            surfaces: RefCell::new(Vec::new()),
            next_surface: Cell::new(0),
            next_font: Cell::new(1),
            next_image: Cell::new(1),
            next_shader: Cell::new(1),
            next_filter: Cell::new(1),
            next_backdrop_shader: Cell::new(1),
            next_producer: Cell::new(1),
            thread: render_thread,
            post: Rc::new(move |message| {
                let _ = post_tx.send(message);
            }),
            replace_image,
            _not_send: PhantomData,
        }
    }

    /// The backend's provenance (`B::Info`).
    #[must_use]
    pub const fn info(&self) -> &B::Info {
        &self.info
    }

    /// The images the backend admits: the largest image in each
    /// dimension (the device's texture limit) and the image budget, the
    /// bytes every registered image holds together (the backend's memory
    /// budget). Read once off the live device and budget when the engine
    /// is created; it never changes.
    ///
    /// [`Engine::image`] and [`Image::replace`] check it on the calling
    /// thread before anything is queued, so an image the device cannot
    /// hold fails at registration with [`ResourceError::TooLarge`], and
    /// one that does not fit beside the images already registered with
    /// [`ResourceError::OverBudget`], instead of failing every render that
    /// draws it.
    #[must_use]
    pub fn image_limits(&self) -> crate::ImageLimits {
        self.images.limits()
    }

    /// Statistics of the last [`Engine::render`]. GPU timings are kept by
    /// the render thread and returned by [`Engine::finish_timings`] instead.
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.stats.borrow().clone()
    }

    /// Returns all GPU timings accumulated since the previous call,
    /// oldest first. Blocks the render thread until the GPU finishes frames
    /// still in flight, so this marks the end of a measured window and
    /// belongs to tooling, never to a frame path. Empty when the backend
    /// reports no GPU timing or has nothing outstanding.
    ///
    /// # Errors
    /// [`RenderError::Timeout`] when the GPU makes no progress for a
    /// wait window, [`RenderError::Readback`] when a timing buffer cannot
    /// be read, and [`RenderError::Thread`] when the render thread is
    /// gone.
    pub fn finish_timings(&self) -> Result<Vec<FrameTiming>, RenderError> {
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Message::FinishTimings { reply })
            .map_err(|_| RenderError::Thread)?;
        rx.recv().map_err(|_| RenderError::Thread)?
    }

    /// The engine's current memory usage.
    ///
    #[must_use]
    pub fn memory(&self) -> MemoryUsage {
        if self
            .tx
            .send(Message::Memory {
                reply: self.memory_reply.request(),
            })
            .is_err()
        {
            return MemoryUsage::default();
        }
        let Ok(reply) = self.memory_reply_rx.recv_blocking() else {
            return MemoryUsage::default();
        };
        reply.usage
    }

    /// Reports system memory pressure. `Critical` drops every cache.
    pub fn trim(&self, pressure: Pressure) {
        let _ = self.tx.send(Message::Trim(pressure));
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
    /// draw, [`ResourceError::Lost`] when the render thread is gone.
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
    /// `image` is validated by [`ImageData::new`] before it is passed here,
    /// and its size is checked against [`Engine::image_limits`] on the
    /// calling thread: an image the device cannot hold, or one that does
    /// not fit the image budget beside the images already registered,
    /// fails at registration instead of failing every render that draws
    /// it. The image holds its share of the budget until its last handle
    /// drops. The upload is queued in order with every render and does
    /// not wait for the backend. A rejection only the backend can detect
    /// (an allocation the device cannot back) leaves every surface whose
    /// content draws the image out of the frame, reported as
    /// [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::TooLarge`] when the image exceeds
    /// [`Engine::image_limits`], [`ResourceError::OverBudget`] when it
    /// does not fit the image budget beside the images already
    /// registered, [`ResourceError::Lost`] when the render
    /// thread is gone.
    pub fn image<F: Format>(&self, image: ImageData<F>) -> Result<Image<F>, ResourceError>
    where
        B: Uploads<F>,
    {
        let bytes = self.images.admit::<F>(image.width(), image.height(), 0)?;
        let id = ImageId::new(Self::alloc(&self.next_image));
        let upload = image.into_upload();
        let resource = ResourceId::Image(id);
        self.register(
            resource,
            Box::new(move |r: &mut B::Renderer| r.add_image(id, upload)),
        )?;
        self.images.settle(0, bytes);
        Ok(Image::new(
            id,
            Rc::clone(&self.replace_image),
            Rc::clone(&self.images),
            bytes,
            self.on_release(resource, move |r| r.remove_image(id)),
        ))
    }

    /// Creates a surface over `target`: an [`Offscreen`](crate::Offscreen)
    /// texture or an interop window target. `wake` is the surface's host
    /// wake-up, fixed for the surface's life.
    ///
    /// Changes made outside a frame (a `surface.update`, a layer drop, a
    /// bound signal firing) are queued, not sent. When the display link is
    /// paused after `Next::Idle`, the host must learn that a frame is
    /// needed: the engine calls `wake` at most once between two
    /// [`Engine::render`]s the surface participates in — the first time
    /// something is queued on it, a backend completion lands for it, an
    /// image or a producer frame it draws lands, or a producer or filter
    /// it draws asks for a redraw — and once when the surface becomes
    /// visible (see [`Surface::visibility`]). A hidden surface never calls
    /// it. `wake` may run on the engine's thread, on the render thread —
    /// an image replacement or a submitted frame wakes the host from there
    /// once it knows the surface draws it — on a producer's or a filter
    /// parameter's thread, or on the main thread, where a completion the
    /// render thread queued fires it, so it must be [`Send`] and [`Sync`].
    ///
    /// A host that keeps one presentation loop behind several surfaces
    /// passes each of them the same request-redraw callback. A host that
    /// drives its own frames opens each with [`Surface::begin_frame`] and
    /// holds the scope across the frame's render, so the edits it makes
    /// for that frame do not ask for another.
    ///
    /// # Errors
    /// [`SurfaceError`] when the backend cannot draw the target, or
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn surface(
        &self,
        target: impl Into<B::Target>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Surface<B>, SurfaceError> {
        let (id, waker, rx, mut registration) = self.start_surface(target, wake)?;
        let info = rx.recv_blocking().map_err(|_| SurfaceError::Lost)??;
        registration.disarm();
        Ok(self.finish_surface(id, info, waker))
    }

    /// The asynchronous surface constructor the final interface uses:
    /// the worker's `CreateSurface` reply resolves the future, and a
    /// cancellation before or after the worker handles it posts
    /// `DestroySurface` for the same id — nothing is left registered.
    ///
    /// # Errors
    /// [`SurfaceError`] when the backend refuses the target or the render
    /// thread is gone.
    #[expect(
        clippy::future_not_send,
        reason = "the engine is !Send — its futures run on the UI thread that owns it"
    )]
    pub async fn surface_async(
        &self,
        target: impl Into<B::Target>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Surface<B>, SurfaceError> {
        let (id, waker, rx, mut registration) = self.start_surface(target, wake)?;
        let info = match rx.recv().await {
            Ok(Ok(info)) => info,
            Ok(Err(error)) => {
                registration.disarm();
                return Err(error);
            }
            Err(_) => return Err(SurfaceError::Lost),
        };
        registration.disarm();
        Ok(self.finish_surface(id, info, waker))
    }

    /// The shared surface start: allocs the id, wires the wake, arms the
    /// registration guard — its drop posts `DestroySurface` — and queues
    /// `CreateSurface`.
    fn start_surface(
        &self,
        target: impl Into<B::Target>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<SurfaceStart<B>, SurfaceError> {
        let id = SurfaceId::new(Self::alloc(&self.next_surface));
        let waker = Arc::new(SurfaceWaker::new(wake));
        let (reply, rx) = crate::transport::reply();
        let tx = self.tx.clone();
        let registration = Registration::<B> {
            release: Some(Box::new(move || {
                let _ = tx.send(Message::DestroySurface { id });
            })),
            _engine: PhantomData,
        };
        self.tx
            .send(Message::CreateSurface {
                id,
                target: target.into(),
                waker: Arc::clone(&waker),
                reply,
            })
            .map_err(|_| SurfaceError::Lost)?;
        Ok((id, waker, rx, registration))
    }

    /// The shared surface start's pieces: id, waker, reply receiver and
    /// the registration guard that destroys the id on cancel.
    /// The shared surface finish: builds the handle and registers it
    /// with the engine's weak list.
    fn finish_surface(
        &self,
        id: SurfaceId,
        info: crate::backend::SurfaceInfo,
        waker: Arc<SurfaceWaker>,
    ) -> Surface<B> {
        let surface = Surface::new(id, info, self.tx.clone(), waker);
        self.surfaces.borrow_mut().push(super::SurfaceEntry {
            shared: Rc::downgrade(&surface.shared),
            waker: Arc::clone(&surface.waker),
            next_frame: Rc::downgrade(&surface.next_frame),
        });
        surface
    }

    /// The number of surfaces still alive, for leak testing.
    #[doc(hidden)]
    pub fn live_surfaces(&self) -> usize {
        let mut surfaces = self.surfaces.borrow_mut();
        surfaces.retain(|entry| entry.shared.strong_count() > 0);
        surfaces.len()
    }

    /// Renders every dirty visible surface for the frame at `time`,
    /// blocking until the render thread has applied the queued commits,
    /// sampled the animations and rendered. A hidden surface is neither
    /// sampled nor drawn; its changes were applied as it made them.
    ///
    /// # Errors
    /// [`RenderError::Hidden`] when every surface is hidden, before
    /// anything is drained. [`RenderError::Rejected`] when the content of
    /// some surfaces draws a resource the backend rejected: those surfaces
    /// were left out of the frame, and every other surface rendered. Any
    /// other [`RenderError`] fails this call; a surface that failed to
    /// render is left in its previous state.
    pub fn render(&self, time: FrameTime) -> Result<Next, RenderError> {
        let mut slot = self
            .frame
            .acquire_blocking()
            .map_err(|_| RenderError::Thread)?;
        self.drain_pending(&mut slot)?;
        self.submit_render(&mut slot, time)?;
        let reply = slot
            .reply_rx
            .recv_blocking()
            .map_err(|_| RenderError::Thread)?;
        slot.pending = false;
        self.finish_render(&mut slot, reply)
    }

    /// The asynchronous frame request the final interface uses: the
    /// worker's `Render` reply resolves the future, and cancelling it
    /// returns the frame slot still marked pending — the next owner
    /// drains and discards the stale response's presentation before
    /// submitting its own frame, never applying a commit its caller
    /// is not waiting for.
    ///
    /// # Errors
    /// [`RenderError`] under the same conditions as [`Engine::render`].
    #[expect(
        clippy::future_not_send,
        reason = "the engine is !Send — its futures run on the UI thread that owns it"
    )]
    pub async fn render_async(&self, time: FrameTime) -> Result<Next, RenderError> {
        let mut slot = self
            .frame
            .acquire()
            .await
            .map_err(|_| RenderError::Thread)?;
        self.drain_pending_async(&mut slot).await?;
        self.submit_render(&mut slot, time)?;
        let Ok(reply) = slot.reply_rx.recv().await else {
            slot.pending = false;
            return Err(RenderError::Thread);
        };
        slot.pending = false;
        self.finish_render(&mut slot, reply)
    }

    /// The abandoned-reply drain the synchronous `render` bridge uses: a
    /// cancelled `render_async` leaves the slot marked `pending` with its
    /// reply posted or still owed — receive it, recycle its buffers and
    /// drop the presentation commit, which is never applied for a caller
    /// that is not waiting.
    fn drain_pending(
        &self,
        slot: &mut crate::transport::SlotLease<'_, FrameSlot<B>>,
    ) -> Result<(), RenderError> {
        if !slot.pending {
            return Ok(());
        }
        match slot.reply_rx.recv_blocking() {
            Ok(stale) => self.recycle_stale(slot, stale),
            Err(_) => return Err(RenderError::Thread),
        }
        slot.pending = false;
        Ok(())
    }

    /// The awaitable [`Self::drain_pending`] for `render_async`.
    #[expect(
        clippy::future_not_send,
        reason = "the engine is !Send — its futures run on the UI thread that owns it"
    )]
    async fn drain_pending_async(
        &self,
        slot: &mut crate::transport::SlotLease<'_, FrameSlot<B>>,
    ) -> Result<(), RenderError> {
        if !slot.pending {
            return Ok(());
        }
        match slot.reply_rx.recv().await {
            Ok(stale) => self.recycle_stale(slot, stale),
            Err(_) => return Err(RenderError::Thread),
        }
        slot.pending = false;
        Ok(())
    }

    /// The stale reply's buffers recycle into the slot; its presentation
    /// commit drops un-applied.
    fn recycle_stale(
        &self,
        slot: &mut crate::transport::SlotLease<'_, FrameSlot<B>>,
        mut stale: RenderReply<B>,
    ) {
        self.recycle_commits(&mut stale.commits);
        stale.commits.clear();
        slot.commits = stale.commits;
    }

    /// The shared frame submission: hidden-surfaces check, commit drain,
    /// `Render` send, then the pending mark — set only after the send
    /// lands so a failed send leaves the slot clean.
    fn submit_render(
        &self,
        slot: &mut crate::transport::SlotLease<'_, FrameSlot<B>>,
        time: FrameTime,
    ) -> Result<(), RenderError> {
        if super::all_hidden(&self.surfaces.borrow()) {
            return Err(RenderError::Hidden);
        }
        let mut commits = std::mem::take(&mut slot.commits);
        commits.clear();
        // Re-arms each drained surface's wake before the render is sent:
        // completions may arrive while it is in flight, before its reply.
        super::drain_visible(&mut self.surfaces.borrow_mut(), time, &mut commits);
        if let Err(error) = self.tx.send(Message::Render {
            time,
            commits,
            reply: slot.reply.request(),
        }) {
            if let Message::Render { commits, .. } = error.0 {
                slot.commits = commits;
            }
            return Err(RenderError::Thread);
        }
        slot.pending = true;
        Ok(())
    }

    /// The shared frame finish: recycle the reply's buffers, apply its
    /// presentation commit on the caller's thread, publish `next_frame`
    /// and stats — exactly once, after a live reply.
    fn finish_render(
        &self,
        slot: &mut crate::transport::SlotLease<'_, FrameSlot<B>>,
        mut reply: RenderReply<B>,
    ) -> Result<Next, RenderError> {
        self.recycle_commits(&mut reply.commits);
        reply.commits.clear();
        slot.commits = reply.commits;
        let frame = reply.result?;
        // The frame's main-thread work applies here, on the awaiting
        // caller's thread, before the render resolves: a window surface's
        // drawable presents land inside this call, ordered before the
        // next frame's acquire — a frame that left surfaces out included.
        // `RenderError` above drops the commit with its acquired
        // drawables, which a later render re-acquires.
        <B::Renderer as crate::backend::Renderer>::apply_frame_commit(frame.commit);
        super::publish_next(&self.surfaces.borrow(), &frame.surface_next);
        *self.stats.borrow_mut() = frame.stats;
        super::rendered(frame.next, frame.rejected)
    }

    fn recycle_commits(&self, commits: &mut [(SurfaceId, ChangeSet<B>)]) {
        let surfaces = self.surfaces.borrow();
        for (id, changes) in commits {
            let Some(shared) = surfaces
                .iter()
                .filter_map(|entry| entry.shared.upgrade())
                .find(|shared| shared.borrow().id == *id)
            else {
                continue;
            };
            shared
                .borrow_mut()
                .recycle(std::mem::take(&mut changes.ops), &mut changes.recycled);
        }
    }

    /// Queues a registration the backend may reject; the render loop
    /// records a rejection against `resource`.
    fn register(&self, resource: ResourceId, op: RegisterOp<B>) -> Result<(), ResourceError> {
        self.tx
            .send(Message::Register { resource, op })
            .map_err(|_| ResourceError::Lost)
    }

    fn on_drop(
        &self,
        op: impl FnOnce(&mut B::Renderer) + Send + 'static,
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
        remove: impl FnOnce(&mut B::Renderer) + Send + 'static,
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
    /// render and does not wait for the backend. A pipeline the backend
    /// cannot create leaves every surface whose content draws the shader
    /// out of the frame, reported as [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the source fails validation,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn shader(&self, source: ShaderSource) -> Result<Shader, ResourceError> {
        B::validate_shader(&source)?;
        let id = ShaderId::new(Self::alloc(&self.next_shader));
        let resource = ResourceId::Shader(id);
        self.register(
            resource,
            Box::new(move |r: &mut B::Renderer| B::add_shader(r, id, source)),
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
    /// with every render and does not wait for the backend. A pipeline the
    /// backend cannot create leaves every surface that samples the shader
    /// out of the frame, reported as [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the source fails validation or its
    /// `reach` is not a finite non-negative number;
    /// [`ResourceError::Lost`] when the render thread is gone.
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
            Box::new(move |r: &mut B::Renderer| B::add_backdrop_shader(r, id, source)),
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
    pub fn filter<F: filtrate_core::Filter + Send>(&self, filter: F) -> Filter
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
        GpuProducer::new(id, self.retire.clone())
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
        (self.post)(Message::Resource(Box::new(move |r: &mut B::Renderer| {
            B::add_frame_producer(r, id);
        })));
        (
            GpuProducer::new(id, self.retire.clone()),
            FrameSink::new(id, self.tx.clone()),
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
    /// [`RenderError::Thread`] when the render thread is gone.
    pub fn drain_gpu_producers(
        &self,
    ) -> Result<Vec<(ProducerId, DrainedProducer<B>)>, RenderError> {
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Message::Resource(Box::new(move |r: &mut B::Renderer| {
                let _ = reply.send(B::drain_gpu_producers(r));
            })))
            .map_err(|_| RenderError::Thread)?;
        rx.recv().map_err(|_| RenderError::Thread)
    }
}

/// The pieces `Engine::spawn` hands to either constructor's wait: the
/// transaction channel, the retirement queue, init's reply receiver and
/// the worker's handle — everything `Engine` owns or the guard needs.
struct Spawn<B: Backend> {
    tx: crossbeam_channel::Sender<Message<B>>,
    retire_tx: crossbeam_channel::Sender<ResOp<B>>,
    init_rx: ReplyRx<Result<(B::Info, crate::ImageLimits), EngineError>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Owns the nascent connection during `new_async`'s init wait: a
/// cancellation posts `Shutdown` and detaches the worker — it exits after
/// its ordered shutdown without the caller's thread joining it. Success
/// disarms; only then does the engine take over the channels.
struct InitGuard<B: Backend> {
    tx: Option<crossbeam_channel::Sender<Message<B>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _engine: PhantomData<B>,
}

impl<B: Backend> InitGuard<B> {
    /// `new_async`'s init resolved — the engine takes the worker handle;
    /// disarming empties the guard, so its destructor posts nothing.
    fn disarm(mut self) -> Option<std::thread::JoinHandle<()>> {
        self.tx.take();
        self.thread.take()
    }
}

impl<B: Backend> Drop for InitGuard<B> {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Message::Shutdown);
        }
        self.thread.take();
    }
}

/// The shared surface start's pieces: the allocated id, its waker, the
/// reply receiver and the guard that destroys the id on cancel.
type SurfaceStart<B> = (
    SurfaceId,
    Arc<SurfaceWaker>,
    ReplyRx<Result<crate::backend::SurfaceInfo, SurfaceError>>,
    Registration<B>,
);

/// Owns a surface id from `CreateSurface`'s enqueue on: dropping the
/// `surface_async` future still destroys what the worker committed —
/// the same registration protocol the browser engine's `Registration`
/// runs (#150).
struct Registration<B: Backend> {
    release: Option<Box<dyn FnOnce()>>,
    _engine: PhantomData<B>,
}

impl<B: Backend> Registration<B> {
    /// The worker replied and the registration survived — the id is the
    /// surface's now, no destruction to post.
    fn disarm(&mut self) {
        self.release = None;
    }
}

impl<B: Backend> Drop for Registration<B> {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl<B: Backend> Drop for Engine<B> {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        // The worker exits after its ordered shutdown; the handle drops
        // detached — a synchronous join on the UI thread is what #3753
        // removes, and the worker releases its own resources on exit.
        self.thread.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Event, Null, NullConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A binding's last reference can die on the render thread — the
    /// unbind a `clear_content` runs drops the `GpuProducer` clone the
    /// binding held. Parked behind another message, the retirement must
    /// still land and the loop must not hang: it rides the producer's
    /// own unbounded queue, never `tx`.
    #[test]
    fn a_render_thread_binding_drop_retires_without_hanging() {
        let (events, rx) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .unwrap();
        let surface = engine
            .surface(
                crate::Offscreen::new((16, 16), crate::OffscreenFormat::LinearF16),
                || {},
            )
            .unwrap();
        surface.visibility(crate::Visibility::Hidden).unwrap();
        let layer = surface.layer();
        let producer = engine.gpu_producer(());
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
            tx[&layer].content(producer.at((8, 8)));
        });
        // A hidden surface's ops go straight out as `Message::Apply`;
        // BindProducer names the binding once it is installed on the
        // render thread — the clone it holds is then the producer's
        // only reference.
        let id = producer.id();
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(Event::BindProducer(_, _, bound)) if bound == id => break,
                Ok(_) => {}
                other => panic!("binding never installed: {other:?}"),
            }
        }
        drop(producer);

        // Park the render thread and queue the unbind behind it, so the
        // retirement lands while the loop sits inside the parked
        // message — a blocking retire send could never complete if it
        // rode `tx`.
        let (parked_tx, parked_rx) = std::sync::mpsc::channel::<()>();
        engine
            .tx
            .send(Message::Resource(Box::new(move |_| {
                let _ = parked_rx.recv();
            })))
            .unwrap();
        surface.update(|tx| {
            tx[&layer].clear_content();
        });
        // The transaction channel is unbounded, so the retirement can
        // never be held behind a full queue the way the bounded
        // transaction channel allowed — the parked gate still makes the
        // retirement ride the producer's own unbounded queue while the
        // loop sits inside the parked message.
        drop(parked_tx);

        // The render thread drains: the Apply unbinds and drops the
        // last clone — the retirement goes through the producer's
        // unbounded queue. `RetireProducer` landing proves the loop
        // never wedged on a blocking send.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Event::RetireProducer(retired)) if retired == id => break,
                _ => assert!(
                    std::time::Instant::now() < deadline,
                    "retirement never landed"
                ),
            }
        }
        // And the loop kept consuming: a transaction queued behind the
        // parked gate still applies.
        surface.visibility(crate::Visibility::Visible).unwrap();
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Event::Visibility(_, crate::Visibility::Visible)) => break,
                _ => assert!(
                    std::time::Instant::now() < deadline,
                    "render loop stopped consuming"
                ),
            }
        }
    }

    /// A frame producer's submitted frame wakes, from the render loop once
    /// the frame has landed, the host of each surface a binding of the
    /// producer is on — through that surface's own wake, coalesced until
    /// the surface next renders — and nothing else: not a surface that
    /// does not draw it, not a hidden one, not a dropped one. A frame that
    /// lands after a render wakes again.
    #[test]
    fn submitted_frames_wake_exactly_the_drawing_surfaces() {
        let (events, _rx) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .unwrap();
        let counted = |count: &Arc<AtomicUsize>| {
            let count = Arc::clone(count);
            move || {
                count.fetch_add(1, Ordering::Relaxed);
            }
        };
        let target = || crate::Offscreen::new((8, 8), crate::OffscreenFormat::LinearF16);
        let [drawing, idle, hidden, dropped] =
            std::array::from_fn::<_, 4, _>(|_| Arc::new(AtomicUsize::new(0)));
        let shown = engine.surface(target(), counted(&drawing)).unwrap();
        let _unbound = engine.surface(target(), counted(&idle)).unwrap();
        let concealed = engine.surface(target(), counted(&hidden)).unwrap();
        let gone = engine.surface(target(), counted(&dropped)).unwrap();
        let (producer, sink) = engine.frame_producer();
        let bind = |surface: &Surface<Null>| {
            let layer = surface.layer();
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(producer.at((8, 8)));
            });
            layer
        };
        let _layers = [bind(&shown), bind(&concealed), bind(&gone)];
        engine.render(FrameTime::now()).unwrap();
        concealed.visibility(crate::Visibility::Hidden).unwrap();
        drop(gone);
        for count in [&drawing, &idle, &hidden, &dropped] {
            count.store(0, Ordering::Relaxed);
        }

        sink.submit(());
        sink.submit(());
        // The reply follows every earlier message: both frames have landed.
        let _ = engine.memory();
        assert_eq!(
            drawing.load(Ordering::Relaxed),
            1,
            "a drawing surface wakes once until it renders"
        );
        assert_eq!(
            idle.load(Ordering::Relaxed),
            0,
            "a surface not drawing it woke"
        );
        assert_eq!(hidden.load(Ordering::Relaxed), 0, "a hidden surface woke");
        assert_eq!(dropped.load(Ordering::Relaxed), 0, "a dropped surface woke");

        engine.render(FrameTime::now()).unwrap();
        sink.submit(());
        let _ = engine.memory();
        assert_eq!(
            drawing.load(Ordering::Relaxed),
            2,
            "a frame landing after a render wakes again"
        );
    }

    /// A Null engine and one visible surface whose host wake counts its
    /// calls, rendered once so the count starts at zero with the wake
    /// armed.
    fn counted_surface() -> (Engine<Null>, Surface<Null>, Arc<AtomicUsize>) {
        let (events, _) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let surface = engine
            .surface(
                crate::Offscreen::new((8, 8), crate::OffscreenFormat::LinearF16),
                {
                    let wakes = Arc::clone(&wakes);
                    move || {
                        wakes.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .unwrap();
        engine.render(FrameTime::now()).unwrap();
        wakes.store(0, Ordering::Relaxed);
        (engine, surface, wakes)
    }

    /// Queues an edit on the surface's root.
    fn edit(surface: &Surface<Null>, opacity: f32) {
        surface.update(|tx| {
            tx[surface.root()].opacity(opacity);
        });
    }

    /// The edits a frame makes inside its scope wake nothing; its render
    /// answers them and re-arms the wake, which the scope's end leaves
    /// alone.
    #[test]
    fn a_frame_scope_holds_its_edits_until_its_render_rearms() {
        let (engine, surface, wakes) = counted_surface();
        let frame = surface.begin_frame();
        edit(&surface, 0.5);
        edit(&surface, 0.25);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            0,
            "an edit in the scope woke"
        );
        engine.render(FrameTime::now()).unwrap();
        drop(frame);
        assert_eq!(wakes.load(Ordering::Relaxed), 0, "the scope's end woke");
        edit(&surface, 0.75);
        edit(&surface, 1.0);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "the render re-armed the wake once"
        );
    }

    /// A scope that ends without its render delivers what it owes, once.
    #[test]
    fn a_frame_scope_dropped_owing_a_wake_delivers_exactly_one() {
        let (_engine, surface, wakes) = counted_surface();
        let frame = surface.begin_frame();
        edit(&surface, 0.5);
        edit(&surface, 0.25);
        drop(frame);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "the owed wake was delivered once"
        );
        edit(&surface, 0.75);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "the delivered wake coalesces until a render"
        );
    }

    /// A scope that ends without its render and owes nothing re-arms.
    #[test]
    fn a_frame_scope_dropped_without_a_debt_rearms() {
        let (_engine, surface, wakes) = counted_surface();
        drop(surface.begin_frame());
        assert_eq!(wakes.load(Ordering::Relaxed), 0, "an empty scope woke");
        edit(&surface, 0.5);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "the scope's end re-armed the wake"
        );
    }

    /// A frame submitted from another thread while a render is in flight
    /// lands behind that render and still wakes the host.
    #[test]
    fn a_frame_submitted_during_an_in_flight_render_wakes_the_host() {
        let (engine, surface, wakes) = counted_surface();
        let (producer, sink) = engine.frame_producer();
        let layer = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
            tx[&layer].content(producer.at((8, 8)));
        });
        engine.render(FrameTime::now()).unwrap();
        wakes.store(0, Ordering::Relaxed);

        // The render thread is parked, so the next `Render` waits in the
        // channel while the frame is submitted behind it.
        let (parked, park) = std::sync::mpsc::channel::<()>();
        let (release, held) = std::sync::mpsc::channel::<()>();
        engine
            .tx
            .send(Message::Resource(Box::new(move |_| {
                parked.send(()).unwrap();
                let _ = held.recv();
            })))
            .unwrap();
        park.recv().unwrap();
        let submitter = std::thread::spawn({
            let tx = engine.tx.clone();
            move || {
                while tx.is_empty() {
                    std::thread::yield_now();
                }
                sink.submit(());
                drop(release);
            }
        });
        engine.render(FrameTime::now()).unwrap();
        submitter.join().unwrap();
        // The reply follows every earlier message: the frame has landed.
        let _ = engine.memory();
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "a frame landing behind an in-flight render wakes the host"
        );
    }

    #[test]
    fn completion_before_render_reply_wakes_the_host() {
        let (events, _) = std::sync::mpsc::channel();
        let mut engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .unwrap();
        let wake_count = Arc::new(AtomicUsize::new(0));
        let surface = engine
            .surface(
                crate::Offscreen::new((8, 8), crate::OffscreenFormat::LinearF16),
                {
                    let wake_count = Arc::clone(&wake_count);
                    move || {
                        wake_count.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .unwrap();
        // Replace only the render transport. The real native render method
        // receives a completion after submission but before its reply.
        engine.tx.send(Message::Shutdown).unwrap();
        engine.thread.take().unwrap().join().unwrap();
        let (tx, rx) = crossbeam_channel::bounded(1);
        engine.tx = tx;
        surface.waker.wake();
        let waker = Arc::clone(&surface.waker);
        engine.thread = Some(std::thread::spawn(move || {
            let Message::Render { commits, reply, .. } = rx.recv().unwrap() else {
                panic!("expected render");
            };
            waker.wake();
            reply.send(RenderReply {
                result: Ok(crate::message::RenderedFrame {
                    next: Next::Idle,
                    surface_next: rustc_hash::FxHashMap::default(),
                    stats: FrameStats::default(),
                    commit: (),
                    rejected: Vec::new(),
                }),
                commits,
            });
            assert!(matches!(rx.recv().unwrap(), Message::DestroySurface { .. }));
            assert!(matches!(rx.recv().unwrap(), Message::Shutdown));
        }));
        assert_eq!(engine.render(FrameTime::now()).unwrap(), Next::Idle);
        assert_eq!(wake_count.load(Ordering::Relaxed), 2);
        drop(surface);
    }

    /// `Surface::next_frame` publishes each surface's own deadline and
    /// only its own: a hidden surface drops to `Idle` instead of
    /// retaining or borrowing demand, a finished animation overwrites
    /// its old `At` with a real idle frame, and removal clears a live
    /// deadline — all while the surviving peer's `At` stays untouched.
    #[test]
    fn hide_idle_and_remove_publish_idle_while_peer_keeps_demand() {
        let (events, _rx) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .unwrap();
        let surface = engine
            .surface(
                crate::Offscreen::new((16, 16), crate::OffscreenFormat::LinearF16),
                || {},
            )
            .unwrap();
        let peer = engine
            .surface(
                crate::Offscreen::new((16, 16), crate::OffscreenFormat::LinearF16),
                || {},
            )
            .unwrap();
        // Property animations give each surface real frame demand while
        // they run; the peer's long curve outlives the short one, so an
        // idle frame can be observed on `surface` alone. The `Layer`
        // handles stay bound for the animation's life: a dropped layer
        // removes itself from the tree.
        let layer = surface.layer();
        surface.update_animated(
            crate::Curve::linear(std::time::Duration::from_secs(60)),
            |tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].opacity(0.5f32);
            },
        );
        let peer_layer = peer.layer();
        peer.update_animated(
            crate::Curve::linear(std::time::Duration::from_secs(3600)),
            |tx| {
                tx[peer.root()].push(&peer_layer);
                tx[&peer_layer].opacity(0.5f32);
            },
        );

        let _ = engine.render(FrameTime::now()).unwrap();
        assert!(matches!(surface.next_frame(), Next::At { .. }));
        assert!(matches!(peer.next_frame(), Next::At { .. }));

        // Hidden: the surface reads Idle — never the peer's deadline.
        surface.visibility(crate::Visibility::Hidden).unwrap();
        let _ = engine.render(FrameTime::now()).unwrap();
        assert_eq!(surface.next_frame(), Next::Idle);
        assert!(matches!(peer.next_frame(), Next::At { .. }));

        // Revealed again: its own demand republishes a live deadline.
        surface.visibility(crate::Visibility::Visible).unwrap();
        let _ = engine.render(FrameTime::now()).unwrap();
        assert!(matches!(surface.next_frame(), Next::At { .. }));
        assert!(matches!(peer.next_frame(), Next::At { .. }));

        // Visible with its animation finished: a real idle frame
        // overwrites the stale `At`; the still-running peer is
        // unaffected.
        let _ = engine
            .render(FrameTime::at(
                crate::Instant::now() + std::time::Duration::from_secs(120),
            ))
            .unwrap();
        assert_eq!(surface.next_frame(), Next::Idle);
        assert!(matches!(peer.next_frame(), Next::At { .. }));

        // Re-arming the still-owned layer with a different animation
        // republishes its deadline: removing it next genuinely clears a
        // live `At`, not an already-idle slot.
        surface.update_animated(
            crate::Curve::linear(std::time::Duration::from_secs(600)),
            |tx| {
                tx[&layer].opacity(0.25f32);
            },
        );
        let _ = engine.render(FrameTime::now()).unwrap();
        assert!(matches!(surface.next_frame(), Next::At { .. }));
        assert!(matches!(peer.next_frame(), Next::At { .. }));

        // Removed: the layer's own drop sends the remove op, so its
        // animation and deadline go with it — only the peer's stays.
        drop(layer);
        let _ = engine.render(FrameTime::now()).unwrap();
        assert_eq!(surface.next_frame(), Next::Idle);
        assert!(matches!(peer.next_frame(), Next::At { .. }));

        // The surface itself going away likewise leaves the peer's
        // deadline untouched.
        drop(surface);
        let _ = engine.render(FrameTime::now()).unwrap();
        assert!(matches!(peer.next_frame(), Next::At { .. }));
    }

    /// A waker that signals through a channel: `recv` on the other end is
    /// the wake delivery the tests assert, not eventual readiness under
    /// busy polling.
    struct WakeSignal(std::sync::mpsc::Sender<()>);

    impl std::task::Wake for WakeSignal {
        fn wake(self: Arc<Self>) {
            let _ = self.0.send(());
        }
        fn wake_by_ref(self: &Arc<Self>) {
            let _ = self.0.send(());
        }
    }

    fn wake_signal() -> (std::task::Waker, std::sync::mpsc::Receiver<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (std::task::Waker::from(Arc::new(WakeSignal(tx))), rx)
    }

    /// Polls the future once with the real waker, reports that first
    /// poll's state to the controlling test, then re-polls the same waker
    /// only when a real notification arrives — wake delivery is proven
    /// across the whole pending lifetime, not after a fresh waker
    /// registers.
    fn drive_gated<Fut: std::future::Future>(
        future: Fut,
        report: &std::sync::mpsc::Sender<bool>,
    ) -> Fut::Output {
        let (waker, signals) = wake_signal();
        let mut cx = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        let start = std::time::Instant::now();
        let first = std::future::Future::poll(future.as_mut(), &mut cx);
        report
            .send(first.is_pending())
            .expect("controller awaits the first poll");
        if let std::task::Poll::Ready(value) = first {
            return value;
        }
        loop {
            signals
                .recv_timeout(std::time::Duration::from_secs(30).saturating_sub(start.elapsed()))
                .expect("pending future was not woken");
            if let std::task::Poll::Ready(value) =
                std::future::Future::poll(future.as_mut(), &mut cx)
            {
                return value;
            }
        }
    }

    /// Polls once with the noop waker — only for a future cancelled or
    /// dropped immediately after this poll, never to claim a wake.
    fn poll_once<Fut: std::future::Future>(fut: std::pin::Pin<&mut Fut>) -> bool {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        matches!(
            std::future::Future::poll(fut, &mut cx),
            std::task::Poll::Pending
        )
    }

    fn null_engine(events: std::sync::mpsc::Sender<Event>) -> Engine<Null> {
        Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
            image_limits: crate::ImageLimits::UNLIMITED,
        })
        .unwrap()
    }

    fn offscreen(engine: &Engine<Null>) -> crate::surface::Surface<Null> {
        engine
            .surface(
                crate::Offscreen::new((16, 16), crate::OffscreenFormat::LinearF16),
                || {},
            )
            .unwrap()
    }

    /// A render future polled to `Pending` while the render thread sits
    /// inside `gate_rx`: `report_tx` carries the pending observation to
    /// the controlling test, which alone holds `gate_tx` and releases it.
    #[test]
    fn async_render_yields_while_worker_is_paused() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let (events, _rx) = std::sync::mpsc::channel();
            let engine = null_engine(events);
            let surface = offscreen(&engine);
            surface.update(|tx| {
                tx[surface.root()].opacity(0.5f32);
            });
            engine
                .tx
                .send(Message::Resource(Box::new(move |_| {
                    let _ = gate_rx.recv();
                })))
                .unwrap();
            drive_gated(engine.render_async(FrameTime::now()), &report_tx)
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the worker waits on the gate");
        let result = helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported its poll"),
            "render_async must yield while the worker is parked"
        );
        result.expect("the released render completed");
    }

    #[test]
    fn async_render_cancellation_reuses_producer() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let (events, event_rx) = std::sync::mpsc::channel();
            let engine = null_engine(events);
            let surface = offscreen(&engine);
            let producer = engine.gpu_producer(());
            let layer = surface.layer();
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(producer.at((8, 8)));
            });
            engine
                .tx
                .send(Message::Resource(Box::new(move |_| {
                    let _ = gate_rx.recv();
                })))
                .unwrap();
            let mut first = Box::pin(engine.render_async(FrameTime::now()));
            assert!(
                poll_once(first.as_mut()),
                "the first render must queue behind the parked worker"
            );
            drop(first);
            // The second render reports its first pending state, then
            // completes only on a real wake: its drain collects the
            // cancelled render's reply, recycles the buffers and submits
            // its own — never applying the abandoned commit.
            drive_gated(engine.render_async(FrameTime::now()), &report_tx).unwrap();
            // Assertions run while engine, surface, layer and producer
            // are all alive — the cancelled frame's reply was drained and
            // recycled, never applied.
            let id = producer.id();
            let mut added = 0;
            let mut bound = 0;
            let mut retired = 0;
            while let Ok(event) = event_rx.try_recv() {
                match event {
                    Event::AddProducer(registered) if registered == id => added += 1,
                    Event::BindProducer(_, _, binding) if binding == id => bound += 1,
                    Event::RetireProducer(gone) if gone == id => retired += 1,
                    _ => {}
                }
            }
            assert_eq!(added, 1, "producer {id:?} registered once");
            assert_eq!(bound, 1, "producer {id:?} bound once");
            assert_eq!(retired, 0, "producer {id:?} must not retire while it lives");
            drop((surface, layer, producer));
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the worker waits on the gate");
        helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported the second render's poll"),
            "the second render must yield while the worker is parked"
        );
    }

    #[test]
    fn async_surface_cancellation_destroys_registration() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let (events, event_rx) = std::sync::mpsc::channel();
            let engine = null_engine(events);
            engine
                .tx
                .send(Message::Resource(Box::new(move |_| {
                    let _ = gate_rx.recv();
                })))
                .unwrap();
            let mut surface = Box::pin(engine.surface_async(
                crate::Offscreen::new((16, 16), crate::OffscreenFormat::LinearF16),
                || {},
            ));
            assert!(
                poll_once(surface.as_mut()),
                "surface_async must yield while the worker is parked"
            );
            drop(surface);
            report_tx
                .send(())
                .expect("the test awaits the cancellation");
            // The engine no longer joins the worker on Drop: the create
            // and destroy events are awaited for real.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let (mut created, mut destroyed) = (None, None);
            while std::time::Instant::now() < deadline && destroyed.is_none() {
                match event_rx.recv_timeout(std::time::Duration::from_secs(1)) {
                    Ok(Event::CreateSurface(id)) => created = Some(id),
                    Ok(Event::DestroySurface(id)) => destroyed = Some(id),
                    _ => {}
                }
            }
            (created, destroyed, engine.live_surfaces())
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        let released = gate_tx.send(());
        let completed = helper.join();
        observed.expect("helper never reported the cancellation");
        released.expect("the worker waits on the gate");
        let (created, destroyed, live) = completed.expect("helper panicked");
        assert_eq!(
            created.expect("CreateSurface never ran"),
            destroyed.expect("DestroySurface never ran"),
            "the cancelled registration destroyed the same id it created"
        );
        assert_eq!(live, 0, "no live surface after a cancelled registration");
    }

    /// A render queued behind a gated `Resource` whose release panics:
    /// the queued message dies with the worker, its request guard closes
    /// the channel, and the waiting future resolves `Thread` — the
    /// frontend's own sender cannot close it.
    #[test]
    fn async_queued_render_is_woken_when_the_worker_panics() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let (events, _rx) = std::sync::mpsc::channel();
            let engine = null_engine(events);
            let surface = offscreen(&engine);
            surface.update(|tx| {
                tx[surface.root()].opacity(0.5f32);
            });
            engine
                .tx
                .send(Message::Resource(Box::new(move |_| {
                    let _ = gate_rx.recv();
                    panic!("render thread failure");
                })))
                .unwrap();
            drive_gated(engine.render_async(FrameTime::now()), &report_tx)
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the worker waits on the gate");
        let result = helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported its poll"),
            "render_async must yield while the worker is parked"
        );
        match result {
            Err(RenderError::Thread) => {}
            other => panic!("render_async resolved {other:?}, not RenderError::Thread"),
        }
    }

    /// A request the worker has already dequeued and is holding when it
    /// unwinds: the accepted request's guard closes the channel during
    /// the panic — the hole an exit-time armed sender misses.
    #[test]
    fn async_accepted_render_is_woken_when_the_worker_unwinds() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let (events, _rx) = std::sync::mpsc::channel();
            let mut engine = null_engine(events);
            let surface = offscreen(&engine);
            surface.update(|tx| {
                tx[surface.root()].opacity(0.5f32);
            });
            // Replace the render transport: the fake worker receives the
            // real Render message, holds its request armed on the gate,
            // then unwinds — the guard's drop closes the channel.
            engine.tx.send(Message::Shutdown).unwrap();
            engine.thread.take().unwrap().join().unwrap();
            let (tx, rx) = crossbeam_channel::unbounded();
            engine.tx = tx;
            engine.thread = Some(std::thread::spawn(move || {
                let Message::Render { reply, .. } = rx.recv().expect("expected render") else {
                    panic!("expected a Render");
                };
                let _armed = reply;
                let _ = gate_rx.recv();
                panic!("the worker unwound while it owed a reply");
            }));
            drive_gated(engine.render_async(FrameTime::now()), &report_tx)
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the fake worker waits on the gate");
        let result = helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported its poll"),
            "render_async must yield while the reply is owed"
        );
        match result {
            Err(RenderError::Thread) => {}
            other => panic!("render_async resolved {other:?}, not RenderError::Thread"),
        }
    }

    /// A `Backend` wrapper whose `init` blocks on a release channel —
    /// the gate `new_async`'s cancellation test holds the render
    /// thread's init behind.
    struct InitGate;

    struct InitGateConfig {
        inner: NullConfig,
        release: std::sync::mpsc::Receiver<()>,
    }

    impl cherenkov_record::Target for InitGate {
        type Queue = crate::EngineQueue<Self>;
        type Install = crate::InstallOp<Self>;
    }

    impl crate::backend::Backend for InitGate {
        type Config = InitGateConfig;
        type Info = crate::testing::NullInfo;
        type Target = crate::testing::NullTarget;
        type Renderer = crate::testing::NullRenderer;

        #[cfg(not(target_arch = "wasm32"))]
        fn init(
            config: InitGateConfig,
        ) -> Result<(crate::testing::NullRenderer, crate::testing::NullInfo), EngineError> {
            config
                .release
                .recv()
                .map_err(|_| EngineError::Thread("init gate dropped".into()))?;
            <Null as crate::backend::Backend>::init(config.inner)
        }
    }

    #[test]
    fn async_new_cancellation_posts_shutdown() {
        let (events, event_rx) = std::sync::mpsc::channel();
        // The test thread owns the init gate's release before the helper
        // polls — a hung poll can always be freed.
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let mut engine = Box::pin(Engine::<InitGate>::new_async(InitGateConfig {
                inner: NullConfig {
                    events,
                    reject: std::collections::HashSet::default(),
                    image_limits: crate::ImageLimits::UNLIMITED,
                },
                release: release_rx,
            }));
            assert!(
                poll_once(engine.as_mut()),
                "new_async must yield while init is gated"
            );
            drop(engine);
            report_tx
                .send(())
                .expect("the test awaits the cancellation");
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        let released = release_tx.send(());
        let completed = helper.join();
        observed.expect("helper never cancelled the init");
        released.expect("the worker's init waits on the gate");
        completed.expect("helper panicked");
        // Init completes, the queued Shutdown unwinds the worker and the
        // renderer's event sender disconnects — ordered shutdown.
        match event_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
            other => panic!("the renderer's event sender did not disconnect: {other:?}"),
        }
    }
}
