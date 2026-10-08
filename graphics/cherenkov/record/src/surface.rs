//! The queueing side of a layer tree: [`Shared`], [`Layer`] handles and
//! the [`Transaction`] edits that queue ops, none of which knows what
//! consumes them.
//!
//! `Shared` is a surface's UI-thread change queue. Ops queued outside
//! transactions (layer creates and drops, bound-signal changes) and the
//! edits a transaction records land in `pending`; the consumer drains them
//! through [`Shared::take_changes`], and every queueing call notifies the
//! target through its [`Queue`] — [`Queue::wake`] while the target drains
//! on a frame, or [`Queue::apply`] while it drains inline.

use std::cell::{Cell, RefCell};
use std::ops::{Index, IndexMut};
use std::rc::{Rc, Weak};

use kurbo::{Affine, Size, Vec2};
use nami_core::watcher::Context;
use rustc_hash::FxHashMap;

use crate::Instant;
use crate::animation::Animation;
use crate::ops::{
    AnimationStart, BackdropId, ChangeSet, ContentOp, Install, LayerId, LayerOp, Op, Prop,
    SurfaceId,
};
use crate::projective::Projective;
use crate::record::{Binding, Content, ContentSpare, Live, LiveOwner, SampleFlag};
use crate::shape::{Shape, ShapeData};
use crate::size::LayoutSize;
use crate::style::{BlendMode, FilterId};
use crate::target::{BackdropSampling, GpuInstalls, ProjectiveLayers, Queue, Target};
use crate::{BackdropSample, ContentChange, Picture, WorkingColor};

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

/// State shared between a target's surface handle, its [`Layer`] handles
/// and the consumer that drains it.
///
/// Everything here is queued, not sent: the consumer drains `pending` into
/// a change set when it wants one. While the queue's [`Target::Queue`]
/// answers [`Queue::drains_inline`], every change instead drains as it is
/// made and the result goes to the queue's [`Queue::apply`].
pub struct Shared<T: Target> {
    /// The surface's identifier with the consumer.
    pub id: SurfaceId,
    /// Ops queued outside transactions (layer creates and drops, bound
    /// signal changes) plus queued transaction ops.
    pending: Vec<Op<T>>,
    /// Reusable change-set op buffer.
    spare_ops: Vec<Op<T>>,
    /// Reusable change-set recycled-picture buffer.
    spare_recycled: Vec<(LayerId, Picture)>,
    /// Reusable transaction edit buffers.
    edit_buffer: Vec<(LayerId, LayerEdit<T>)>,
    edit_ops: Vec<Vec<EditOp<T>>>,
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
    /// The target's queue endpoint, notified when changes arrive.
    queue: T::Queue,
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

impl<T: Target> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("id", &self.id)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl<T: Target> Shared<T> {
    /// A shared queue identified by `id`, notifying `queue` when changes
    /// arrive.
    #[must_use]
    pub fn new(id: SurfaceId, queue: T::Queue) -> Self {
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
            queue,
            animated: SampleFlag::new(),
            owner: None,
        }
    }

    /// The root layer `this` surface owns: layer id 0, the layer whose
    /// handle dropping queues no `Remove`.
    #[must_use]
    pub fn root(this: &Rc<RefCell<Self>>) -> Layer {
        let owner = Rc::clone(this) as Rc<dyn LayerOwner>;
        Layer::new(LayerId::new(0), owner, false)
    }

    /// A layer `this` surface owns: allocated now — its `Create` is
    /// queued — and removed when the handle drops.
    #[must_use]
    pub fn layer(this: &Rc<RefCell<Self>>) -> Layer {
        let owner = Rc::clone(this) as Rc<dyn LayerOwner>;
        Layer::new(owner.allocate(), owner, true)
    }

    /// `layer`'s layout size: the entry is created on first read so a
    /// recording can bind it.
    pub fn layout_size(&mut self, layer: LayerId) -> LayoutSize {
        self.sizes.entry(layer).or_default().clone()
    }

    /// Queues the clear colour into the pending change set.
    pub fn set_clear(&mut self, color: WorkingColor) {
        self.clear = Some(color);
        self.flush();
    }

    /// Allocates a backdrop group id.
    pub fn allocate_backdrop(&mut self) -> BackdropId {
        let id = BackdropId::new(self.next_backdrop.get());
        self.next_backdrop.set(id.raw() + 1);
        id
    }

    /// Queues an op outside a transaction.
    fn push(&mut self, op: Op<T>) {
        self.pending.push(op);
        self.flush();
    }

    /// Something was queued. A queue that drains inline gets the drained
    /// change set through [`Queue::apply`]; any other is woken for its own
    /// drain through [`Queue::wake`]. The drain runs before the hook, so
    /// the hook never needs to borrow `self`.
    ///
    /// The owner of a `Shared` calls this to drain through the current
    /// mode whatever is queued — for an engine's surface, when hiding
    /// applies the backlog at once.
    pub fn flush(&mut self) {
        if self.queue.drains_inline() {
            if let Some(changes) = self.drain(None) {
                self.queue.apply(changes);
            }
        } else {
            self.queue.wake();
        }
    }

    /// Samples running operand animations at `time`, then drains pending
    /// ops and live content changes into a change set. Returns `None` when
    /// nothing changed.
    pub fn take_changes(&mut self, time: crate::Instant) -> Option<ChangeSet<T>> {
        self.drain(Some(time))
    }

    /// Drains pending ops and live content changes into a change set,
    /// sampling running operand animations first when a frame `time` is
    /// given. Returns `None` when nothing changed.
    fn drain(&mut self, time: Option<crate::Instant>) -> Option<ChangeSet<T>> {
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

    /// Returns the buffers a drained change set was built from: the op
    /// vector becomes the next one's spare, and each returned picture
    /// becomes its layer's reusable recording storage.
    pub fn recycle(&mut self, ops: Vec<Op<T>>, recycled: &mut Vec<(LayerId, Picture)>) {
        self.spare_ops = ops;
        for (layer, picture) in recycled.drain(..) {
            if let Some(slot) = self.contents.get_mut(&layer) {
                slot.spare.put_picture(picture);
            }
        }
        self.spare_recycled = std::mem::take(recycled);
    }

    /// Queues a transaction's edits into the pending change set. Ops
    /// queued between transactions (layer creates, drops, bound-signal
    /// changes) come first.
    ///
    /// `animation` fills every animatable op that lacks one; `start` is
    /// the instant those animations begin at on the host's clock — `None`
    /// keeps first-sampled-frame starts.
    ///
    /// # Panics
    /// Panics if `body` panics; the transaction is then dropped unapplied.
    #[expect(clippy::too_many_lines, reason = "one edit-op dispatch per design")]
    pub fn run_transaction(
        this: &Rc<RefCell<Self>>,
        animation: Option<Animation>,
        start: Option<Instant>,
        body: impl FnOnce(&mut Transaction<'_, T>),
    ) {
        let (edits, edit_ops) = {
            let mut shared = this.borrow_mut();
            (
                std::mem::take(&mut shared.edit_buffer),
                std::mem::take(&mut shared.edit_ops),
            )
        };
        let mut tx = Transaction {
            edits,
            edit_ops,
            shared: this,
            animation,
            start,
        };
        body(&mut tx);
        let mut shared = this.borrow_mut();
        // Ops queued between transactions (layer creates, drops,
        // bound-signal changes) come first.
        let mut ops = std::mem::take(&mut shared.pending);
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
                                Rc::new(Owner(Rc::downgrade(this))) as Rc<dyn LiveOwner>
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
                        ops.push(Op::Install(*id, install));
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
        shared.flush();
    }

    /// Binds `live` so its later changes queue `op(layer, value, animation,
    /// start)` and notify the queue, returning the value the binding starts
    /// from. Replaces the property's previous binding.
    fn bind<V, F>(
        shared: &Rc<RefCell<Self>>,
        layer: LayerId,
        kind: PropKind,
        live: Live<V>,
        op: F,
    ) -> V
    where
        V: 'static,
        F: Fn(LayerId, V, Option<Animation>, Option<Instant>) -> LayerOp + 'static,
    {
        let weak = Rc::downgrade(shared);
        let (target, guard) = live.watch(move |context: Context<V>| {
            let animation = context.metadata().try_get::<Animation>();
            let start = context.metadata().try_get::<AnimationStart>();
            let target = context.into_value();
            if let Some(shared) = weak.upgrade() {
                shared.borrow_mut().push(Op::Layer(op(
                    layer,
                    target,
                    animation,
                    start.map(|s| s.0),
                )));
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
    /// Queues a `Remove` for `id` — only `id` — and drops the layer's
    /// bindings and contents.
    fn remove(&self, id: LayerId);
}

/// The surface as an installed content's live-operand owner: a
/// signal change queues the surface. It weakly holds the surface's queue,
/// which holds the one [`Rc`] of it, so neither keeps the other alive.
struct Owner<T: Target>(Weak<RefCell<Shared<T>>>);

impl<T: Target> LiveOwner for Owner<T> {
    fn changed(&self) {
        if let Some(shared) = self.0.upgrade() {
            shared.borrow_mut().flush();
        }
    }
}

impl<T: Target> LayerOwner for RefCell<Shared<T>> {
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

/// A layer handle.
///
/// Layers are `!Send` and not `Clone`; dropping one removes it — only it —
/// from its surface at the next commit. Its children stay in the tree,
/// detached and undrawn, until their own handles drop or they are
/// re-attached.
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
    /// A layer handle owned by `owner`. With `remove_on_drop` the drop
    /// queues the layer's `Remove`; without it the layer survives the
    /// handle (a tree's root is so owned by its surface). `pub(crate)`:
    /// handles come from [`Shared::root`] and [`Shared::layer`] — a public
    /// constructor could forge one, including a root that drops a
    /// `Remove`.
    #[must_use]
    pub(crate) const fn new(id: LayerId, owner: Rc<dyn LayerOwner>, remove_on_drop: bool) -> Self {
        Self {
            id,
            owner,
            remove_on_drop,
        }
    }

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
pub enum LayerContent<T: Target> {
    /// Live recorded content; its bound signals keep updating it with no
    /// further transactions.
    Content(Content),
    /// A shared immutable picture.
    Picture(Picture),
    /// An opaque render-side install (GPU producers). The payload learns
    /// the surface and layer it is installed on when the edit is applied,
    /// and reports the installed content's declared alpha — `None` before
    /// the producer's first frame — which the layer's alpha contract
    /// notes. [`Install`] is sealed: only [`install`](Self::install),
    /// gated on [`GpuInstalls`], wraps one.
    Install(Install<T>),
    /// Nothing.
    None,
}

impl<T: GpuInstalls> LayerContent<T> {
    /// An opaque render-side install, carried as `Install` in the change
    /// set. The payload is the target's own — an engine backend's install
    /// closure — sealed in [`Install`], and only a target implementing
    /// [`GpuInstalls`] can produce one.
    #[must_use]
    pub const fn install(install: T::Install) -> Self {
        Self::Install(Install::new(install))
    }
}

impl<T: Target> std::fmt::Debug for LayerContent<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Install` carries an opaque render-side payload.
        match self {
            Self::Content(content) => f.debug_tuple("Content").field(content).finish(),
            Self::Picture(picture) => f.debug_tuple("Picture").field(picture).finish(),
            Self::Install(_) => f.write_str("Install(..)"),
            Self::None => f.write_str("None"),
        }
    }
}

impl<T: Target> From<Content> for LayerContent<T> {
    fn from(content: Content) -> Self {
        Self::Content(content)
    }
}

impl<T: Target> From<Picture> for LayerContent<T> {
    fn from(picture: Picture) -> Self {
        Self::Picture(picture)
    }
}

/// A recorded layer edit inside a [`Transaction`].
enum EditOp<T: Target> {
    Transform(Prop<Affine>),
    Translation(Prop<Vec2>),
    Rotation(Prop<f64>),
    Scale(Prop<Vec2>),
    Skew(Prop<Vec2>),
    Pivot(Prop<Vec2>),
    Projection(Projective),
    Tilt(Prop<Vec2>),
    Depth(Prop<f64>),
    ClearProjection,

    Opacity(Prop<f32>),
    ScrollOffset(Prop<Vec2>),
    Clip(Option<ShapeData>),
    Blend(BlendMode),
    Filter(Option<FilterId>),
    Backdrop(Option<BackdropSample>),
    Content(LayerContent<T>),
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
/// metadata carries an [`Animation`] interpolates while the consumer
/// samples it — an [`AnimationStart`] next to it starts the track at that
/// instant instead of the first frame that samples it.
pub struct LayerEdit<T: Target> {
    ops: Vec<EditOp<T>>,
    layer: LayerId,
    shared: Rc<RefCell<Shared<T>>>,
    /// The transaction-wide animation, filled for animatable ops that lack
    /// one.
    default_animation: Option<Animation>,
    /// The transaction-wide animation start, on the host's clock.
    default_start: Option<Instant>,
}

impl<T: Target> std::fmt::Debug for LayerEdit<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerEdit")
            .field("layer", &self.layer)
            .field("default_animation", &self.default_animation)
            .field("default_start", &self.default_start)
            .finish_non_exhaustive()
    }
}

impl<T: ProjectiveLayers> LayerEdit<T> {
    /// Makes the layer projective with `value` as its projection base.
    ///
    /// A projective layer is a flattening boundary: its content, clip,
    /// scroll offset, filter and children render into a layer-local image
    /// with the ordinary affine rasterizers, and that image is projected
    /// when it composes into its parent, where its opacity and blend apply
    /// once. The complete pose is `transform · translate(translation +
    /// pivot) · projection · translate_z(depth) · rotate_z(rotation) ·
    /// rotate_y(tilt.y) · rotate_x(tilt.x) · skew · scale ·
    /// translate(−pivot)`. A projective layer's local image is bounded by
    /// its clip.
    ///
    /// The raw matrix is not animatable: a new value, bound or set,
    /// replaces the base, and `.animation(...)` after it panics. Animate
    /// flips with [`Self::tilt`], [`Self::depth`] and the affine
    /// components.
    pub fn projection(&mut self, value: impl Into<Live<Projective>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Projection,
            value.into(),
            |layer, target, _, _| LayerOp::Projection(layer, target),
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
            |layer, target, animation, start| {
                LayerOp::Tilt(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Tilt(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
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
            |layer, target, animation, start| {
                LayerOp::Depth(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Depth(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
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

impl<T: BackdropSampling> LayerEdit<T> {
    /// Makes the layer a member of a backdrop group: it composites the
    /// group's capture as the bottom-most draw inside its clip. A sample
    /// made with [`BackdropSample::with_effect`] carries a per-member
    /// effect evaluated in the member's composite.
    pub fn backdrop(&mut self, sample: BackdropSample) -> &mut Self {
        self.ops.push(EditOp::Backdrop(Some(sample)));
        self
    }

    /// Clears the layer's backdrop group membership.
    pub fn clear_backdrop(&mut self) -> &mut Self {
        self.ops.push(EditOp::Backdrop(None));
        self
    }
}

impl<T: Target> LayerEdit<T> {
    /// Sets the local transform.
    pub fn transform(&mut self, transform: impl Into<Live<Affine>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Transform,
            transform.into(),
            |layer, target, animation, start| {
                LayerOp::Transform(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Transform(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
        }));
        self
    }

    /// Sets the translation in local coordinates.
    /// Component properties compose after [`Self::transform`]. Each keeps
    /// its own live subscription and animation track; changing it never
    /// re-records content.
    pub fn translation(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Translation,
            value.into(),
            |layer, target, animation, start| {
                LayerOp::Translation(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Translation(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
        }));
        self
    }

    /// Sets the unwrapped rotation angle in radians; initially zero.
    /// Positive angles rotate clockwise in a downward-y coordinate system.
    /// Full turns are preserved; no shortest-path angle normalization occurs.
    /// Component properties compose after [`Self::transform`]. Each keeps
    /// its own live subscription and animation track; changing it never
    /// re-records content.
    pub fn rotation(&mut self, value: impl Into<Live<f64>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Rotation,
            value.into(),
            |layer, target, animation, start| {
                LayerOp::Rotation(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Rotation(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
        }));
        self
    }

    /// Sets the x/y scale factors; initially (1, 1).
    /// Component properties compose after [`Self::transform`]. Each keeps
    /// its own live subscription and animation track; changing it never
    /// re-records content.
    pub fn scale(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Scale,
            value.into(),
            |layer, target, animation, start| {
                LayerOp::Scale(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Scale(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
        }));
        self
    }

    /// Sets x/y skew angles in radians; initially zero.
    /// The skew matrix is `[1, tan(y), tan(x), 1, 0, 0]`.
    /// Component properties compose after [`Self::transform`]. Each keeps
    /// its own live subscription and animation track; changing it never
    /// re-records content.
    pub fn skew(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Skew,
            value.into(),
            |layer, target, animation, start| {
                LayerOp::Skew(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Skew(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
        }));
        self
    }

    /// Sets the local pivot for rotation, skew and scale; initially zero.
    /// Component properties compose after [`Self::transform`]. Each keeps
    /// its own live subscription and animation track; changing it never
    /// re-records content.
    pub fn pivot(&mut self, value: impl Into<Live<Vec2>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Pivot,
            value.into(),
            |layer, target, animation, start| {
                LayerOp::Pivot(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Pivot(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
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
            |layer, target, animation, start| {
                LayerOp::Opacity(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::Opacity(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
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
            |layer, target, animation, start| {
                LayerOp::ScrollOffset(
                    layer,
                    Prop {
                        target,
                        animation,
                        start,
                    },
                )
            },
        );
        self.ops.push(EditOp::ScrollOffset(Prop {
            target,
            animation: self.default_animation,
            start: self.default_start,
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
            |layer, shape: S, _, _| LayerOp::Clip(layer, Some(ShapeData::of(&shape))),
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

    /// Sets the filter applied to this layer's subtree, as its
    /// consumer-registered id.
    pub fn filter(&mut self, filter: FilterId) -> &mut Self {
        self.ops.push(EditOp::Filter(Some(filter)));
        self
    }

    /// Clears the layer's filter.
    pub fn clear_filter(&mut self) -> &mut Self {
        self.ops.push(EditOp::Filter(None));
        self
    }

    /// Sets the content.
    pub fn content(&mut self, content: impl Into<LayerContent<T>>) -> &mut Self {
        self.ops.push(EditOp::Content(content.into()));
        self
    }

    /// Records content, reusing picture storage the consumer returned.
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
    /// it animate under the transaction's animation or a bound change's
    /// `Animation` metadata, like any animated operand; it is not a
    /// consumer-side property, so [`animation`](Self::animation) does not
    /// apply to it.
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
pub struct Transaction<'a, T: Target> {
    edits: Vec<(LayerId, LayerEdit<T>)>,
    edit_ops: Vec<Vec<EditOp<T>>>,
    shared: &'a Rc<RefCell<Shared<T>>>,
    /// The transaction-wide animation.
    animation: Option<Animation>,
    /// The instant the transaction's animations start at, on the host's
    /// clock; `None` starts each track at the first frame that samples it.
    start: Option<Instant>,
}

impl<T: Target> std::fmt::Debug for Transaction<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction")
            .field("animation", &self.animation)
            .field("start", &self.start)
            .finish_non_exhaustive()
    }
}

impl<T: Target> Transaction<'_, T> {
    fn edit(&mut self, layer: &Layer) -> &mut LayerEdit<T> {
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
                    default_start: self.start,
                },
            ));
            &mut self.edits.last_mut().expect("just pushed").1
        }
    }
}

impl<T: Target> Index<&Layer> for Transaction<'_, T> {
    type Output = LayerEdit<T>;

    fn index(&self, layer: &Layer) -> &Self::Output {
        self.edits
            .iter()
            .find(|(id, _)| *id == layer.id)
            .map(|(_, edit)| edit)
            .expect("the layer has no edits in this transaction yet")
    }
}

impl<T: Target> IndexMut<&Layer> for Transaction<'_, T> {
    fn index_mut(&mut self, layer: &Layer) -> &mut Self::Output {
        self.edit(layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::Command;
    use crate::{Curve, Draw, Operand, ShapeData, SurfaceId, WorkingColor};
    use kurbo::{Rect, Size};
    use nami::{SignalExt, binding};
    use nami_core::Signal;
    use std::time::Duration;

    /// A target for queue tests: always visible (never drains inline),
    /// installs nothing.
    struct TestTarget;

    impl Target for TestTarget {
        type Queue = TestQueue;
        type Install = ();
    }

    /// The queue of a target that is never hidden and never asked to
    /// drain: `wake` is a no-op, `apply` never fires.
    struct TestQueue;

    impl Queue<TestTarget> for TestQueue {
        fn drains_inline(&self) -> bool {
            false
        }

        fn apply(&self, _changes: ChangeSet<TestTarget>) {
            unreachable!("a visible target never drains inline");
        }

        fn wake(&self) {}
    }

    fn shared() -> Rc<RefCell<Shared<TestTarget>>> {
        Rc::new(RefCell::new(Shared::new(SurfaceId::new(1), TestQueue)))
    }

    fn layer(shared: &Rc<RefCell<Shared<TestTarget>>>) -> Layer {
        let owner: Rc<dyn LayerOwner> = Rc::clone(shared) as Rc<dyn LayerOwner>;
        Layer::new(owner.allocate(), owner, true)
    }

    #[test]
    fn layer_record_reuses_picture_storage_every_other_frame() {
        let shared = shared();
        let layer = layer(&shared);
        // The consumer keeps each commit's new picture and hands the
        // replaced ones back, like a render thread does.
        let mut kept: std::collections::HashMap<LayerId, Picture> =
            std::collections::HashMap::new();
        let mut pointers = Vec::new();

        for _ in 0..7 {
            Shared::run_transaction(&shared, None, None, |tx| {
                tx[&layer].record(|_| {});
            });
            let Some(changes) = shared.borrow_mut().take_changes(crate::Instant::now()) else {
                panic!("a re-record always changes");
            };
            let mut recycled = changes.recycled;
            let mut ops = changes.ops;
            for op in &mut ops {
                if let Op::Layer(LayerOp::Content(id, Some(content))) = op
                    && let ContentOp::Replace(picture) =
                        std::mem::replace(content, ContentOp::Update(Vec::new()))
                {
                    // The consumer keeps the new picture; a replaced one
                    // it drops, recycling the storage when it was the
                    // last reference — like a layer node does.
                    if let Some(mut old) = kept.insert(*id, picture)
                        && old.try_recycle()
                    {
                        recycled.push((*id, old));
                    }
                }
            }
            shared.borrow_mut().recycle(ops, &mut recycled);
            let mut shared_mut = shared.borrow_mut();
            let slot = shared_mut
                .contents
                .get_mut(&layer.id())
                .expect("recorded content");
            pointers.push(std::ptr::from_ref(
                slot.content.as_mut().expect("recorded content").snapshot(),
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
        shared: &Rc<RefCell<Shared<TestTarget>>>,
        layer: LayerId,
        time: crate::Instant,
    ) -> Vec<(bool, Rect)> {
        let Some(changes) = shared.borrow_mut().take_changes(time) else {
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

    #[test]
    fn a_layout_size_reaches_bound_geometry_without_rerecording() {
        let shared = shared();
        let root = {
            let owner: Rc<dyn LayerOwner> = Rc::clone(&shared) as Rc<dyn LayerOwner>;
            Layer::new(LayerId::new(0), owner, false)
        };
        let layer = layer(&shared);
        let records = std::cell::Cell::new(0);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&root].push(&layer);
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
            content_rects(&shared, layer.id(), now),
            [(true, Rect::new(0.0, 0.0, 10.0, 20.0))]
        );

        // A constant resize updates the bound fill in place.
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].layout_size(Size::new(30.0, 40.0));
        });
        assert_eq!(
            content_rects(&shared, layer.id(), now),
            [(false, Rect::new(0.0, 0.0, 30.0, 40.0))]
        );

        // A bound host signal keeps it updated with no transaction.
        let host = binding(Size::new(5.0, 6.0));
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].layout_size(host.clone());
        });
        assert_eq!(
            content_rects(&shared, layer.id(), now),
            [(false, Rect::new(0.0, 0.0, 5.0, 6.0))]
        );
        host.set(Size::new(7.0, 8.0));
        assert_eq!(
            content_rects(&shared, layer.id(), now),
            [(false, Rect::new(0.0, 0.0, 7.0, 8.0))]
        );

        // Setting the same size again changes nothing.
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].layout_size(Size::new(7.0, 8.0));
        });
        assert_eq!(content_rects(&shared, layer.id(), now), Vec::new());
        assert_eq!(records.get(), 1, "no resize re-recorded the content");
    }

    #[test]
    fn an_animated_resize_animates_the_bound_operands() {
        let shared = shared();
        let root = {
            let owner: Rc<dyn LayerOwner> = Rc::clone(&shared) as Rc<dyn LayerOwner>;
            Layer::new(LayerId::new(0), owner, false)
        };
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&root].layout_size(Size::new(10.0, 10.0));
        });
        let content = {
            let size = shared.borrow_mut().layout_size(root.id());
            Content::record(&size, |c| {
                c.fill(c.layout_size().map(Size::to_rect), WorkingColor::WHITE);
            })
        };
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&root].content(content);
        });
        let start = crate::Instant::now();
        let _ = content_rects(&shared, root.id(), start);

        Shared::run_transaction(
            &shared,
            Some(Curve::linear(Duration::from_millis(400)).into()),
            None,
            |tx| {
                tx[&root].layout_size(Size::new(30.0, 10.0));
            },
        );
        assert_eq!(
            content_rects(&shared, root.id(), start),
            [(false, Rect::new(0.0, 0.0, 10.0, 10.0))],
            "the first sample holds the start"
        );
        let mid = content_rects(&shared, root.id(), start + Duration::from_millis(200));
        let [(false, rect)] = mid.as_slice() else {
            panic!("a mid-flight update: {mid:?}");
        };
        assert!((rect.width() - 20.0).abs() < 0.01, "half-way: {rect:?}");
        assert_eq!(
            content_rects(&shared, root.id(), start + Duration::from_millis(400)),
            [(false, Rect::new(0.0, 0.0, 30.0, 10.0))]
        );
    }

    #[test]
    fn a_transaction_start_reaches_animated_props() {
        let shared = shared();
        let layer = layer(&shared);
        let start = crate::Instant::now();
        Shared::run_transaction(
            &shared,
            Some(Curve::linear(Duration::from_millis(400)).into()),
            Some(start),
            |tx| {
                tx[&layer].opacity(0.5_f32);
                tx[&layer].scale(Vec2::new(2.0, 2.0));
            },
        );
        let Some(changes) = shared.borrow_mut().take_changes(crate::Instant::now()) else {
            panic!("the transaction queued edits");
        };
        let mut seen = 0;
        for op in &changes.ops {
            match op {
                Op::Layer(LayerOp::Opacity(_, prop)) => {
                    assert_eq!(prop.start, Some(start));
                    seen += 1;
                }
                Op::Layer(LayerOp::Scale(_, prop)) => {
                    assert_eq!(prop.start, Some(start));
                    seen += 1;
                }
                _ => {}
            }
        }
        assert_eq!(seen, 2);
    }

    #[test]
    fn a_bound_change_reads_its_animation_start_from_metadata() {
        let shared = shared();
        let layer = layer(&shared);
        let start = crate::Instant::now();
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(
                opacity
                    .clone()
                    .with(Animation::from(Curve::linear(Duration::from_millis(400))))
                    .with(AnimationStart(start)),
            );
        });
        let _ = shared.borrow_mut().take_changes(crate::Instant::now());
        opacity.set(0.5);
        let Some(changes) = shared.borrow_mut().take_changes(crate::Instant::now()) else {
            panic!("the bound change queued an op");
        };
        let [op] = changes.ops.as_slice() else {
            panic!("one opacity op: {:?}", changes.ops);
        };
        let Op::Layer(LayerOp::Opacity(_, prop)) = op else {
            panic!("an opacity op: {op:?}");
        };
        assert_eq!(prop.start, Some(start));
        assert!(matches!(prop.animation, Some(Animation::Curve(_))));
    }
}
