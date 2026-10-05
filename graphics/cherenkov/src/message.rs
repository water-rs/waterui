//! Messages the UI thread sends to the render thread, and the change-set
//! types they carry. Everything crossing the channel is owned and `Send`;
//! there are no locks anywhere in the engine.

#[cfg(target_arch = "wasm32")]
use crate::local::ReplySender as Sender;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Sender, SyncSender};

#[cfg(target_arch = "wasm32")]
pub type FrameReplySender<T> = Sender<T>;
#[cfg(not(target_arch = "wasm32"))]
pub type FrameReplySender<T> = SyncSender<T>;

use kurbo::{Affine, Vec2};

use crate::WorkingColor;
use crate::animation::Animation;
use crate::backend::{Backend, Display, SurfaceInfo};
use crate::config::{MemoryUsage, Pressure};
use crate::display_list::{Picture, SlotUpdate};
use crate::error::{RenderError, ResourceError, SurfaceError};
use crate::frame::{FrameStats, FrameTime, FrameTiming, Next, Readback};
use crate::image::ImageUpload;
use crate::paint::ImageId;
use crate::resource::ResourceId;
use crate::shape::ShapeData;
use crate::style::{BlendMode, FilterId};

/// A render-thread operation a capability method or a resource drop queues.
#[cfg(not(target_arch = "wasm32"))]
pub type ResOp<B> = Box<dyn FnOnce(&mut <B as Backend>::Renderer) + Send>;
/// A resource operation that stays on the creating JS thread.
#[cfg(target_arch = "wasm32")]
pub type ResOp<B> = Box<dyn FnOnce(&mut <B as Backend>::Renderer)>;
/// A render-side install: the closure reports the installed content's
/// declared alpha — `Some(opaque)` from the producer's current frame,
/// `None` before one has landed — which the [`Op::Install`] arm notes
/// on the layer.
#[cfg(not(target_arch = "wasm32"))]
pub type InstallApply<B> = Box<dyn FnOnce(&mut <B as Backend>::Renderer) -> Option<bool> + Send>;
/// The owning JS thread's [`InstallApply`].
#[cfg(target_arch = "wasm32")]
pub type InstallApply<B> = Box<dyn FnOnce(&mut <B as Backend>::Renderer) -> Option<bool>>;
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

/// Identifier of a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(u64);

impl SurfaceId {
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

/// Identifier of a layer within a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(u64);

impl LayerId {
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

/// Identifier of a backdrop group, allocated per surface by
/// [`Surface::backdrop_group`](crate::Surface::backdrop_group).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BackdropId(u64);

impl BackdropId {
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

// `BackdropShaderId` lives in `cherenkov-record` with the other plain
// resource ids; re-exported so `crate::message::BackdropShaderId` still
// resolves.
pub use cherenkov_record::BackdropShaderId;

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

/// A property target plus the animation that reaches it.
#[derive(Clone, Debug)]
pub struct Prop<T> {
    /// The value the property moves to.
    pub target: T,
    /// The animation applied, if any; `None` snaps.
    pub animation: Option<Animation>,
}

/// What a layer draws, crossing the channel.
#[derive(Clone, Debug)]
pub enum ContentOp {
    /// The whole display list of a live content, sent on first commit.
    Replace(Picture),
    /// New values for a live content's bound slots.
    Update(Vec<SlotUpdate>),
    /// A shared immutable picture.
    Picture(Picture),
}

/// One layer mutation in a committed change set.
#[derive(Clone, Debug)]
pub enum LayerOp {
    /// Create a detached layer node.
    Create(LayerId),
    /// Remove a layer node and its descendants.
    Remove(LayerId),
    /// Set the local transform.
    Transform(LayerId, Prop<Affine>),
    /// Sets the translation in local coordinates; initially zero.
    Translation(LayerId, Prop<Vec2>),
    /// Sets the unwrapped rotation angle in radians; initially zero.
    Rotation(LayerId, Prop<f64>),
    /// Sets the x/y scale factors; initially (1, 1).
    Scale(LayerId, Prop<Vec2>),
    /// Sets x/y skew angles in radians; initially zero.
    Skew(LayerId, Prop<Vec2>),
    /// Sets the local pivot for rotation, skew and scale; initially zero.
    Pivot(LayerId, Prop<Vec2>),
    /// Sets the projection base of a projective layer; never animated.
    Projection(LayerId, crate::Projective),
    /// Sets the X/Y depth-rotation angles in radians; initially zero.
    Tilt(LayerId, Prop<Vec2>),
    /// Sets the translation along Z; initially zero.
    Depth(LayerId, Prop<f64>),
    /// Removes projection, tilt and depth; the layer is affine again.
    ClearProjection(LayerId),
    /// Set the opacity.
    Opacity(LayerId, Prop<f32>),
    /// Set the scroll offset.
    ScrollOffset(LayerId, Prop<Vec2>),
    /// Set or clear the clip shape.
    Clip(LayerId, Option<ShapeData>),
    /// Set the blend mode.
    Blend(LayerId, BlendMode),
    /// Set or clear the filter.
    Filter(LayerId, Option<FilterId>),
    /// Set or clear the backdrop sample (group and optional per-member
    /// effect).
    Backdrop(LayerId, Option<crate::BackdropSample>),
    /// Set the layer content, or clear it.
    Content(LayerId, Option<ContentOp>),
    /// Append a child.
    Push {
        /// The parent.
        parent: LayerId,
        /// The child.
        child: LayerId,
    },
    /// Insert a child at an index.
    Insert {
        /// The parent.
        parent: LayerId,
        /// Child index.
        index: usize,
        /// The child.
        child: LayerId,
    },
    /// Remove a child from a parent's child list.
    Detach {
        /// The parent.
        parent: LayerId,
        /// The child.
        child: LayerId,
    },
}

/// One committed op: a layer mutation, or an opaque render-side install a
/// capability method wrapped (GPU producers) travelling in order with the
/// layer ops.
pub enum Op<B: Backend> {
    /// A layer-tree mutation.
    Layer(LayerOp),
    /// A render-side install on `layer`, applied in order: the reported
    /// declared alpha is noted on the layer — `None`, no frame landed
    /// yet, notes it not known opaque.
    Install(LayerId, InstallApply<B>),
}

impl<B: Backend> std::fmt::Debug for Op<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Install` carries an opaque render-side closure.
        match self {
            Self::Layer(op) => f.debug_tuple("Layer").field(op).finish(),
            Self::Install(..) => f.write_str("Install(..)"),
        }
    }
}

/// The committed change set for one surface.
#[derive(Debug)]
pub struct ChangeSet<B: Backend> {
    /// New clear colour, when set this commit.
    pub clear: Option<WorkingColor>,
    /// The ops, in order.
    pub ops: Vec<Op<B>>,
    /// Replaced pictures, cleared on the render thread, whose storage returns to the UI thread.
    pub recycled: Vec<(LayerId, Picture)>,
    /// Whether recorded-content operands still animate: their tracks live
    /// on the UI thread and need the next frame's sample, at the fast rate
    /// class like a spring or curve on a layer.
    pub animating: bool,
}

/// The render result and drained buffers returned to the UI thread.
#[derive(Debug)]
pub struct RenderReply<B: Backend> {
    /// The result of rendering the frame: the aggregate [`Next`] and,
    /// per surface the frame listed, that surface's own deadline —
    /// the values the engine publishes through
    /// [`Surface::next_frame`](crate::Surface::next_frame).
    pub result: Result<(Next, rustc_hash::FxHashMap<SurfaceId, Next>, FrameStats), RenderError>,
    /// The drained commits, including their reusable empty op vectors.
    pub commits: Vec<(SurfaceId, ChangeSet<B>)>,
    /// The persistent reply sender, returned so a disconnected render thread
    /// releases the receiver.
    #[cfg(not(target_arch = "wasm32"))]
    pub sender: FrameReplySender<Self>,
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
        waker: crate::engine::SharedWaker<crate::engine::SurfaceWaker>,
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
        /// The deadline map the frame fills — the last frame's storage,
        /// handed back instead of allocating a fresh map every render.
        next_scratch: rustc_hash::FxHashMap<SurfaceId, Next>,
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
