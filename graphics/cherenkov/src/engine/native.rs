//! The engine: owns the render thread and every resource's identity.
//! `!Send`, lives on the UI thread.

use super::{SurfaceWaker, Waker, thread};

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};

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
    ChangeSet, FontData, MemoryReply, Message, ProducerId, RegisterOp, RenderReply, ResOp,
    SurfaceId,
};
use crate::paint::ImageId;
use crate::resource::{
    Filter, Font, FontSource, FrameSink, GpuProducer, Image, ReplaceImage, ResourceId, Shader,
};
use crate::style::FilterId;
use crate::surface::{Shared, Surface};

/// The engine: owns the device and the render thread. `!Send`, lives on
/// the UI thread.
///
/// The render thread owns every backend object. `Engine` is deliberately
/// `!Send` — everything it hands out (`Surface`, `Layer`, resource
/// handles) may only live on the UI thread that created the engine.
///
/// The native UI-to-render message queue is bounded to 64 messages. If the
/// UI thread gets more than 64 messages ahead, it waits for the render thread.
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<cherenkov::Engine<cherenkov::testing::Null>>();
/// ```
pub struct Engine<B: Backend> {
    tx: SyncSender<Message<B>>,
    /// A producer's last `Arc` drop posts its retirement here — an
    /// unbounded queue the render loop drains after each applied
    /// message, so a drop on the render thread itself never blocks on
    /// the bounded transaction channel.
    retire: std::sync::mpsc::Sender<ResOp<B>>,
    info: B::Info,
    stats: RefCell<FrameStats>,
    render_reply: RefCell<Option<SyncSender<RenderReply<B>>>>,
    render_reply_rx: Receiver<RenderReply<B>>,
    memory_reply: RefCell<Option<SyncSender<MemoryReply>>>,
    memory_reply_rx: Receiver<MemoryReply>,
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
    thread: Option<std::thread::JoinHandle<()>>,
    /// The type-erased sender resource drops use; a render thread that is
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

impl<B: Backend> Engine<B> {
    /// Spawns the render thread and runs `B::init` on it, blocking until it
    /// reports success or failure.
    ///
    /// # Errors
    /// [`EngineError`] when the backend fails to initialize or the render
    /// thread cannot start.
    pub fn new(config: B::Config) -> Result<Self, EngineError> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Message<B>>(64);
        let (retire_tx, retire_rx) = std::sync::mpsc::channel::<ResOp<B>>();
        let (init_tx, init_rx) = std::sync::mpsc::channel();
        let (render_reply, render_reply_rx) = std::sync::mpsc::sync_channel(1);
        let (memory_reply, memory_reply_rx) = std::sync::mpsc::sync_channel(1);
        let render_thread = std::thread::Builder::new()
            .name("cherenkov-render".into())
            .spawn(move || thread::run::<B>(config, &rx, &retire_rx, &init_tx))
            .map_err(|e| EngineError::Thread(format!("spawn failed: {e}")))?;
        let info = init_rx
            .recv()
            .map_err(|_| EngineError::Thread("render thread died during init".into()))??;
        let post_tx = tx.clone();
        let waker = Arc::new(Waker::new());
        let replace_image = {
            let tx = tx.clone();
            // The render loop wakes the host through every visible surface
            // that draws the image, once it knows which surfaces do.
            Rc::new(move |id, image| {
                tx.send(Message::ReplaceImage { id, image })
                    .map_err(|_| ResourceError::Lost)
            }) as ReplaceImage
        };
        Ok(Self {
            tx,
            retire: retire_tx,
            info,
            stats: RefCell::new(FrameStats::default()),
            render_reply: RefCell::new(Some(render_reply)),
            render_reply_rx,
            memory_reply: RefCell::new(Some(memory_reply)),
            memory_reply_rx,
            commits: RefCell::new(Vec::new()),
            surfaces: RefCell::new(Vec::new()),
            next_surface: Cell::new(0),
            next_font: Cell::new(1),
            next_image: Cell::new(1),
            next_shader: Cell::new(1),
            next_filter: Cell::new(1),
            next_backdrop_shader: Cell::new(1),
            next_producer: Cell::new(1),
            thread: Some(render_thread),
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
        let Some(reply_sender) = self.memory_reply.borrow_mut().take() else {
            return MemoryUsage::default();
        };
        if let Err(error) = self.tx.send(Message::Memory {
            reply: reply_sender,
        }) {
            if let Message::Memory { reply } = error.0 {
                *self.memory_reply.borrow_mut() = Some(reply);
            }
            return MemoryUsage::default();
        }
        let Ok(reply) = self.memory_reply_rx.recv() else {
            return MemoryUsage::default();
        };
        *self.memory_reply.borrow_mut() = Some(reply.sender);
        reply.usage
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
    /// [`Surface::visibility`]). A hidden surface never calls it. `f` may
    /// run on the engine's thread, on the render thread — an image
    /// replacement wakes the host from there once it knows a visible
    /// surface draws the image — or on the main thread, where a
    /// completion the render thread queued fires it, so it must be
    /// [`Send`] and [`Sync`].
    ///
    /// # Panics
    /// When the callback slot is poisoned by a panic inside a previous
    /// `f` running under the lock.
    pub fn set_waker(&self, f: impl Fn() + Send + Sync + 'static) {
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
    /// `image` is validated by [`ImageData::new`] before it is passed here.
    /// The upload is queued in order with every render and does not wait
    /// for the backend. A rejection only the backend can detect (a device
    /// or budget limit) fails every render that draws the image with
    /// [`RenderError::Rejected`].
    ///
    /// # Errors
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn image<F: Format>(&self, image: ImageData<F>) -> Result<Image<F>, ResourceError>
    where
        B: Uploads<F>,
    {
        let id = ImageId::new(Self::alloc(&self.next_image));
        let upload = image.into_upload();
        let resource = ResourceId::Image(id);
        self.register(
            resource,
            Box::new(move |r: &mut B::Renderer| r.add_image(id, upload)),
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
    pub fn surface(&self, target: impl Into<B::Target>) -> Result<Surface<B>, SurfaceError> {
        let id = SurfaceId::new(Self::alloc(&self.next_surface));
        let waker = Arc::new(SurfaceWaker::new(Arc::clone(&self.waker)));
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Message::CreateSurface {
                id,
                target: target.into(),
                waker: Arc::clone(&waker),
                reply,
            })
            .map_err(|_| SurfaceError::Lost)?;
        let info = rx.recv().map_err(|_| SurfaceError::Lost)??;
        let surface = Surface::new(id, info, self.tx.clone(), waker);
        self.surfaces
            .borrow_mut()
            .push(Rc::downgrade(&surface.shared));
        Ok(surface)
    }

    /// The number of surfaces still alive, for leak testing.
    #[doc(hidden)]
    pub fn live_surfaces(&self) -> usize {
        let mut surfaces = self.surfaces.borrow_mut();
        surfaces.retain(|weak| weak.strong_count() > 0);
        surfaces.len()
    }

    /// Renders every dirty visible surface for the frame at `time`,
    /// blocking until the render thread has applied the queued commits,
    /// sampled the animations and rendered. A hidden surface is neither
    /// sampled nor drawn; its changes were applied as it made them.
    ///
    /// # Errors
    /// [`RenderError::Hidden`] when every surface is hidden, before
    /// anything is drained. Any other [`RenderError`] fails this call; a
    /// surface that failed to render is left in its previous state.
    pub fn render(&self, time: FrameTime) -> Result<Next, RenderError> {
        if super::all_hidden(&self.surfaces.borrow()) {
            return Err(RenderError::Hidden);
        }
        let Some(reply_sender) = self.render_reply.borrow_mut().take() else {
            return Err(RenderError::Thread);
        };
        let mut commits = std::mem::take(&mut *self.commits.borrow_mut());
        commits.clear();
        super::drain_visible(&mut self.surfaces.borrow_mut(), time, &mut commits);
        // Completions may arrive while render is in flight, before its reply.
        self.waker.arm();
        if let Err(error) = self.tx.send(Message::Render {
            time,
            commits,
            reply: reply_sender,
        }) {
            if let Message::Render { commits, reply, .. } = error.0 {
                *self.commits.borrow_mut() = commits;
                *self.render_reply.borrow_mut() = Some(reply);
            }
            return Err(RenderError::Thread);
        }
        let mut reply = self
            .render_reply_rx
            .recv()
            .map_err(|_| RenderError::Thread)?;
        *self.render_reply.borrow_mut() = Some(reply.sender);
        self.recycle_commits(&mut reply.commits);
        reply.commits.clear();
        *self.commits.borrow_mut() = reply.commits;
        let (next, stats) = reply.result?;
        *self.stats.borrow_mut() = stats;
        Ok(next)
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
    /// cannot create fails every render that draws the shader with
    /// [`RenderError::Rejected`].
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
    /// backend cannot create fails every render that samples the shader
    /// with [`RenderError::Rejected`].
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
        let dirty = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gate = std::sync::Arc::new(crate::WakeGate::default());
        (self.post)(Message::Resource(Box::new({
            let dirty = std::sync::Arc::clone(&dirty);
            let gate = std::sync::Arc::clone(&gate);
            move |r: &mut B::Renderer| {
                B::add_frame_producer(r, id, dirty, gate);
            }
        })));
        let engine_waker = std::sync::Arc::clone(&self.waker);
        (
            GpuProducer::new(id, self.retire.clone()),
            FrameSink::new(
                id,
                self.tx.clone(),
                dirty,
                gate,
                std::sync::Arc::new(move || engine_waker.wake()),
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

impl<B: Backend> Drop for Engine<B> {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Event, Null, NullConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A binding's last reference can die on the render thread — the
    /// unbind a `clear_content` runs drops the `GpuProducer` clone the
    /// binding held. With the bounded transaction channel saturated the
    /// retirement must still land and the loop must not hang: it rides
    /// the producer's own unbounded queue, never `tx`.
    #[test]
    fn a_render_thread_binding_drop_retires_without_hanging() {
        let (events, rx) = std::sync::mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
        })
        .unwrap();
        let surface = engine
            .surface(crate::Offscreen::new(
                (16, 16),
                crate::OffscreenFormat::LinearF16,
            ))
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

        // Park the render thread, queue the unbind behind it, then pin
        // the bounded channel full so a blocking retire send from
        // inside the render thread could never complete.
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
        let stop = Arc::new(AtomicUsize::new(0));
        let filler = std::thread::spawn({
            let tx = engine.tx.clone();
            let stop = Arc::clone(&stop);
            move || {
                while stop.load(Ordering::Relaxed) == 0
                    && tx.send(Message::Resource(Box::new(|_| {}))).is_ok()
                {}
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(parked_tx);

        // The render thread drains: the Apply unbinds and drops the
        // last clone — the retirement goes through the producer's
        // unbounded queue even with `tx` pinned full. `RetireProducer`
        // landing proves the loop never wedged on a blocking send.
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
        // saturation still applies.
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
        stop.store(1, Ordering::Relaxed);
        filler.join().unwrap();
    }

    #[test]
    fn completion_before_render_reply_wakes_the_host() {
        let (events, _) = std::sync::mpsc::channel();
        let mut engine = Engine::<Null>::new(NullConfig {
            events,
            reject: std::collections::HashSet::default(),
        })
        .unwrap();
        // Replace only the render transport. The real native render method
        // receives a completion after submission but before its reply.
        engine.tx.send(Message::Shutdown).unwrap();
        engine.thread.take().unwrap().join().unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        engine.tx = tx;
        let wake_count = Arc::new(AtomicUsize::new(0));
        engine.set_waker({
            let wake_count = Arc::clone(&wake_count);
            move || {
                wake_count.fetch_add(1, Ordering::Relaxed);
            }
        });
        engine.waker.wake();
        let waker = Arc::clone(&engine.waker);
        engine.thread = Some(std::thread::spawn(move || {
            let Message::Render { commits, reply, .. } = rx.recv().unwrap() else {
                panic!("expected render");
            };
            waker.wake();
            reply
                .send(RenderReply {
                    result: Ok((Next::Idle, FrameStats::default())),
                    commits,
                    sender: reply.clone(),
                })
                .unwrap();
            assert!(matches!(rx.recv().unwrap(), Message::Shutdown));
        }));
        assert_eq!(engine.render(FrameTime::now()).unwrap(), Next::Idle);
        assert_eq!(wake_count.load(Ordering::Relaxed), 2);
    }
}
