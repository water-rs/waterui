//! Messages the UI thread sends to the render thread. Everything crossing
//! the channel is owned and `Send`; there are no locks anywhere in the
//! engine. The change sets they carry — the layer ops a [`SurfaceTree`]
//! applies — live in `cherenkov-record`.
//!
//! [`SurfaceTree`]: cherenkov_record::SurfaceTree

#[cfg(target_arch = "wasm32")]
use crate::local::ReplySender as Sender;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Sender, SyncSender};

#[cfg(target_arch = "wasm32")]
pub type FrameReplySender<T> = Sender<T>;
#[cfg(not(target_arch = "wasm32"))]
pub type FrameReplySender<T> = SyncSender<T>;

use cherenkov_record::{ChangeSet, LayerId, ResourceId, SurfaceId};

use crate::backend::{Backend, Display, Renderer, SurfaceInfo};
use crate::config::{MemoryUsage, Pressure};
use crate::error::{RenderError, ResourceError, SurfaceError};
use crate::frame::{FrameStats, FrameTime, FrameTiming, Next, Readback};
use crate::image::ImageUpload;
use crate::paint::ImageId;

/// A render-thread operation a capability method or a resource drop queues.
#[cfg(not(target_arch = "wasm32"))]
pub type ResOp<B> = Box<dyn FnOnce(&mut <B as Backend>::Renderer) + Send>;
/// A resource operation that stays on the creating JS thread.
#[cfg(target_arch = "wasm32")]
pub type ResOp<B> = Box<dyn FnOnce(&mut <B as Backend>::Renderer)>;
/// A render-side install: a closure run on the render thread.
///
/// It installs on the render side, learns the surface and layer it is
/// installed on, and reports the installed content's declared alpha —
/// `Some(opaque)` from the producer's current frame, `None` before one
/// has landed — which the [`Op::Install`](cherenkov_record::ops::Op::Install)
/// arm notes on the layer.
#[cfg(not(target_arch = "wasm32"))]
pub type InstallOp<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer, SurfaceId, LayerId) -> Option<bool> + Send>;
/// The owning JS thread's [`InstallOp`].
#[cfg(target_arch = "wasm32")]
pub type InstallOp<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer, SurfaceId, LayerId) -> Option<bool>>;
/// A submitted frame's application: installs the frame on the render side
/// and returns the `(surface, layer)` pairs the producer is bound on, so
/// the frame's declared alpha contract is noted on each of them.
#[cfg(not(target_arch = "wasm32"))]
pub type ProducerApply<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer) -> Vec<(SurfaceId, LayerId)> + Send>;
/// The owning JS thread's [`ProducerApply`].
#[cfg(target_arch = "wasm32")]
pub type ProducerApply<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer) -> Vec<(SurfaceId, LayerId)>>;
/// A registration the backend may reject: the render loop records the
/// rejection against the resource.
#[cfg(not(target_arch = "wasm32"))]
pub type RegisterOp<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer) -> Result<(), ResourceError> + Send>;
/// A registration the backend may reject, awaiting browser work on the
/// owning JS thread: the render loop records the rejection against the
/// resource.
#[cfg(target_arch = "wasm32")]
pub type RegisterOp<B> = Box<
    dyn for<'a> FnOnce(
        &'a mut <B as Backend>::Renderer,
    ) -> core::pin::Pin<
        Box<dyn core::future::Future<Output = Result<(), ResourceError>> + 'a>,
    >,
>;

/// Identifier of a GPU producer shared across surfaces, allocated by
/// [`Engine::gpu_producer`](crate::Engine::gpu_producer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProducerId(u64);

impl ProducerId {
    /// Creates an identifier from a raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A font crossing to the render thread.
#[derive(Clone)]
pub struct FontData {
    /// The raw font data.
    pub data: Arc<[u8]>,
    /// The font index inside a collection.
    pub index: u32,
}

impl std::fmt::Debug for FontData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontData")
            .field("len", &self.data.len())
            .field("index", &self.index)
            .finish()
    }
}

/// What one rendered frame produces: the aggregate [`Next`], each listed
/// surface's own deadline, the frame's stats, and the frame's
/// [`Renderer::FrameCommit`], the work the awaiting caller applies on its
/// own thread before `render` returns (a window surface's main-thread
/// present).
pub type RenderOutcome<B> = Result<
    (
        Next,
        rustc_hash::FxHashMap<SurfaceId, Next>,
        FrameStats,
        <<B as Backend>::Renderer as Renderer>::FrameCommit,
    ),
    RenderError,
>;

/// The render result and drained buffers returned to the UI thread.
pub struct RenderReply<B: Backend> {
    /// The result of rendering the frame — the deadlines the engine
    /// publishes through [`Surface::next_frame`](crate::Surface::next_frame).
    pub result: RenderOutcome<B>,
    /// The drained commits, including their reusable empty op vectors.
    pub commits: Vec<(SurfaceId, ChangeSet<B>)>,
    /// The persistent reply sender, returned so a disconnected render thread
    /// releases the receiver.
    #[cfg(not(target_arch = "wasm32"))]
    pub sender: FrameReplySender<Self>,
}

impl<B: Backend> std::fmt::Debug for RenderReply<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderReply")
            .field("ok", &self.result.is_ok())
            .field("commits", &self.commits.len())
            .finish_non_exhaustive()
    }
}

/// Memory usage and its persistent reply sender.
#[derive(Debug)]
pub struct MemoryReply {
    /// The engine's current usage.
    pub usage: MemoryUsage,
    /// The persistent reply sender, returned so a disconnected render thread
    /// releases the receiver.
    #[cfg(not(target_arch = "wasm32"))]
    pub sender: FrameReplySender<Self>,
}

/// A message to the render thread.
pub enum Message<B: Backend> {
    /// Create a surface.
    CreateSurface {
        /// The new surface id.
        id: SurfaceId,
        /// What it renders into.
        target: B::Target,
        /// The surface's host wake-up, shared with its UI-thread handle.
        waker: Arc<crate::engine::SurfaceWaker>,
        /// Result of the creation.
        reply: Sender<Result<SurfaceInfo, SurfaceError>>,
    },
    /// Resize a surface.
    ResizeSurface {
        /// The surface id.
        id: SurfaceId,
        /// New size in pixels.
        size: (u32, u32),
    },
    /// Destroy a surface and its layer tree.
    DestroySurface {
        /// The surface id.
        id: SurfaceId,
    },
    /// Update a surface's display properties.
    Display {
        /// The surface id.
        id: SurfaceId,
        /// The new display properties.
        display: Display,
    },
    /// Announce whether the user can see the surface. A hidden surface is
    /// left out of every frame; becoming visible marks it for presentation.
    Visibility {
        /// The surface id.
        id: SurfaceId,
        /// The new visibility; always differs from the previous one.
        visibility: crate::backend::Visibility,
    },
    /// Announce the surface moved to another display: the next frame
    /// carries `display_moved` and re-runs output negotiation (#98).
    DisplayMoved {
        /// The surface id.
        id: SurfaceId,
    },
    /// An opaque render-thread operation that cannot fail: font, filter
    /// and effect registration and removal, capability hooks.
    Resource(ResOp<B>),
    /// A submitted frame for a frame producer
    /// ([`FrameSink::submit`](crate::FrameSink::submit)). Applied in order;
    /// the layers the producer is bound on get the frame's declared alpha
    /// contract noted and count as a frame swap on their surface — a
    /// planes-capable backend presents those alone when they are the
    /// surface's only change (#90).
    ProducerFrame {
        /// The producer the frame belongs to.
        producer: ProducerId,
        /// Whether the frame's declared alpha contract is fully opaque.
        opaque: bool,
        /// Installs the frame and returns the layers the producer is
        /// bound on.
        apply: ProducerApply<B>,
    },
    /// Register a resource the backend may reject. A rejection is recorded
    /// against `resource`, and every render that draws it fails with
    /// [`RenderError::Rejected`].
    Register {
        /// The resource being registered.
        resource: ResourceId,
        /// The backend's registration.
        op: RegisterOp<B>,
    },
    /// Release a resource registered with [`Message::Register`]. `op`, the
    /// backend's removal, runs only when the backend holds the resource: a
    /// rejected registration committed nothing.
    Release {
        /// The resource being released.
        resource: ResourceId,
        /// The backend's removal.
        op: ResOp<B>,
    },
    /// Replace a registered image's pixels behind the same id, then mark
    /// changed every surface whose content samples the image. A rejection
    /// is recorded against the image, as for [`Message::Register`]; the
    /// next successful replacement clears it.
    ReplaceImage {
        /// The image.
        id: ImageId,
        /// The new pixels.
        image: ImageUpload,
    },
    /// Apply a hidden surface's changes, sent as they are made: no frame is
    /// rendered and nothing is sampled, and the releases waiting on the
    /// surface's installed content are settled.
    Apply {
        /// The hidden surface.
        id: SurfaceId,
        /// Its changes since the last ones it sent.
        changes: ChangeSet<B>,
    },
    /// Render every dirty surface for the frame at `time`, applying every
    /// surface's queued change set first.
    Render {
        /// The frame time.
        time: FrameTime,
        /// The surfaces' queued change sets, one entry per dirty surface.
        commits: Vec<(SurfaceId, ChangeSet<B>)>,
        /// The render result and the buffers returned to the UI thread.
        reply: FrameReplySender<RenderReply<B>>,
    },
    /// Wait for every outstanding frame timing and return it.
    FinishTimings {
        /// The timings, oldest first.
        reply: Sender<Result<Vec<FrameTiming>, RenderError>>,
    },
    /// Read back a surface's pixels.
    Readback {
        /// The surface.
        surface: SurfaceId,
        /// The decoded pixels.
        reply: Sender<Result<Readback, RenderError>>,
    },
    /// Report memory usage.
    Memory {
        /// The usage.
        reply: FrameReplySender<MemoryReply>,
    },
    /// System memory pressure.
    Trim(Pressure),
    /// Stop the render thread.
    Shutdown,
}

impl<B: Backend> std::fmt::Debug for Message<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Variants carry backend targets and opaque render-side closures.
        f.write_str("Message(..)")
    }
}
