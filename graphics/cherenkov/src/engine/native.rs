//! The engine: owns the render thread and every resource's identity.
//! `!Send`, lives on the UI thread.

use super::pending::{Frontend, PendingFrame};
use super::{SurfaceWaker, thread};

use crate::transport::ReplyRx;
use futures_lite::future::block_on;
use std::cell::Cell;
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
use cherenkov_record::SurfaceId;

use crate::message::{FontData, Message, ProducerId, RegisterOp, ResOp};
use crate::paint::ImageId;
use cherenkov_record::ResourceId;

use crate::resource::{
    Filter, Font, FontSource, FrameSink, GpuProducer, Image, ImageBudget, ReplaceImage, Shader,
    TextLayoutHandle,
};
use crate::style::FilterId;
use crate::surface::Surface;
use crate::text::{TextLayout, TextLayoutData};
use cherenkov_record::TextLayoutId;

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
    /// The memory query's persistent reply channel, reused across calls:
    /// the worker answers through the same request guard a frame's reply
    /// uses, so a dropped request closes it and a successful one leaves
    /// it reusable.
    memory_reply: crate::transport::Reply<crate::message::MemoryReply>,
    memory_reply_rx: crate::transport::ReplyRx<crate::message::MemoryReply>,
    /// The frame state every [`PendingFrame`] shares: the live surfaces'
    /// queues, the last frame's stats and the one frame slot.
    frontend: Rc<Frontend<B>>,
    next_surface: Cell<u64>,
    next_font: Cell<u64>,
    next_text: Cell<u64>,
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
    /// Spawns the render thread and runs `B::init` on it, waiting on the
    /// calling thread for [`Engine::new_async`] to resolve — the
    /// constructor of hosts whose setup is synchronous (Hydrolysis's
    /// frame pump). A host on a UI thread that must not wait uses
    /// [`Engine::new_async`].
    ///
    /// # Errors
    /// [`EngineError`] when the backend fails to initialize or the render
    /// thread cannot start.
    pub fn new(config: B::Config) -> Result<Self, EngineError> {
        block_on(Self::new_async(config))
    }

    /// Spawns the render thread and runs `B::init` on it without making
    /// the caller wait: the render thread's init reply resolves the
    /// future, and a cancellation before it arrives posts `Shutdown` to
    /// the nascent worker without joining it on the caller's thread.
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

    /// Spawns the render thread and returns the pieces `new_async`'s wait
    /// then turns into an `Engine`.
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
    /// capabilities.
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
            frontend: Rc::new(Frontend::new(tx.clone())),
            tx,
            retire: retire_tx,
            info,
            images: ImageBudget::new(image_limits),
            memory_reply,
            memory_reply_rx,
            next_surface: Cell::new(0),
            next_font: Cell::new(1),
            next_text: Cell::new(1),
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

    /// Shuts the render thread down and joins it. Test-only: dropping an
    /// engine detaches the thread (#3753), so a surface only starts
    /// answering [`SurfaceError::Lost`] some time after the drop; once this
    /// returns the thread has ended, its channel receiver is gone, and
    /// every surface of this engine reports `Lost` deterministically.
    ///
    /// # Panics
    /// When the render thread panicked.
    #[cfg(feature = "testing")]
    pub fn shutdown_and_join_for_testing(mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the render thread ends cleanly");
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

    /// Statistics of the last frame that resolved. GPU timings are kept by
    /// the render thread and returned by [`Engine::finish_timings`] instead.
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.frontend.stats.borrow().clone()
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

    /// The engine's current memory usage, waiting on the calling thread for
    /// the render thread's answer. It belongs to tooling and to hosts
    /// whose frame loop is synchronous, never to a UI thread that must not
    /// wait.
    ///
    /// # Errors
    /// [`RenderError::Thread`] when the render thread cannot answer — a
    /// request it dropped closes the reply channel and marks it gone — so
    /// an unmeasurable engine is an error, never a fabricated zero usage.
    pub fn memory(&self) -> Result<MemoryUsage, RenderError> {
        self.tx
            .send(Message::Memory {
                reply: self.memory_reply.request(),
            })
            .map_err(|_| RenderError::Thread)?;
        let reply = block_on(self.memory_reply_rx.recv()).map_err(|_| RenderError::Thread)?;
        Ok(reply.usage)
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

    /// Registers a laid-out text. The layout's fonts, registered when it
    /// was lowered, stay registered as long as the handle lives.
    ///
    /// # Errors
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn text_layout(&self, layout: TextLayout) -> Result<TextLayoutHandle, ResourceError> {
        let (commands, default, fonts) = layout.into_parts();
        let data = TextLayoutData::new(commands, default, &fonts);
        let id = TextLayoutId::new(Self::alloc(&self.next_text));
        self.tx
            .send(Message::Resource(Box::new(move |r: &mut B::Renderer| {
                r.add_text_layout(id, data);
            })))
            .map_err(|_| ResourceError::Lost)?;
        Ok(TextLayoutHandle::new(
            id,
            fonts,
            self.on_release(ResourceId::TextLayout(id), move |r| {
                r.remove_text_layout(id);
            }),
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
    /// needed: the engine calls `wake` at most once between two frames
    /// ([`Engine::submit`]) the surface participates in — the first time
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
    /// holds the scope across the frame's submission, so the edits it
    /// makes for that frame do not ask for another.
    ///
    /// This waits on the calling thread for [`Engine::surface_async`] to
    /// resolve — the constructor of hosts whose frame loop is synchronous.
    /// A host on a UI thread that must not wait uses
    /// [`Engine::surface_async`].
    ///
    /// # Errors
    /// [`SurfaceError`] when the backend cannot draw the target, or
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn surface(
        &self,
        target: impl Into<B::Target>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Surface<B>, SurfaceError> {
        block_on(self.surface_async(target, wake))
    }

    /// Creates a surface like [`Engine::surface`] without making the
    /// caller wait: the worker's `CreateSurface` reply resolves the
    /// future, and a cancellation before or after the worker handles it
    /// posts `DestroySurface` for the same id — nothing is left
    /// registered.
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
        self.frontend.surfaces.borrow_mut().push(super::SurfaceEntry {
            shared: Rc::downgrade(&surface.shared),
            waker: Arc::clone(&surface.waker),
            next_frame: Rc::downgrade(&surface.next_frame),
            running_tracks: Rc::downgrade(&surface.running_tracks),
        });
        surface
    }

    /// Sets or clears the band of host window `window` on `surface`: the
    /// content painted above the mark naming that window
    /// ([`LayerEdit::host_window`](crate::LayerEdit::host_window)) and
    /// beneath the next mark presents into `target`, which the host stacks
    /// right above the window (water-rs/waterui#2225). `None` leaves the
    /// region with no band, and its content is not shown. A renderer
    /// without a band realization reports a contract violation on the render
    /// thread and drops the band
    /// ([`Renderer::set_band`](crate::Renderer::set_band)).
    ///
    /// # Errors
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn set_band(
        &self,
        surface: &Surface<B>,
        window: u64,
        target: Option<impl Into<B::Target>>,
    ) -> Result<(), SurfaceError> {
        self.tx
            .send(Message::SetBand {
                id: surface.id(),
                window,
                target: target.map(Into::into),
            })
            .map_err(|_| SurfaceError::Lost)
    }

    /// The number of surfaces still alive, for leak testing.
    #[doc(hidden)]
    pub fn live_surfaces(&self) -> usize {
        let mut surfaces = self.frontend.surfaces.borrow_mut();
        surfaces.retain(|entry| entry.shared.strong_count() > 0);
        surfaces.len()
    }

    /// Submits the frame at `time` and returns at once, before the render
    /// thread has done anything with it: drains every visible surface's
    /// queued changes into one `Render` message and hands it to the render
    /// thread, which applies the commits, samples the animations at `time`
    /// and renders every dirty visible surface. A hidden surface is neither
    /// sampled nor drawn; its changes were applied as it made them.
    ///
    /// The returned [`PendingFrame`] resolves on the thread that polls it,
    /// woken by the render thread's reply. Dropping it discards the
    /// frame's presentation without waiting. While an earlier frame is
    /// still in flight, the new one is queued and posts its render once
    /// that one resolves or is dropped.
    ///
    /// # Errors
    /// [`RenderError::Hidden`] when every surface is hidden, before
    /// anything is drained; [`RenderError::Thread`] when the render thread
    /// is gone. The frame itself resolves to the [`Next`] the render
    /// produced, [`RenderError::Rejected`] when the content of some
    /// surfaces draws a resource the backend rejected — those surfaces
    /// were left out of the frame, and every other surface rendered — or
    /// the [`RenderError`] that failed the frame; a surface that failed to
    /// render is left in its previous state.
    pub fn submit(&self, time: FrameTime) -> Result<PendingFrame<B>, RenderError> {
        self.frontend.submit(time)
    }

    /// Renders the frame at `time`, waiting on the calling thread for the
    /// frame [`Engine::submit`] hands out to resolve — the frame entry
    /// point of hosts whose frame loop is synchronous (Hydrolysis's pump).
    /// A host on a UI thread that must not wait submits and composites
    /// once the frame resolves.
    ///
    /// # Errors
    /// [`RenderError`] under the same conditions as [`Engine::submit`] and
    /// its frame.
    pub fn render(&self, time: FrameTime) -> Result<Next, RenderError> {
        block_on(self.submit(time)?)
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
        let id = FilterId::new(Self::alloc(&self.next_filter))
            .with_coordinate_remap(F::COORDINATE_REMAP);
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

    /// A second scope on a surface is inert: dropping it leaves the first
    /// scope's frame open.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "a surface has at most one open FrameScope")
    )]
    fn a_second_frame_scope_is_inert() {
        let (_engine, surface, wakes) = counted_surface();
        let first = surface.begin_frame();
        let second = surface.begin_frame();
        edit(&surface, 0.5);
        drop(second);
        edit(&surface, 0.25);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            0,
            "the first scope still holds the frame"
        );
        drop(first);
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "the first scope's end woke"
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

    /// Points the engine and its frame state at a fake worker's channel,
    /// keeping the registered surfaces.
    fn replace_transport(engine: &mut Engine<Null>, tx: crossbeam_channel::Sender<Message<Null>>) {
        let surfaces = engine.frontend.surfaces.take();
        engine.frontend = Rc::new(Frontend::new(tx.clone()));
        *engine.frontend.surfaces.borrow_mut() = surfaces;
        engine.tx = tx;
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
        replace_transport(&mut engine, tx);
        surface.waker.wake();
        let waker = Arc::clone(&surface.waker);
        engine.thread = Some(std::thread::spawn(move || {
            let Message::Render { commits, reply, .. } = rx.recv().unwrap() else {
                panic!("expected render");
            };
            waker.wake();
            reply.send(crate::message::RenderReply {
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
            .render(FrameTime::unpresented_at(
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

    /// Parks the render thread inside a `Resource` op until `gate` fires.
    fn park(engine: &Engine<Null>, gate: std::sync::mpsc::Receiver<()>) {
        engine
            .tx
            .send(Message::Resource(Box::new(move |_| {
                let _ = gate.recv();
            })))
            .unwrap();
    }

    /// `submit` returns to its caller while the render thread sits inside
    /// `gate_rx`, and the frame it hands out polls `Pending` until the
    /// reply wakes it: `report_tx` carries the pending observation — made
    /// after `submit` returned — to the controlling test, which alone holds
    /// `gate_tx` and releases the worker only once it has the report.
    #[test]
    fn submission_returns_while_the_render_worker_is_paused() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (report_tx, report_rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let (events, _rx) = std::sync::mpsc::channel();
            let engine = null_engine(events);
            let surface = offscreen(&engine);
            surface.update(|tx| {
                tx[surface.root()].opacity(0.5f32);
            });
            park(&engine, gate_rx);
            let frame = engine.submit(FrameTime::now()).unwrap();
            drive_gated(frame, &report_tx)
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the worker waits on the gate");
        let result = helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported its poll"),
            "the submitted frame must stay pending while the worker is parked"
        );
        result.expect("the released frame resolved");
    }

    /// The surface and its pending frame both go away while the render
    /// thread is parked: nothing waits on the worker, the abandoned reply
    /// is received and dropped by the next submission, and the engine keeps
    /// rendering a surface created afterwards.
    #[test]
    fn surface_teardown_while_a_frame_is_pending_is_safe() {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (events, event_rx) = std::sync::mpsc::channel();
        let engine = null_engine(events);
        let surface = offscreen(&engine);
        surface.update(|tx| {
            tx[surface.root()].opacity(0.5f32);
        });
        park(&engine, gate_rx);
        let frame = engine.submit(FrameTime::now()).unwrap();
        let retired = surface.id();
        drop(frame);
        drop(surface);
        gate_tx.send(()).expect("the worker waits on the gate");
        let replacement = offscreen(&engine);
        replacement.update(|tx| {
            tx[replacement.root()].opacity(0.25f32);
        });
        engine
            .render(FrameTime::now())
            .expect("the engine renders after the teardown");
        assert_eq!(engine.live_surfaces(), 1);
        let mut destroyed = false;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(event, Event::DestroySurface(id) if id == retired) {
                destroyed = true;
            }
        }
        assert!(destroyed, "the torn-down surface was destroyed");
    }

    /// A frame submitted while another is live is queued: it posts its
    /// render once the live one resolves, woken by that resolution.
    #[test]
    fn a_frame_submitted_behind_a_live_one_follows_it() {
        let (events, _rx) = std::sync::mpsc::channel();
        let engine = null_engine(events);
        let surface = offscreen(&engine);
        surface.update(|tx| {
            tx[surface.root()].opacity(0.5f32);
        });
        let first = engine.submit(FrameTime::now()).unwrap();
        let mut second = engine.submit(FrameTime::now()).unwrap();
        let (waker, woken) = wake_signal();
        assert!(
            std::future::Future::poll(
                std::pin::Pin::new(&mut second),
                &mut std::task::Context::from_waker(&waker)
            )
            .is_pending(),
            "the second frame waits for the first"
        );
        block_on(first).expect("the first frame resolves");
        woken
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the first frame's resolution wakes the queued one");
        block_on(second).expect("the queued frame resolves");
    }

    #[test]
    fn a_dropped_pending_frame_keeps_its_producer() {
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
            park(&engine, gate_rx);
            let mut first = engine.submit(FrameTime::now()).unwrap();
            assert!(
                poll_once(std::pin::Pin::new(&mut first)),
                "the first frame must queue behind the parked worker"
            );
            drop(first);
            // The second frame reports its first pending state, then
            // completes only on a real wake: it receives the dropped
            // frame's reply, recycles the buffers and posts its own —
            // never applying the abandoned commit.
            let second = engine.submit(FrameTime::now()).unwrap();
            drive_gated(second, &report_tx).unwrap();
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
    fn a_queued_render_is_woken_when_the_worker_panics() {
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
            drive_gated(engine.submit(FrameTime::now()).unwrap(), &report_tx)
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the worker waits on the gate");
        let result = helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported its poll"),
            "the frame must stay pending while the worker is parked"
        );
        match result {
            Err(RenderError::Thread) => {}
            other => panic!("the frame resolved {other:?}, not RenderError::Thread"),
        }
    }

    /// A request the worker has already dequeued and is holding when it
    /// unwinds: the accepted request's guard closes the channel during
    /// the panic — the hole an exit-time armed sender misses.
    #[test]
    fn an_accepted_render_is_woken_when_the_worker_unwinds() {
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
            replace_transport(&mut engine, tx);
            engine.thread = Some(std::thread::spawn(move || {
                let Message::Render { reply, .. } = rx.recv().expect("expected render") else {
                    panic!("expected a Render");
                };
                let _armed = reply;
                let _ = gate_rx.recv();
                panic!("the worker unwound while it owed a reply");
            }));
            drive_gated(engine.submit(FrameTime::now()).unwrap(), &report_tx)
        });
        let observed = report_rx.recv_timeout(std::time::Duration::from_secs(30));
        gate_tx.send(()).expect("the fake worker waits on the gate");
        let result = helper.join().expect("helper panicked");
        assert!(
            observed.expect("helper never reported its poll"),
            "the frame must stay pending while the reply is owed"
        );
        match result {
            Err(RenderError::Thread) => {}
            other => panic!("the frame resolved {other:?}, not RenderError::Thread"),
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

    /// A memory query distinguishes a failed measurement from a successful
    /// zero: the `Null` renderer owns nothing and answers an all-zero
    /// [`MemoryUsage`] through `Ok`, while an engine whose render thread is
    /// gone reports [`RenderError::Thread`] — never a fabricated zero —
    /// and the reply channel stays unusable after the loss.
    #[test]
    fn memory_distinguishes_failure_from_zero_usage() {
        let (events, _rx) = std::sync::mpsc::channel();
        let mut engine = null_engine(events);

        let usage = engine.memory().unwrap();
        assert_eq!(usage, MemoryUsage::default());

        engine.tx.send(Message::Shutdown).unwrap();
        engine.thread.take().unwrap().join().unwrap();

        assert!(matches!(engine.memory(), Err(RenderError::Thread)));
        assert!(matches!(engine.memory(), Err(RenderError::Thread)));
    }
}
