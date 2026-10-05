//! Surfaces: the UI-thread handle over the [`Shared`] queue and its
//! [`Layer`] handles — all of which live in `cherenkov-record` — plus the
//! engine's [`Queue`] endpoint for it.
//!
//! `Surface::update`, layer drops and bound-signal changes only queue
//! owned ops; [`Engine::render`](crate::Engine::render) drains every
//! surface's queue into one [`Message::Render`], so the render thread
//! wakes once per frame. A hidden surface's queue drains inline instead:
//! its changes leave as [`Message::Apply`] as they are made.
//!
//! [`Layer`]: crate::Layer

#[cfg(target_arch = "wasm32")]
use crate::local::Sender;
#[cfg(not(target_arch = "wasm32"))]
use crossbeam_channel::Sender;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use cherenkov_record::{ChangeSet, Layer, Queue, Shared, SurfaceId, Transaction};

use crate::WorkingColor;
use crate::animation::Animation;
use crate::backend::{Backend, Display, SurfaceInfo, Visibility};
use crate::capability::{Backdrop, BackdropChain, BackdropRuns};
use crate::engine::SurfaceWaker;
use crate::error::{RenderError, SurfaceError};
use crate::frame::Readback;
use crate::message::Message;
use crate::record::Content;

/// The engine's [`Queue`] endpoint for a [`Shared`].
///
/// It holds the surface's host waker and the render-thread channel. A
/// hidden surface drains inline — the drained [`ChangeSet`] leaves as
/// [`Message::Apply`] in order with the surface's other messages — a
/// visible one wakes the host for the frame's drain instead.
pub struct EngineQueue<B: Backend> {
    /// The surface's identifier on the render thread.
    id: SurfaceId,
    /// The surface's host wake-up.
    waker: Arc<SurfaceWaker>,
    /// The render loop, which a hidden surface's changes are sent to as
    /// they are made.
    tx: Sender<Message<B>>,
}

impl<B: Backend> std::fmt::Debug for EngineQueue<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineQueue")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> EngineQueue<B> {
    /// A queue for the surface `id`: `waker` is the surface's host wake-up
    /// and `tx` the render loop's channel.
    pub(crate) const fn new(
        id: SurfaceId,
        waker: Arc<SurfaceWaker>,
        tx: Sender<Message<B>>,
    ) -> Self {
        Self { id, waker, tx }
    }
}

impl<B: Backend> Queue<B> for EngineQueue<B> {
    fn drains_inline(&self) -> bool {
        self.waker.visibility() == Visibility::Hidden
    }

    fn apply(&self, changes: ChangeSet<B>) {
        // A lost render thread fails the host's next render; there is
        // nothing left to apply the change to.
        let _ = self.tx.send(Message::Apply {
            id: self.id,
            changes,
        });
    }

    fn wake(&self) {
        self.waker.wake();
    }
}

/// A surface: a render target plus its layer tree. `!Send`; dropping sends
/// [`Message::DestroySurface`].
pub struct Surface<B: Backend> {
    /// The shared pending-changes state, also registered with the engine
    /// for the per-frame drain.
    pub shared: Rc<RefCell<Shared<B>>>,
    id: SurfaceId,
    size: Cell<(u32, u32)>,
    readable: bool,
    max_dimension: u32,
    root: Layer,
    /// The next frame this surface asked for, published by each
    /// [`Engine::render`](crate::Engine::render).
    pub(crate) next_frame: Rc<RefCell<crate::frame::Next>>,
    /// The surface's host wake-up — held here too so dropping the handle
    /// retires it without borrowing the shared state.
    pub(crate) waker: Arc<SurfaceWaker>,
    tx: Sender<Message<B>>,
}

impl<B: Backend> std::fmt::Debug for Surface<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surface")
            .field("id", &self.id)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> Surface<B> {
    /// Builds the UI-thread handle once `CreateSurface` succeeded.
    #[must_use]
    pub fn new(
        id: SurfaceId,
        info: SurfaceInfo,
        tx: Sender<Message<B>>,
        waker: Arc<SurfaceWaker>,
    ) -> Self {
        let queue = EngineQueue::new(id, Arc::clone(&waker), tx.clone());
        let shared = Rc::new(RefCell::new(Shared::new(id, queue)));
        let root = Shared::root(&shared);
        Self {
            shared,
            id,
            size: Cell::new(info.size),
            readable: info.readable,
            max_dimension: info.max_dimension,
            root,
            next_frame: Rc::new(RefCell::new(crate::frame::Next::Idle)),
            waker,
            tx,
        }
    }

    /// The surface's identifier.
    #[must_use]
    pub const fn id(&self) -> SurfaceId {
        self.id
    }

    /// The root layer.
    #[must_use]
    pub const fn root(&self) -> &Layer {
        &self.root
    }

    /// A new detached layer.
    #[must_use]
    pub fn layer(&self) -> Layer {
        Shared::layer(&self.shared)
    }

    /// The surface size in pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.size.get()
    }

    /// Resizes the surface.
    ///
    /// # Errors
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn resize(&self, size: (u32, u32)) -> Result<(), SurfaceError> {
        if size.0 > self.max_dimension || size.1 > self.max_dimension {
            return Err(SurfaceError::TooLarge {
                width: size.0,
                height: size.1,
                max: self.max_dimension,
            });
        }
        self.tx
            .send(Message::ResizeSurface { id: self.id, size })
            .map_err(|_| SurfaceError::Lost)?;
        self.size.set(size);
        Ok(())
    }

    /// Announces the display's properties (scale and HDR headroom) to the
    /// surface. On a surface whose backend presents (a window), the update
    /// marks the next frame for presentation; on a retained target it
    /// still lands but never marks a present (#98).
    ///
    /// # Errors
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn display(&self, display: Display) -> Result<(), SurfaceError> {
        self.tx
            .send(Message::Display {
                id: self.id,
                display,
            })
            .map_err(|_| SurfaceError::Lost)
    }

    /// Announces the surface moved to another display: the next frame
    /// carries [`SurfaceFrame::display_moved`] and a presenting backend
    /// re-enumerates the surface's output capabilities (#98). Hosts call
    /// this from the platform's display-change notification —
    /// `NSWindowDidChangeScreenNotification`, a winit monitor change, an
    /// Android display change — because a move to a numerically
    /// identical display is invisible in [`Display`]'s values.
    ///
    /// [`SurfaceFrame::display_moved`]: crate::backend::SurfaceFrame::display_moved
    ///
    /// # Errors
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn display_moved(&self) -> Result<(), SurfaceError> {
        self.tx
            .send(Message::DisplayMoved { id: self.id })
            .map_err(|_| SurfaceError::Lost)
    }

    /// The next frame this surface asked for, published by each
    /// [`Engine::render`](crate::Engine::render): its own animation and
    /// backend deadline — not the engine's aggregate, which keeps
    /// answering the whole engine's demand for hosts that run one
    /// presentation loop. [`Next::Idle`](crate::Next::Idle) while nothing
    /// on the surface runs; a surface that never rendered reads `Idle`.
    #[must_use]
    pub fn next_frame(&self) -> crate::frame::Next {
        self.next_frame.borrow().clone()
    }

    /// Announces whether the user can see the surface, from the platform's
    /// visibility signal (window occlusion or minimization, the app moving
    /// to the background, the view leaving its window, the document's
    /// visibility state). Surfaces start [`Visible`](Visibility::Visible);
    /// announcing the current visibility again does nothing.
    ///
    /// While the surface is hidden:
    /// - Nothing on it asks for a frame. Animation tracks, live operands,
    ///   bound signals, transactions, image replacements it draws, custom
    ///   GPU content, filters and external-frame installs wake no host,
    ///   and [`Engine::render`](crate::Engine::render) neither samples nor
    ///   draws it, nor counts it in its [`Next`](crate::Next).
    /// - Its changes are still accepted and applied. Transactions, layer
    ///   creates and drops, bound signals and live operands are sent to the
    ///   render thread as they are made, which applies them without
    ///   sampling or drawing the surface — the changes queued before the
    ///   surface hid included. Content installed while hidden therefore
    ///   counts for resource releases like any other installed content,
    ///   and nothing accumulates on the UI thread however long the surface
    ///   stays hidden.
    /// - A host renders only while a surface is visible: rendering while
    ///   every surface of the engine is hidden fails with
    ///   [`RenderError::Hidden`].
    ///
    /// Becoming visible asks the host for exactly one frame, even if it
    /// dropped a frame it had been asked for while the surface was hidden.
    /// That frame redraws the surface whole from its current state and
    /// presents it. It samples every animation at its own time, so a track
    /// that ran on while the surface was hidden shows where it is now, with
    /// no replay of the frames it missed; a track committed while it was
    /// hidden starts on that frame, like any other.
    ///
    /// Every wake on the surface's behalf stops the moment this returns,
    /// whichever thread it starts on — the backend's producers and filters
    /// included, which wake through the surface's own wake and read the
    /// announced visibility when they fire rather than waiting for the
    /// render thread to apply the change.
    ///
    /// # Errors
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn visibility(&self, visibility: Visibility) -> Result<(), SurfaceError> {
        if self.waker.visibility() == visibility {
            return Ok(());
        }
        let message = Message::Visibility {
            id: self.id,
            visibility,
        };
        match visibility {
            Visibility::Hidden => {
                self.waker.hide();
                self.tx.send(message).map_err(|_| SurfaceError::Lost)?;
                // What was queued for the next frame is applied now, like
                // every later change: hiding switched the queue to its
                // inline drain, which applies the backlog at once.
                self.shared.borrow_mut().flush();
            }
            Visibility::Visible => {
                // The render loop learns first, so the frame the wake asks
                // for lists the surface.
                self.tx.send(message).map_err(|_| SurfaceError::Lost)?;
                self.waker.show();
            }
        }
        Ok(())
    }

    /// Announces that the host is building a frame that ends in
    /// [`Engine::render`](crate::Engine::render): until that render, nothing
    /// on the surface's behalf wakes the host, because the render drains it.
    ///
    /// A host that drives its own frames and edits the surface inside its
    /// frame callback calls this when the frame begins, so the edits it
    /// makes for the frame it is rendering do not ask for another one.
    /// Every wake from then on — the host's own edits, a bound signal, a
    /// producer, filter or backend completion on any thread — is answered
    /// by that render; the render re-arms the wake for whatever comes
    /// after it.
    pub fn begin_frame(&self) {
        self.waker.disarm();
    }

    /// The clear colour, queued into the pending change set. Defaults to
    /// transparent.
    pub fn clear_color(&self, color: WorkingColor) {
        self.shared.borrow_mut().set_clear(color);
    }

    /// Records live content for this surface. The recording reads the root
    /// layer's [`layout_size`](LayerEdit::layout_size); content for another
    /// layer that reads its size is recorded with
    /// [`LayerEdit::record`].
    ///
    /// [`layout_size`]: cherenkov_record::LayerEdit::layout_size
    /// [`LayerEdit::record`]: cherenkov_record::LayerEdit::record
    #[must_use]
    pub fn record(&self, body: impl FnOnce(&mut crate::Recorder)) -> Content {
        let size = self.shared.borrow_mut().layout_size(self.root.id());
        Content::record(&size, body)
    }

    /// Queues a transaction's edits into the surface's change set. Nothing
    /// is sent; [`Engine::render`](crate::Engine::render) drains the queue.
    /// A hidden surface sends the edits at once instead (see
    /// [`Surface::visibility`]).
    ///
    /// # Panics
    /// Panics if `body` panics; the transaction is then dropped unapplied.
    pub fn update(&self, body: impl FnOnce(&mut Transaction<'_, B>)) {
        Shared::run_transaction(&self.shared, None, body);
    }

    /// Like [`Surface::update`], filling `animation` for every animatable
    /// op that lacks one.
    ///
    /// # Panics
    /// Panics if `body` panics.
    pub fn update_animated(
        &self,
        animation: impl Into<Animation>,
        body: impl FnOnce(&mut Transaction<'_, B>),
    ) {
        Shared::run_transaction(&self.shared, Some(animation.into()), body);
    }

    /// The pixels of the surface after the last
    /// [`Engine::render`](crate::Engine::render). Only readable surfaces
    /// (offscreen targets) answer.
    ///
    /// # Errors
    /// [`RenderError::NotReadable`] for a non-readable surface,
    /// [`RenderError::Readback`] when the readback fails, or
    /// [`RenderError::Thread`] when the render thread is gone.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn readback(&self) -> Result<Readback, RenderError> {
        if !self.readable {
            return Err(RenderError::NotReadable);
        }
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Message::Readback {
                surface: self.id,
                reply,
            })
            .map_err(|_| RenderError::Thread)?;
        rx.recv().map_err(|_| RenderError::Thread)?
    }

    /// Reads the last rendered pixels, yielding until browser mapping completes.
    ///
    /// # Errors
    /// Returns `NotReadable`, a readback error, or `Thread` if the executor stopped.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn readback(&self) -> Result<Readback, RenderError> {
        if !self.readable {
            return Err(RenderError::NotReadable);
        }
        let (reply, rx) = crate::local::channel();
        self.tx
            .send(Message::Readback {
                surface: self.id,
                reply,
            })
            .map_err(|_| RenderError::Thread)?;
        rx.recv().await.map_err(|_| RenderError::Thread)?
    }
}

impl<B: Backend> Drop for Surface<B> {
    fn drop(&mut self) {
        // The surface is gone: backend completions landing after this and
        // ops a leaked layer still queues wake nobody.
        self.waker.retire();
        let _ = self.tx.send(Message::DestroySurface { id: self.id });
    }
}

impl<B: Backdrop> Surface<B> {
    /// Allocates a group id and queues its registration with `op`.
    fn new_backdrop_group(
        &self,
        op: impl FnOnce(&mut B::Renderer, SurfaceId, crate::BackdropId)
        + crate::RenderTransfer
        + 'static,
    ) -> crate::BackdropGroup {
        let id = self.shared.borrow_mut().allocate_backdrop();
        let surface = self.id;
        let _ = self.tx.send(Message::Resource(Box::new(move |r| {
            op(r, surface, id);
        })));
        let tx = self.tx.clone();
        crate::BackdropGroup::new(id, move || {
            let _ = tx.send(Message::Resource(Box::new(move |r| {
                B::remove_backdrop_group(r, surface, id);
            })));
        })
    }

    /// Creates a backdrop group on this surface whose members sample the
    /// unfiltered backdrop.
    #[must_use]
    pub fn backdrop_group_unfiltered(&self) -> crate::BackdropGroup {
        self.new_backdrop_group(B::add_backdrop_group)
    }

    /// Creates a backdrop group whose capture runs through `filter` once;
    /// members share the result.
    #[must_use]
    pub fn backdrop_group<K, F>(&self, filter: F) -> crate::BackdropGroup
    where
        K: filtrate_core::kind::Kind,
        F: BackdropChain<K> + crate::RenderTransfer,
        B: BackdropRuns<K, F>,
    {
        self.new_backdrop_group(move |r, surface, id| {
            B::add_filtered_backdrop_group(r, surface, id, filter);
        })
    }
}
