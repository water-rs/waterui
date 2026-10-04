//! Surfaces, layers and transactions: the UI-thread state machine.
//!
//! `Surface::update`, layer drops and bound-signal changes only queue
//! owned ops; [`Engine::render`](crate::Engine::render) drains every
//! surface's queue into one [`Message::Render`], so the render thread
//! wakes once per frame.

#[cfg(target_arch = "wasm32")]
use crate::local::Sender;
#[cfg(not(target_arch = "wasm32"))]
use crossbeam_channel::Sender;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use kurbo::{Affine, Size, Vec2};
use nami_core::watcher::Context;
use rustc_hash::FxHashMap;

use crate::animation::Animation;
use crate::backend::{Backend, Display, SurfaceInfo, Visibility};
use crate::capability::{Backdrop, BackdropChain, BackdropRuns, ProjectiveLayers};
use crate::engine::SurfaceWaker;
use crate::error::{RenderError, SurfaceError};
use crate::frame::Readback;
use crate::message::{
    BackdropId, ChangeSet, ContentOp, LayerId, LayerOp, Message, Op, Prop, SurfaceId,
};
use crate::record::{Binding, Content, ContentSpare, Live, LiveOwner, SampleFlag};
use crate::shape::{Shape, ShapeData};
use crate::size::LayoutSize;
use crate::style::{BlendMode, FilterId};
use crate::{ContentChange, Picture, WorkingColor};

/// Which bound property a subscription updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PropKind {
    Transform,
    Translation,
    Rotation,
    Scale,
    Skew,
    Pivot,
    Projection,
    Tilt,
    Depth,

    Opacity,
    ScrollOffset,
    Clip,
    LayoutSize,
}

/// State shared between a [`Surface`], its [`Layer`] handles and the
/// engine.
///
/// While the surface is visible everything here is queued, not sent:
/// [`Engine::render`](crate::Engine::render) drains `pending` into the
/// single per-frame [`Message::Render`]. While it is hidden every change is
/// sent as it is made, in a [`Message::Apply`].
pub struct Shared<B: Backend> {
    /// The surface's identifier on the render thread.
    pub id: SurfaceId,
    /// Ops queued outside transactions (layer creates and drops, bound
    /// signal changes) plus queued transaction ops.
    pending: Vec<Op<B>>,
    /// Reusable frame-handoff op buffer.
    spare_ops: Vec<Op<B>>,
    /// Reusable frame-handoff recycled-picture buffer.
    spare_recycled: Vec<(LayerId, Picture)>,
    /// Reusable transaction edit buffers.
    edit_buffer: Vec<(LayerId, LayerEdit<B>)>,
    edit_ops: Vec<Vec<EditOp<B>>>,
    /// Content and reusable storage per layer.
    contents: FxHashMap<LayerId, ContentSlot>,
    /// Each layer's layout size, created when first set or recorded for.
    sizes: FxHashMap<LayerId, LayoutSize>,
    /// Pending clear colour.
    clear: Option<WorkingColor>,
    /// Layer id allocator (0 is the root).
    next_layer: Cell<u64>,
    /// Backdrop group id allocator.
    next_backdrop: Cell<u64>,
    /// Live property subscriptions, keyed by layer and property. Binding a
    /// property replaces its previous subscription; dropping a layer drops
    /// them all.
    bindings: FxHashMap<(u64, PropKind), Binding>,
    /// The surface's host wake-up, fired when an op is queued outside a
    /// frame; silent while the surface is hidden.
    waker: Arc<SurfaceWaker>,
    /// The render loop, which a hidden surface's changes are sent to as
    /// they are made.
    tx: Sender<Message<B>>,
    /// The last display properties announced to the render thread.
    display: Cell<Display>,
    /// Set by installed contents' live states the moment an animated
    /// operand arrives, so [`Shared::take_changes`] skips per-content
    /// sampling probes on surfaces that never saw one.
    animated: SampleFlag,
    /// The `LiveOwner` handle installed contents attach to, created on
    /// first install and held for the surface's life.
    owner: Option<Rc<dyn LiveOwner>>,
}

#[derive(Default)]
struct ContentSlot {
    content: Option<Content>,
    spare: ContentSpare,
}

impl<B: Backend> std::fmt::Debug for Shared<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("id", &self.id)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl<B: Backend> Shared<B> {
    fn new(id: SurfaceId, waker: Arc<SurfaceWaker>, tx: Sender<Message<B>>) -> Self {
        Self {
            id,
            pending: Vec::new(),
            spare_ops: Vec::new(),
            spare_recycled: Vec::new(),
            edit_buffer: Vec::new(),
            edit_ops: Vec::new(),
            contents: FxHashMap::default(),
            sizes: FxHashMap::default(),
            clear: None,
            next_layer: Cell::new(1),
            next_backdrop: Cell::new(1),
            bindings: FxHashMap::default(),
            waker,
            tx,
            display: Cell::new(Display::default()),
            animated: SampleFlag::new(),
            owner: None,
        }
    }

    /// `layer`'s layout size.
    fn layout_size(&mut self, layer: LayerId) -> LayoutSize {
        self.sizes.entry(layer).or_default().clone()
    }

    /// Queues an op outside a frame.
    fn push(&mut self, op: Op<B>) {
        self.pending.push(op);
        self.queued();
    }

    /// Something was queued outside a frame. A visible surface wakes the
    /// host for the frame that applies it; a hidden one sends it to the
    /// render loop at once.
    fn queued(&mut self) {
        match self.visibility() {
            Visibility::Visible => self.waker.wake(),
            Visibility::Hidden => self.apply_hidden(),
        }
    }

    /// Sends everything queued to the render loop, which applies it without
    /// sampling or drawing the surface. A hidden surface's changes are so
    /// applied as they are made, in order with every message sent after
    /// them: a resource release sees the content they install, and nothing
    /// accumulates while the surface stays hidden. Animations are not
    /// sampled; a content's animated operands wait for the frame that
    /// shows the surface.
    fn apply_hidden(&mut self) {
        if let Some(changes) = self.drain(None) {
            // A lost render thread fails the host's next render; there is
            // nothing left to apply the change to.
            let _ = self.tx.send(Message::Apply {
                id: self.id,
                changes,
            });
        }
    }

    /// The visibility the host last announced. A hidden surface's changes
    /// are applied as they are made, and it is not sampled.
    pub(crate) fn visibility(&self) -> Visibility {
        self.waker.visibility()
    }

    /// Samples running operand animations at `time`, then drains pending
    /// ops and live content changes into a change set. Returns `None` when
    /// nothing changed.
    pub fn take_changes(&mut self, time: crate::Instant) -> Option<ChangeSet<B>> {
        self.drain(Some(time))
    }

    /// Drains pending ops and live content changes into a change set,
    /// sampling running operand animations first when a frame `time` is
    /// given. Returns `None` when nothing changed.
    fn drain(&mut self, time: Option<crate::Instant>) -> Option<ChangeSet<B>> {
        let mut ops = std::mem::take(&mut self.spare_ops);
        let recycled = std::mem::take(&mut self.spare_recycled);
        ops.append(&mut self.pending);
        let mut animating = false;
        // `animated` is poked by a content's `LiveState` the moment an
        // animated operand arrives, so a surface that never saw one
        // skips the per-content sampling probes entirely. Without a frame
        // time nothing is sampled, and the flag stays as it is.
        let sampling = time.filter(|_| self.animated.get());
        for (id, slot) in &mut self.contents {
            let Some(content) = slot.content.as_mut() else {
                continue;
            };
            // The sample queues the operands' per-frame values, so
            // `take_change` emits them like signal updates. The flag read
            // keeps a static content at a probe, not a call.
            if let Some(time) = sampling
                && content.needs_sample()
                && content.sample(time).is_animating()
            {
                animating = true;
            }
            if let Some(change) = content.take_change() {
                ops.push(Op::Layer(LayerOp::Content(
                    *id,
                    Some(match change {
                        ContentChange::Replace(list) => ContentOp::Replace(list),
                        ContentChange::Update(updates) => ContentOp::Update(updates),
                    }),
                )));
            }
        }
        let clear = self.clear.take();
        if sampling.is_some() && !animating {
            // Nothing sampled this pass: the flag stays down until an
            // animated change pokes it up again.
            self.animated.set(false);
        }
        if clear.is_some() || !ops.is_empty() || !recycled.is_empty() {
            Some(ChangeSet {
                clear,
                ops,
                recycled,
                animating,
            })
        } else {
            self.spare_ops = ops;
            self.spare_recycled = recycled;
            None
        }
    }

    pub(crate) fn recycle(&mut self, ops: Vec<Op<B>>, recycled: &mut Vec<(LayerId, Picture)>) {
        self.spare_ops = ops;
        for (layer, picture) in recycled.drain(..) {
            if let Some(slot) = self.contents.get_mut(&layer) {
                slot.spare.put_picture(picture);
            }
        }
        self.spare_recycled = std::mem::take(recycled);
    }

    /// Binds `live` so its later changes queue `op(layer, value, animation)`
    /// and fire the waker, returning the value the binding starts from.
    /// Replaces the property's previous binding.
    fn bind<T, F>(
        shared: &Rc<RefCell<Self>>,
        layer: LayerId,
        kind: PropKind,
        live: Live<T>,
        op: F,
    ) -> T
    where
        T: 'static,
        F: Fn(LayerId, T, Option<Animation>) -> LayerOp + 'static,
    {
        let weak = Rc::downgrade(shared);
        let (target, guard) = live.watch(move |context: Context<T>| {
            let animation = context.metadata().try_get::<Animation>();
            let target = context.into_value();
            if let Some(shared) = weak.upgrade() {
                shared
                    .borrow_mut()
                    .push(Op::Layer(op(layer, target, animation)));
            }
        });
        let mut shared_mut = shared.borrow_mut();
        if let Some(guard) = guard {
            shared_mut.bindings.insert((layer.raw(), kind), guard);
        } else {
            // Nothing to keep alive, but a previous binding is still
            // replaced by this subscription.
            shared_mut.bindings.remove(&(layer.raw(), kind));
        }
        target
    }

    /// Drops the subscription bound to `layer`'s `kind`, if any.
    fn unbind(shared: &Rc<RefCell<Self>>, layer: LayerId, kind: PropKind) {
        shared.borrow_mut().bindings.remove(&(layer.raw(), kind));
    }
}

/// A surface's bookkeeping shared with its [`Layer`] handles.
pub trait LayerOwner {
    /// Allocates a layer id and queues its `Create`.
    fn allocate(&self) -> LayerId;
    /// Queues a `Remove` and drops the layer's bindings and contents.
    fn remove(&self, id: LayerId);
}

/// The surface as an installed content's live-operand owner: a
/// signal change queues the surface. It weakly holds the surface, which
/// holds the one [`Rc`] of it, so neither keeps the other alive.
struct Owner<B: Backend>(Weak<RefCell<Shared<B>>>);

impl<B: Backend> LiveOwner for Owner<B> {
    fn changed(&self) {
        if let Some(shared) = self.0.upgrade() {
            shared.borrow_mut().queued();
        }
    }
}

impl<B: Backend> LayerOwner for RefCell<Shared<B>> {
    fn allocate(&self) -> LayerId {
        let mut shared = self.borrow_mut();
        let id = LayerId::new(shared.next_layer.get());
        shared.next_layer.set(id.raw() + 1);
        shared.push(Op::Layer(LayerOp::Create(id)));
        id
    }

    fn remove(&self, id: LayerId) {
        let mut shared = self.borrow_mut();
        shared.bindings.retain(|(layer, _), _| *layer != id.raw());
        shared.contents.remove(&id);
        shared.sizes.remove(&id);
        shared.push(Op::Layer(LayerOp::Remove(id)));
    }
}

/// A layer handle. Layers are `!Send` and not `Clone`; dropping one removes
/// it from its surface at the next commit.
pub struct Layer {
    id: LayerId,
    owner: Rc<dyn LayerOwner>,
    remove_on_drop: bool,
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("id", &self.id)
            .field("remove_on_drop", &self.remove_on_drop)
            .finish_non_exhaustive()
    }
}

impl Layer {
    /// The layer's id.
    #[must_use]
    pub const fn id(&self) -> LayerId {
        self.id
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        if self.remove_on_drop {
            self.owner.remove(self.id);
        }
    }
}

/// What a layer draws.
pub enum LayerContent<B: Backend> {
    /// Live recorded content; its bound signals keep updating it with no
    /// further transactions.
    Content(Content),
    /// A shared immutable picture.
    Picture(Picture),
    /// An opaque render-side install (GPU producers). The closure learns
    /// the surface and layer it is installed on when the edit is applied
    /// in `update`, and reports the installed content's declared alpha —
    /// `None` before the producer's first frame — which the layer's
    /// alpha contract notes.
    Install(InstallOp<B>),
    /// Nothing.
    None,
}

impl<B: Backend> std::fmt::Debug for LayerContent<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Install` carries an opaque render-side closure.
        match self {
            Self::Content(content) => f.debug_tuple("Content").field(content).finish(),
            Self::Picture(picture) => f.debug_tuple("Picture").field(picture).finish(),
            Self::Install(_) => f.write_str("Install(..)"),
            Self::None => f.write_str("None"),
        }
    }
}

impl<B: Backend> From<Content> for LayerContent<B> {
    fn from(content: Content) -> Self {
        Self::Content(content)
    }
}

impl<B: Backend> From<Picture> for LayerContent<B> {
    fn from(picture: Picture) -> Self {
        Self::Picture(picture)
    }
}

/// An opaque render-side install a [`GpuContent`](crate::GpuContent)
/// capability wraps; the closure learns its surface and layer at apply
/// time and reports the installed content's declared alpha — `Some`
/// once a frame landed, `None` before — for the layer's alpha contract.
#[cfg(not(target_arch = "wasm32"))]
type InstallOp<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer, SurfaceId, LayerId) -> Option<bool> + Send>;
#[cfg(target_arch = "wasm32")]
type InstallOp<B> =
    Box<dyn FnOnce(&mut <B as Backend>::Renderer, SurfaceId, LayerId) -> Option<bool>>;

/// A recorded layer edit inside a [`Transaction`].
enum EditOp<B: Backend> {
    Transform(Prop<Affine>),
    Translation(Prop<Vec2>),
    Rotation(Prop<f64>),
    Scale(Prop<Vec2>),
    Skew(Prop<Vec2>),
    Pivot(Prop<Vec2>),
    Projection(crate::Projective),
    Tilt(Prop<Vec2>),
    Depth(Prop<f64>),
    ClearProjection,

    Opacity(Prop<f32>),
    ScrollOffset(Prop<Vec2>),
    Clip(Option<ShapeData>),
    Blend(BlendMode),
    Filter(Option<FilterId>),
    Backdrop(Option<crate::BackdropSample>),
    Content(LayerContent<B>),
    Push(LayerId),
    Insert(usize, LayerId),
    Detach(LayerId),
}

/// One layer's pending edits, collected inside a [`Transaction`]. Each
/// method records an op and returns `&mut Self` for chaining.
///
/// `transform`, `opacity`, `scroll_offset` and `clip` accept a constant or
/// a nami signal (`impl Into<Live<T>>`): a bound signal keeps updating the
/// layer with no further transactions, and a change whose nami `Context`
/// metadata carries an [`Animation`] interpolates on the render thread.
pub struct LayerEdit<B: Backend> {
    ops: Vec<EditOp<B>>,
    layer: LayerId,
    shared: Rc<RefCell<Shared<B>>>,
    /// The transaction-wide animation, filled for animatable ops that lack
    /// one.
    default_animation: Option<Animation>,
}

impl<B: Backend> std::fmt::Debug for LayerEdit<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerEdit")
            .field("layer", &self.layer)
            .field("default_animation", &self.default_animation)
            .finish_non_exhaustive()
    }
}

impl<B: ProjectiveLayers> LayerEdit<B> {
    /// Makes the layer projective with `value` as its projection base.
    ///
    /// A projective layer is a flattening boundary: its content, clip,
    /// scroll offset, filter and children render into a layer-local image
    /// with the ordinary affine rasterizers, and that image is projected
    /// when it composes into its parent, where its opacity and blend apply
    /// once. The complete pose is `transform · translate(translation +
    /// pivot) · projection · translate_z(depth) · rotate_z(rotation) ·
    /// rotate_y(tilt.y) · rotate_x(tilt.x) · skew · scale ·
    /// translate(−pivot)`, documented in `docs/api.md`. A projective
    /// layer's local image is bounded by its clip: rendering one without a
    /// clip is an error.
    ///
    /// The raw matrix is not animatable: a new value, bound or set,
    /// replaces the base, and `.animation(...)` after it panics. Animate
    /// flips with [`Self::tilt`], [`Self::depth`] and the affine
    /// components.
    pub fn projection(&mut self, value: impl Into<Live<crate::Projective>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Projection,
            value.into(),
            |layer, target, _| LayerOp::Projection(layer, target),
        );
        self.ops.push(EditOp::Projection(target));
        self
    }

    /// Sets the X and Y rotation angles, in radians; initially zero.
    /// Rotation is about the pivot: positive `x` turns the top edge away
    /// from the viewer, positive `y` turns the right edge away. Angles
    /// are unwrapped: a turn from `0` to `2π` makes a full flip, and both
    /// sides of the layer render. Without [`Self::projection`] the layer
    /// becomes projective with an identity base (an orthographic depth
    /// rotation).
    pub fn tilt(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Tilt,
            value.into(),
            |layer, target, animation| LayerOp::Tilt(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Tilt(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the translation along Z, in layer-coordinate units; initially
    /// zero. Positive values move toward the viewer. Like [`Self::tilt`],
    /// it makes the layer projective.
    pub fn depth(&mut self, value: impl Into<Live<f64>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Depth,
            value.into(),
            |layer, target, animation| LayerOp::Depth(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Depth(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Removes projection, tilt and depth, including their subscriptions
    /// and animation tracks. Existing affine components are unchanged.
    pub fn clear_projection(&mut self) -> &mut Self {
        for kind in [PropKind::Projection, PropKind::Tilt, PropKind::Depth] {
            Shared::unbind(&self.shared, self.layer, kind);
        }
        self.ops.push(EditOp::ClearProjection);
        self
    }
}

impl<B: Backend> LayerEdit<B> {
    /// Sets the local transform.
    pub fn transform(&mut self, transform: impl Into<Live<Affine>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Transform,
            transform.into(),
            |layer, target, animation| LayerOp::Transform(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Transform(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the translation in local coordinates; initially zero.
    /// Component properties compose after [`Self::transform`], in the order
    /// documented in `docs/api.md`. Each keeps its own live subscription and
    /// animation track; changing it never re-records content.
    pub fn translation(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Translation,
            value.into(),
            |layer, target, animation| LayerOp::Translation(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Translation(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the unwrapped rotation angle in radians; initially zero.
    /// Positive angles rotate clockwise in a downward-y coordinate system.
    /// Full turns are preserved; no shortest-path angle normalization occurs.
    /// Component properties compose after [`Self::transform`], in the order
    /// documented in `docs/api.md`. Each keeps its own live subscription and
    /// animation track; changing it never re-records content.
    pub fn rotation(&mut self, value: impl Into<Live<f64>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Rotation,
            value.into(),
            |layer, target, animation| LayerOp::Rotation(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Rotation(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the x/y scale factors; initially (1, 1).
    /// Component properties compose after [`Self::transform`], in the order
    /// documented in `docs/api.md`. Each keeps its own live subscription and
    /// animation track; changing it never re-records content.
    pub fn scale(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Scale,
            value.into(),
            |layer, target, animation| LayerOp::Scale(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Scale(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets x/y skew angles in radians; initially zero.
    /// The skew matrix is `[1, tan(y), tan(x), 1, 0, 0]`.
    /// Component properties compose after [`Self::transform`], in the order
    /// documented in `docs/api.md`. Each keeps its own live subscription and
    /// animation track; changing it never re-records content.
    pub fn skew(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Skew,
            value.into(),
            |layer, target, animation| LayerOp::Skew(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Skew(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the local pivot for rotation, skew and scale; initially zero.
    /// Component properties compose after [`Self::transform`], in the order
    /// documented in `docs/api.md`. Each keeps its own live subscription and
    /// animation track; changing it never re-records content.
    pub fn pivot(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Pivot,
            value.into(),
            |layer, target, animation| LayerOp::Pivot(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Pivot(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the opacity.
    pub fn opacity(&mut self, opacity: impl Into<Live<f32>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Opacity,
            opacity.into(),
            |layer, target, animation| LayerOp::Opacity(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::Opacity(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the scroll offset.
    pub fn scroll_offset(&mut self, offset: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::ScrollOffset,
            offset.into(),
            |layer, target, animation| LayerOp::ScrollOffset(layer, Prop { target, animation }),
        );
        self.ops.push(EditOp::ScrollOffset(Prop {
            target,
            animation: self.default_animation,
        }));
        self
    }

    /// Sets the clip shape, applied in the layer's own space.
    pub fn clip<S: Shape + 'static>(&mut self, shape: impl Into<Live<S>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Clip,
            shape.into(),
            |layer, shape: S, _| LayerOp::Clip(layer, Some(ShapeData::of(&shape))),
        );
        self.ops.push(EditOp::Clip(Some(ShapeData::of(&target))));
        self
    }

    /// Clears the clip.
    pub fn clear_clip(&mut self) -> &mut Self {
        self.ops.push(EditOp::Clip(None));
        self
    }

    /// Sets the blend mode the layer composites onto its parent with.
    pub fn blend(&mut self, blend: BlendMode) -> &mut Self {
        self.ops.push(EditOp::Blend(blend));
        self
    }

    /// Sets the filter applied to this layer's subtree.
    pub fn filter(&mut self, filter: &crate::Filter) -> &mut Self {
        self.ops.push(EditOp::Filter(Some(filter.id())));
        self
    }

    /// Clears the layer's filter.
    pub fn clear_filter(&mut self) -> &mut Self {
        self.ops.push(EditOp::Filter(None));
        self
    }

    /// Makes the layer a member of a backdrop group: it composites the
    /// group's capture as the bottom-most draw inside its clip. A sample
    /// made with [`BackdropGroup::sample_with`](crate::BackdropGroup::sample_with)
    /// carries a per-member effect evaluated in the member's composite.
    pub fn backdrop(&mut self, sample: crate::BackdropSample) -> &mut Self {
        self.ops.push(EditOp::Backdrop(Some(sample)));
        self
    }

    /// Clears the layer's backdrop group membership.
    pub fn clear_backdrop(&mut self) -> &mut Self {
        self.ops.push(EditOp::Backdrop(None));
        self
    }

    /// Sets the content.
    pub fn content(&mut self, content: impl Into<LayerContent<B>>) -> &mut Self {
        self.ops.push(EditOp::Content(content.into()));
        self
    }

    /// Records content, reusing picture storage returned by the render thread.
    /// The recording reads this layer's [`layout_size`](Self::layout_size).
    pub fn record(&mut self, body: impl FnOnce(&mut crate::Recorder)) -> &mut Self {
        let (spare, size) = {
            let mut shared = self.shared.borrow_mut();
            let spare = std::mem::take(&mut shared.contents.entry(self.layer).or_default().spare);
            (spare, shared.layout_size(self.layer))
        };
        self.ops.push(EditOp::Content(LayerContent::Content(
            Content::record_into(spare, &size, body),
        )));
        self
    }

    /// Sets the size the host lays the layer out at: a constant, or a
    /// signal (the host's layout result) that keeps it updated with no
    /// further transactions. Recordings for the layer read it as the
    /// [`LayoutSize`] signal [`Recorder::layout_size`](crate::Recorder::layout_size)
    /// returns, so geometry bound to it follows a resize without
    /// re-recording.
    ///
    /// The size changes when this is called, so a recording later in the
    /// same transaction already reads it. Recorded operands that depend on
    /// it animate under the transaction's animation
    /// ([`Surface::update_animated`]) or a bound change's `Animation`
    /// metadata, like any animated operand; it is not a render-thread
    /// property, so [`animation`](Self::animation) does not apply to it.
    pub fn layout_size(&mut self, size: impl Into<Live<Size>>) -> &mut Self {
        let target = self.shared.borrow_mut().layout_size(self.layer);
        let bound = target.clone();
        let (value, guard) = size.into().watch(move |change| bound.set(&change));
        target.set(&LayoutSize::change(value, self.default_animation));
        let key = (self.layer.raw(), PropKind::LayoutSize);
        {
            let mut shared = self.shared.borrow_mut();
            match guard {
                Some(guard) => {
                    shared.bindings.insert(key, guard);
                }
                None => {
                    shared.bindings.remove(&key);
                }
            }
        }
        self
    }

    /// Clears the content.
    pub fn clear_content(&mut self) -> &mut Self {
        self.ops.push(EditOp::Content(LayerContent::None));
        self
    }

    /// Appends a child layer.
    pub fn push(&mut self, child: &Layer) -> &mut Self {
        self.ops.push(EditOp::Push(child.id));
        self
    }

    /// Inserts a child layer at `index`.
    pub fn insert(&mut self, index: usize, child: &Layer) -> &mut Self {
        self.ops.push(EditOp::Insert(index, child.id));
        self
    }

    /// Removes a child layer.
    pub fn remove(&mut self, child: &Layer) -> &mut Self {
        self.ops.push(EditOp::Detach(child.id));
        self
    }

    /// Overrides the animation of the last recorded property op.
    ///
    /// # Panics
    /// Panics unless the last op was a transform component, `tilt`,
    /// `depth`, `transform`, `opacity` or `scroll_offset` — `.animation(...)` on any other property is an
    /// invariant violation — and panics when `animation` is a
    /// [`Decay`](crate::Decay) on anything but `scroll_offset`.
    pub fn animation(&mut self, animation: impl Into<Animation>) -> &mut Self {
        let animation = animation.into();
        assert!(
            !matches!(animation, Animation::Decay(_))
                || matches!(self.ops.last(), Some(EditOp::ScrollOffset(_))),
            "Decay is only legal on scroll_offset"
        );
        match self.ops.last_mut() {
            Some(EditOp::Transform(prop)) => prop.animation = Some(animation),
            Some(
                EditOp::Translation(prop)
                | EditOp::Tilt(prop)
                | EditOp::Scale(prop)
                | EditOp::Skew(prop)
                | EditOp::Pivot(prop)
                | EditOp::ScrollOffset(prop),
            ) => prop.animation = Some(animation),
            Some(EditOp::Rotation(prop) | EditOp::Depth(prop)) => {
                prop.animation = Some(animation);
            }
            Some(EditOp::Projection(_)) => {
                panic!(
                    "the projection matrix is not animatable; animate tilt, depth or the components"
                )
            }

            Some(EditOp::Opacity(prop)) => prop.animation = Some(animation),
            _ => panic!("animation() must follow an animatable layer property"),
        }
        self
    }
}

/// A transaction's edits to a surface's layer tree. `tx[&layer]` returns
/// the [`LayerEdit`] accumulating that layer's changes.
pub struct Transaction<'a, B: Backend> {
    edits: Vec<(LayerId, LayerEdit<B>)>,
    edit_ops: Vec<Vec<EditOp<B>>>,
    shared: &'a Rc<RefCell<Shared<B>>>,
    /// The transaction-wide animation (`Surface::update_animated`).
    animation: Option<Animation>,
    _surface: PhantomData<&'a Surface<B>>,
}

impl<B: Backend> std::fmt::Debug for Transaction<'_, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction")
            .field("animation", &self.animation)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> Transaction<'_, B> {
    fn edit(&mut self, layer: &Layer) -> &mut LayerEdit<B> {
        let id = layer.id;
        let index = self.edits.iter().position(|(l, _)| *l == id);
        if let Some(index) = index {
            &mut self.edits[index].1
        } else {
            self.edits.push((
                id,
                LayerEdit {
                    ops: self.edit_ops.pop().unwrap_or_default(),
                    layer: id,
                    shared: Rc::clone(self.shared),
                    default_animation: self.animation,
                },
            ));
            &mut self.edits.last_mut().expect("just pushed").1
        }
    }
}

impl<B: Backend> Index<&Layer> for Transaction<'_, B> {
    type Output = LayerEdit<B>;

    fn index(&self, layer: &Layer) -> &Self::Output {
        self.edits
            .iter()
            .find(|(id, _)| *id == layer.id)
            .map(|(_, edit)| edit)
            .expect("the layer has no edits in this transaction yet")
    }
}

impl<B: Backend> IndexMut<&Layer> for Transaction<'_, B> {
    fn index_mut(&mut self, layer: &Layer) -> &mut Self::Output {
        self.edit(layer)
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
        let shared = Rc::new(RefCell::new(Shared::new(id, waker, tx.clone())));
        let owner: Rc<dyn LayerOwner> = Rc::clone(&shared) as Rc<dyn LayerOwner>;
        Self {
            shared,
            id,
            size: Cell::new(info.size),
            readable: info.readable,
            max_dimension: info.max_dimension,
            root: Layer {
                id: LayerId::new(0),
                owner,
                remove_on_drop: false,
            },
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
        let owner: Rc<dyn LayerOwner> = Rc::clone(&self.shared) as Rc<dyn LayerOwner>;
        let id = owner.allocate();
        Layer {
            id,
            owner,
            remove_on_drop: true,
        }
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
            .map_err(|_| SurfaceError::Lost)?;
        self.shared.borrow().display.set(display);
        Ok(())
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
    /// whichever thread it starts on: the surface's own, and those of the
    /// backend's producers and filters, which read the announced
    /// visibility when they fire rather than waiting for the render thread
    /// to apply the change.
    ///
    /// # Errors
    /// [`SurfaceError::Lost`] when the render thread is gone.
    pub fn visibility(&self, visibility: Visibility) -> Result<(), SurfaceError> {
        let waker = Arc::clone(&self.shared.borrow().waker);
        if waker.visibility() == visibility {
            return Ok(());
        }
        let message = Message::Visibility {
            id: self.id,
            visibility,
        };
        match visibility {
            Visibility::Hidden => {
                waker.hide();
                self.tx.send(message).map_err(|_| SurfaceError::Lost)?;
                // What was queued for the next frame is applied now, like
                // every later change.
                self.shared.borrow_mut().apply_hidden();
            }
            Visibility::Visible => {
                // The render loop learns first, so the frame the wake asks
                // for lists the surface.
                self.tx.send(message).map_err(|_| SurfaceError::Lost)?;
                waker.show();
            }
        }
        Ok(())
    }

    /// The clear colour, queued into the pending change set. Defaults to
    /// transparent.
    pub fn clear_color(&self, color: WorkingColor) {
        let mut shared = self.shared.borrow_mut();
        shared.clear = Some(color);
        shared.queued();
    }

    /// Records live content for this surface. The recording reads the root
    /// layer's [`layout_size`](LayerEdit::layout_size); content for another
    /// layer that reads its size is recorded with
    /// [`LayerEdit::record`].
    #[must_use]
    pub fn record(&self, body: impl FnOnce(&mut crate::Recorder)) -> Content {
        let size = self.shared.borrow_mut().layout_size(self.root.id);
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
        self.run_transaction(None, body);
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
        self.run_transaction(Some(animation.into()), body);
    }

    #[expect(clippy::too_many_lines, reason = "one edit-op dispatch per design")]
    fn run_transaction(
        &self,
        animation: Option<Animation>,
        body: impl FnOnce(&mut Transaction<'_, B>),
    ) {
        let (edits, edit_ops) = {
            let mut shared = self.shared.borrow_mut();
            (
                std::mem::take(&mut shared.edit_buffer),
                std::mem::take(&mut shared.edit_ops),
            )
        };
        let mut tx = Transaction {
            edits,
            edit_ops,
            shared: &self.shared,
            animation,
            _surface: PhantomData,
        };
        body(&mut tx);
        let mut shared = self.shared.borrow_mut();
        // Ops queued between updates (layer creates, drops, bound-signal
        // changes) come first.
        let pending = std::mem::take(&mut shared.pending);
        let mut ops = pending;
        // Cloned once per transaction: installed contents attach the
        // surface and its sampling flag to their live states.
        let animated = shared.animated.clone();
        for (id, edit) in &mut tx.edits {
            for op in edit.ops.drain(..) {
                match op {
                    EditOp::Transform(prop) => {
                        ops.push(Op::Layer(LayerOp::Transform(*id, prop)));
                    }
                    EditOp::Translation(prop) => {
                        ops.push(Op::Layer(LayerOp::Translation(*id, prop)));
                    }
                    EditOp::Rotation(prop) => {
                        ops.push(Op::Layer(LayerOp::Rotation(*id, prop)));
                    }
                    EditOp::Scale(prop) => ops.push(Op::Layer(LayerOp::Scale(*id, prop))),
                    EditOp::Skew(prop) => ops.push(Op::Layer(LayerOp::Skew(*id, prop))),
                    EditOp::Pivot(prop) => ops.push(Op::Layer(LayerOp::Pivot(*id, prop))),
                    EditOp::Projection(base) => {
                        ops.push(Op::Layer(LayerOp::Projection(*id, base)));
                    }
                    EditOp::Tilt(prop) => ops.push(Op::Layer(LayerOp::Tilt(*id, prop))),
                    EditOp::Depth(prop) => ops.push(Op::Layer(LayerOp::Depth(*id, prop))),
                    EditOp::ClearProjection => {
                        ops.push(Op::Layer(LayerOp::ClearProjection(*id)));
                    }

                    EditOp::Opacity(prop) => {
                        ops.push(Op::Layer(LayerOp::Opacity(*id, prop)));
                    }
                    EditOp::ScrollOffset(prop) => {
                        ops.push(Op::Layer(LayerOp::ScrollOffset(*id, prop)));
                    }
                    EditOp::Clip(clip) => ops.push(Op::Layer(LayerOp::Clip(*id, clip))),
                    EditOp::Blend(blend) => ops.push(Op::Layer(LayerOp::Blend(*id, blend))),
                    EditOp::Filter(filter) => ops.push(Op::Layer(LayerOp::Filter(*id, filter))),
                    EditOp::Backdrop(backdrop) => {
                        ops.push(Op::Layer(LayerOp::Backdrop(*id, backdrop)));
                    }
                    EditOp::Content(LayerContent::Content(content)) => {
                        // A fresh `Content` replaces the previous one whole
                        // (its first `take_change` is a `Replace`).
                        let owner = shared
                            .owner
                            .get_or_insert_with(|| {
                                Rc::new(Owner(Rc::downgrade(&self.shared))) as Rc<dyn LiveOwner>
                            })
                            .clone();
                        let slot = shared.contents.entry(*id).or_default();
                        if let Some(previous) = slot.content.replace(content) {
                            slot.spare.merge(previous.retire());
                        }
                        let stored = slot.content.as_mut().expect("just inserted");
                        stored.attach(Rc::downgrade(&owner), &animated);
                        if let Some(change) = stored.take_change() {
                            let content_op = match change {
                                ContentChange::Replace(list) => ContentOp::Replace(list),
                                ContentChange::Update(updates) => ContentOp::Update(updates),
                            };
                            ops.push(Op::Layer(LayerOp::Content(*id, Some(content_op))));
                        }
                    }
                    EditOp::Content(LayerContent::Picture(picture)) => {
                        shared.contents.remove(id);
                        ops.push(Op::Layer(LayerOp::Content(
                            *id,
                            Some(ContentOp::Picture(picture)),
                        )));
                    }
                    EditOp::Content(LayerContent::Install(install)) => {
                        shared.contents.remove(id);
                        let surface = self.id;
                        let layer = *id;
                        ops.push(Op::Install(
                            layer,
                            Box::new(move |r| install(r, surface, layer)),
                        ));
                    }
                    EditOp::Content(LayerContent::None) => {
                        shared.contents.remove(id);
                        ops.push(Op::Layer(LayerOp::Content(*id, None)));
                    }
                    EditOp::Push(child) => {
                        ops.push(Op::Layer(LayerOp::Push { parent: *id, child }));
                    }
                    EditOp::Insert(index, child) => ops.push(Op::Layer(LayerOp::Insert {
                        parent: *id,
                        index,
                        child,
                    })),
                    EditOp::Detach(child) => {
                        ops.push(Op::Layer(LayerOp::Detach { parent: *id, child }));
                    }
                }
            }
        }
        shared.pending = ops;
        for (_, edit) in tx.edits.drain(..) {
            tx.edit_ops.push(edit.ops);
        }
        shared.edit_buffer = tx.edits;
        shared.edit_ops = tx.edit_ops;
        shared.queued();
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
        let _ = self.tx.send(Message::DestroySurface { id: self.id });
    }
}

impl<B: Backdrop> Surface<B> {
    /// Allocates a group id and queues its registration with `op`.
    fn new_backdrop_group(
        &self,
        op: impl FnOnce(&mut B::Renderer, SurfaceId, BackdropId) + crate::RenderTransfer + 'static,
    ) -> crate::BackdropGroup {
        let id = BackdropId::new(self.shared.borrow().next_backdrop.get());
        self.shared.borrow_mut().next_backdrop.set(id.raw() + 1);
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::HashSet;
    use std::sync::mpsc;
    use std::time::Duration;

    use kurbo::{Rect, Size};
    use nami::{SignalExt, binding};
    use nami_core::Signal;

    use crate::message::{ContentOp, LayerOp, Op};
    use crate::testing::{Null, NullConfig};
    use crate::{
        Command, Curve, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Operand, ShapeData,
        WorkingColor,
    };

    #[test]
    fn layer_record_reuses_picture_storage_every_other_frame() {
        let (events, _receiver) = mpsc::channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: HashSet::new(),
        })
        .expect("init");
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface");
        let layer = surface.layer();
        let mut pointers = Vec::new();

        for _ in 0..7 {
            surface.update(|tx| {
                tx[&layer].record(|_| {});
            });
            engine.render(FrameTime::now()).expect("render");
            let mut shared = surface.shared.borrow_mut();
            let content = shared
                .contents
                .get_mut(&layer.id())
                .expect("recorded content");
            pointers.push(std::ptr::from_ref(
                content
                    .content
                    .as_mut()
                    .expect("recorded content")
                    .snapshot(),
            ));
        }

        assert_eq!(pointers[0], pointers[2]);
        assert_eq!(pointers[1], pointers[3]);
        assert_eq!(pointers[2], pointers[4]);
        assert_eq!(pointers[3], pointers[5]);
        assert_eq!(pointers[4], pointers[6]);
        assert_ne!(pointers[0], pointers[1]);
    }

    /// The fills of `layer`'s content changes in a drained change set:
    /// whether it was a replacement, and every rectangle it carries.
    fn content_rects(
        surface: &crate::Surface<Null>,
        layer: crate::LayerId,
        time: crate::Instant,
    ) -> Vec<(bool, Rect)> {
        let Some(changes) = surface.shared.borrow_mut().take_changes(time) else {
            return Vec::new();
        };
        let mut rects = Vec::new();
        for op in changes.ops {
            match op {
                Op::Layer(LayerOp::Content(id, Some(ContentOp::Replace(picture))))
                    if id == layer =>
                {
                    for command in picture.display_list().commands() {
                        if let Command::Fill {
                            shape: ShapeData::Rect(rect),
                            ..
                        } = command
                        {
                            rects.push((true, *rect));
                        }
                    }
                }
                Op::Layer(LayerOp::Content(id, Some(ContentOp::Update(updates))))
                    if id == layer =>
                {
                    for update in updates {
                        if let Operand::Shape(ShapeData::Rect(rect)) = update.value {
                            rects.push((false, rect));
                        }
                    }
                }
                _ => {}
            }
        }
        rects
    }

    fn surface(engine: &Engine<Null>) -> crate::Surface<Null> {
        engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
            .expect("surface")
    }

    fn engine() -> Engine<Null> {
        let (events, _receiver) = mpsc::channel();
        Engine::<Null>::new(NullConfig {
            events,
            reject: HashSet::new(),
        })
        .expect("init")
    }

    #[test]
    fn a_layout_size_reaches_bound_geometry_without_rerecording() {
        let engine = engine();
        let surface = surface(&engine);
        let layer = surface.layer();
        let records = std::cell::Cell::new(0);
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
            tx[&layer].layout_size(Size::new(10.0, 20.0)).record(|c| {
                records.set(records.get() + 1);
                let size = c.layout_size();
                assert_eq!(
                    size.snapshot(),
                    Size::new(10.0, 20.0),
                    "set before recording"
                );
                c.fill(size.map(Size::to_rect), WorkingColor::WHITE);
            });
        });
        let now = crate::Instant::now();
        assert_eq!(
            content_rects(&surface, layer.id(), now),
            [(true, Rect::new(0.0, 0.0, 10.0, 20.0))]
        );

        // A constant resize updates the bound fill in place.
        surface.update(|tx| {
            tx[&layer].layout_size(Size::new(30.0, 40.0));
        });
        assert_eq!(
            content_rects(&surface, layer.id(), now),
            [(false, Rect::new(0.0, 0.0, 30.0, 40.0))]
        );

        // A bound host signal keeps it updated with no transaction.
        let host = binding(Size::new(5.0, 6.0));
        surface.update(|tx| {
            tx[&layer].layout_size(host.clone());
        });
        assert_eq!(
            content_rects(&surface, layer.id(), now),
            [(false, Rect::new(0.0, 0.0, 5.0, 6.0))]
        );
        host.set(Size::new(7.0, 8.0));
        assert_eq!(
            content_rects(&surface, layer.id(), now),
            [(false, Rect::new(0.0, 0.0, 7.0, 8.0))]
        );

        // Setting the same size again changes nothing.
        surface.update(|tx| {
            tx[&layer].layout_size(Size::new(7.0, 8.0));
        });
        assert_eq!(content_rects(&surface, layer.id(), now), Vec::new());
        assert_eq!(records.get(), 1, "no resize re-recorded the content");
    }

    #[test]
    fn an_animated_resize_animates_the_bound_operands() {
        let engine = engine();
        let surface = surface(&engine);
        surface.update(|tx| {
            tx[surface.root()].layout_size(Size::new(10.0, 10.0));
        });
        let content = surface.record(|c| {
            c.fill(c.layout_size().map(Size::to_rect), WorkingColor::WHITE);
        });
        surface.update(|tx| {
            tx[surface.root()].content(content);
        });
        let start = crate::Instant::now();
        let root = surface.root().id();
        let _ = content_rects(&surface, root, start);

        surface.update_animated(Curve::linear(Duration::from_millis(400)), |tx| {
            tx[surface.root()].layout_size(Size::new(30.0, 10.0));
        });
        assert_eq!(
            content_rects(&surface, root, start),
            [(false, Rect::new(0.0, 0.0, 10.0, 10.0))],
            "the first sample holds the start"
        );
        let mid = content_rects(&surface, root, start + Duration::from_millis(200));
        let [(false, rect)] = mid.as_slice() else {
            panic!("a mid-flight update: {mid:?}");
        };
        assert!((rect.width() - 20.0).abs() < 0.01, "half-way: {rect:?}");
        assert_eq!(
            content_rects(&surface, root, start + Duration::from_millis(400)),
            [(false, Rect::new(0.0, 0.0, 30.0, 10.0))]
        );
    }
}
