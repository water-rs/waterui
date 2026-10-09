//! The queueing side of a layer tree: [`Shared`], [`Layer`] handles and
//! the [`Transaction`] edits that queue ops, none of which knows what
//! consumes them.
//!
//! `Shared` is a surface's UI-thread change queue. Ops queued outside
//! transactions (layer creates and drops, bound-signal changes) and the
//! edits a transaction records land in `pending`; the consumer drains them
//! through [`Shared::take_changes`], and every queueing call notifies the
//! target through its [`Queue`] — [`Queue::wake`] while the target drains
//! on a frame, or [`Queue::apply`] while it drains inline. While a
//! transaction is open a bound signal's change queues in `deferred`
//! instead, resolved once at the transaction's outermost end — its
//! commit, or its unwind.

use std::cell::{Cell, RefCell};
use std::ops::{Index, IndexMut};
use std::rc::{Rc, Weak};

use kurbo::{Affine, Size, Vec2};
use nami_core::watcher::Context;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::Instant;
use crate::animation::Animation;
use crate::ops::{
    AnimationStart, BackdropId, ChangeSet, ContentOp, Install, LayerId, LayerOp, Op, Prop,
    SurfaceId,
};
use crate::projective::Projective;
use crate::record::{Binding, Content, ContentSpare, Live, LiveOwner, SampleFlag};
use crate::shape::Shape;
use crate::shape::ShapeData;
use crate::size::LayoutSize;
use crate::style::{BlendMode, FilterId};
use crate::target::{BackdropSampling, GpuInstalls, ProjectiveLayers, Queue, Target};
use crate::{BackdropSample, ContentChange, Picture, WorkingColor};

/// Declares [`PropKind`] and its `ALL` list from one variant list, so
/// the list a layer removal walks can never miss a kind.
macro_rules! prop_kinds {
    ($($kind:ident),+ $(,)?) => {
        /// Which bound property a subscription updates.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        enum PropKind {
            $($kind),+
        }

        impl PropKind {
            /// Every kind, in declaration order — its bit order in a
            /// [`KindSet`].
            const ALL: [Self; [$(stringify!($kind)),+].len()] = [$(Self::$kind),+];
        }
    };
}

prop_kinds![
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
    Backdrop,
    LayoutSize,
];

const _: () = assert!(
    PropKind::ALL.len() <= KindSet::BITS as usize,
    "every PropKind needs a bit in a KindSet"
);

impl PropKind {
    /// The kind's bit in a [`KindSet`].
    const fn bit(self) -> KindSet {
        1 << self as u16
    }
}

/// A set of [`PropKind`]s, one bit per kind.
type KindSet = u16;

/// A surface's kept property subscriptions, indexed by layer.
///
/// `kept` holds each binding under its `(layer, kind)` key; `kinds` holds,
/// for every layer with at least one kept binding, the set of its bound
/// kinds — so dropping a layer reaches only that layer's entries instead
/// of scanning every binding on the surface.
#[derive(Default)]
struct Bindings {
    kept: FxHashMap<(u64, PropKind), KeptBinding>,
    kinds: FxHashMap<u64, KindSet>,
}

impl Bindings {
    /// The binding kept for `layer`'s `kind`.
    fn get(&self, layer: u64, kind: PropKind) -> Option<&KeptBinding> {
        self.kept.get(&(layer, kind))
    }

    /// Keeps `binding` for `layer`'s `kind`, returning the one it
    /// replaces.
    fn insert(&mut self, layer: u64, kind: PropKind, binding: KeptBinding) -> Option<KeptBinding> {
        *self.kinds.entry(layer).or_default() |= kind.bit();
        self.kept.insert((layer, kind), binding)
    }

    /// Removes the binding kept for `layer`'s `kind`, if any. A layer
    /// with nothing bound costs one index probe.
    fn remove(&mut self, layer: u64, kind: PropKind) -> Option<KeptBinding> {
        let std::collections::hash_map::Entry::Occupied(mut kinds) = self.kinds.entry(layer) else {
            return None;
        };
        if *kinds.get() & kind.bit() == 0 {
            return None;
        }
        *kinds.get_mut() &= !kind.bit();
        if *kinds.get() == 0 {
            kinds.remove();
        }
        self.kept.remove(&(layer, kind))
    }

    /// Removes every binding kept for `layer`, in a fixed array so the
    /// caller can drop them outside its borrow without allocating.
    fn remove_layer(&mut self, layer: u64) -> [Option<KeptBinding>; PropKind::ALL.len()] {
        let mut removed = [const { None }; PropKind::ALL.len()];
        if let Some(kinds) = self.kinds.remove(&layer) {
            for (slot, kind) in removed.iter_mut().zip(PropKind::ALL) {
                if kinds & kind.bit() != 0 {
                    *slot = self.kept.remove(&(layer, kind));
                }
            }
        }
        removed
    }
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
    /// Ops queued directly — layer creates and drops, bound-signal
    /// changes outside a transaction — plus committed transaction edits
    /// and the deferred changes the transaction's outermost end kept.
    pending: Vec<Op<T>>,
    /// Reusable change-set op buffer.
    spare_ops: Vec<Op<T>>,
    /// Reusable change-set recycled-picture buffer.
    spare_recycled: Vec<(LayerId, Picture)>,
    /// The open transaction's edit stream sits in `pending` past
    /// `stream_start`, in program order: every edit queued while a
    /// transaction is open lands there, and only the outermost commit
    /// applies it, so the last write wins. A nested transaction's edits
    /// join the same stream — interleaved with the outer's in program
    /// order, each carrying its own transaction's animation. Ops the
    /// body queues directly — layer creates and drops — are not stream
    /// edits: they wait in `direct_ops` and the outermost commit splices
    /// them ahead of the stream after the scan.
    /// A content edit enters a `Content(.., None)` placeholder in the
    /// stream: its `LayerContent` waits in `edit_content`, in the same
    /// order, and the commit drains one per placeholder to run the
    /// contents-map bookkeeping a queue-time op cannot.
    stream_start: usize,
    /// The sequence stamp of each `pending` entry past `stream_start`,
    /// drawn from `next_edit_seq` by [`Shared::push`], so a retarget can
    /// tell the entry it recorded from one that later took its index.
    /// `edit_seqs[i]` pairs with `pending[stream_start + i]`.
    edit_seqs: Vec<u64>,
    /// The `LayerContent` payloads of the content edits in the stream,
    /// in the same order, drained one per `Content(.., None)`
    /// placeholder at the commit.
    edit_content: Vec<LayerContent<T>>,
    /// Ops the open transaction queued that are not edits — layer
    /// creates and removes go here, not into the stream: they sit
    /// ahead of the stream at the commit, and an unwind keeps them —
    /// the handles they belong to outlive the transaction, so the ops
    /// must reach the tree. Only stream edits are ever truncated.
    direct_ops: Vec<Op<T>>,
    /// The sequence counter stream entries are stamped from.
    next_edit_seq: u64,
    /// Layers removed while a transaction is open: their `Remove` already
    /// queued, so the outermost commit discards the stream's edits to
    /// them. Cleared when the outermost transaction ends.
    removed_layers: FxHashSet<LayerId>,
    /// Content and reusable storage per layer.
    contents: FxHashMap<LayerId, ContentSlot>,
    /// Each layer's layout size, created when first set or recorded for.
    sizes: FxHashMap<LayerId, LayoutSize>,
    /// Pending clear colour.
    clear: Option<WorkingColor>,
    /// The vectors an outermost commit moves its replaced and removed
    /// contents out of the surface borrow in, handed back after their
    /// drops run: reused across commits so a steady-state commit
    /// allocates nothing.
    spare_replaced: Vec<(LayerId, Content)>,
    spare_removed: Vec<ContentSlot>,
    /// Layer id allocator (0 is the root).
    next_layer: Cell<u64>,
    /// Backdrop group id allocator.
    next_backdrop: Cell<u64>,
    /// Live property subscriptions, keyed by layer and property, each
    /// carrying the generation [`bind`](Self::bind) drew for it. Binding a
    /// property replaces its previous subscription — ending that
    /// generation — and dropping a layer drops them all.
    bindings: Bindings,
    /// The generation counter a kept binding draws from.
    next_generation: u64,
    /// The generation of the last bind whose watcher was dropped: each
    /// watcher carries a [`WatcherMark`] that writes its generation here
    /// when the watcher drops, so a bind learns whether its own signal
    /// kept the watcher it was handed.
    dropped_watcher: Rc<Cell<u64>>,
    /// A transaction is open: [`run_transaction`](Self::run_transaction)
    /// sets it on entry, the outermost commit clears it on the normal
    /// path, and the [`Open`] guard restores it on unwind.
    transaction_open: bool,
    /// Signal changes a binding's watcher queued while a transaction was
    /// open, in fire order: the transaction's outermost end — its commit,
    /// or an unwind reaching it — lands each whose binding is still the
    /// property's current one and discards the rest. Reused across
    /// transactions, so a change allocates nothing.
    deferred: Vec<DeferredOp<T>>,
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
            stream_start: 0,
            edit_seqs: Vec::new(),
            edit_content: Vec::new(),
            direct_ops: Vec::new(),
            next_edit_seq: 0,
            removed_layers: FxHashSet::default(),
            contents: FxHashMap::default(),
            sizes: FxHashMap::default(),
            clear: None,
            spare_replaced: Vec::new(),
            spare_removed: Vec::new(),
            next_layer: Cell::new(1),
            next_backdrop: Cell::new(1),
            bindings: Bindings::default(),
            next_generation: 1,
            dropped_watcher: Rc::new(Cell::new(0)),
            transaction_open: false,
            deferred: Vec::new(),
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

    /// Queues an op now — a layer's create or drop, a change made
    /// outside every transaction, or a transaction's edit: while a
    /// transaction is open the op joins its edit stream past
    /// `stream_start`, stamped with the next edit sequence, which is
    /// returned (0 when no transaction is open). A bound signal's
    /// change queues through [`Shared::deferred`] instead.
    fn push(&mut self, op: Op<T>) -> u64 {
        let mut seq = 0;
        if self.transaction_open {
            seq = self.next_edit_seq;
            self.next_edit_seq += 1;
            self.edit_seqs.push(seq);
        }
        self.pending.push(op);
        self.flush();
        seq
    }

    /// Queues an op that is not an edit — a layer create or remove —
    /// while a transaction is open: it buffers in [`Shared::direct_ops`]
    /// ahead of the stream instead of joining it, so the outermost
    /// commit applies it before the stream's edits and an unwind keeps
    /// it rather than truncating it. Outside a transaction it queues
    /// like any op.
    fn push_direct(&mut self, op: Op<T>) {
        if self.transaction_open {
            self.direct_ops.push(op);
        } else {
            self.push(op);
        }
    }

    /// Something was queued. A queue that drains inline gets the drained
    /// change set through [`Queue::apply`]; any other is woken for its own
    /// drain through [`Queue::wake`]. The drain runs before the hook, so
    /// the hook never needs to borrow `self`.
    ///
    /// The owner of a `Shared` calls this to drain through the current
    /// mode whatever is queued — for an engine's surface, when hiding
    /// applies the backlog at once.
    ///
    /// While a transaction is open nothing drains or wakes, even inline:
    /// the outermost commit flushes once.
    pub fn flush(&mut self) {
        if self.transaction_open {
            return;
        }
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

    /// Queues a transaction's edits into the pending change set.
    ///
    /// A transaction opened while another is open joins the outermost
    /// one: both record into the shared `pending` stream in program
    /// order, each edit carrying the animation of the transaction that
    /// recorded it, and only the outermost commit applies anything — the
    /// stream's edits first, then the changes bound signals deferred
    /// while a transaction was open, in the order they fired, each kept
    /// only while its binding is still the property's current one. The
    /// last write in program order wins. Nothing drains until then; the
    /// commit flushes once.
    ///
    /// `animation` fills every animatable op that lacks one; `start` is
    /// the instant those animations begin at on the host's clock — `None`
    /// keeps first-sampled-frame starts.
    ///
    /// # Panics
    /// Panics if `body` panics. A panicking body leaves the bindings its
    /// setters already replaced or removed replaced or removed, and the
    /// `layout_size` writes it already applied applied, but no edit of it
    /// lands: the recorded edits are truncated from the shared stream.
    /// The deferred changes stay queued for an unwinding inner
    /// transaction; an unwinding outermost one resolves them the same
    /// way its commit would — each lands only while its binding is still
    /// the property's current one. Ops the body queued directly — layer
    /// creates and drops — survive an unwind: they drained out of
    /// `direct_ops` to the consumer with the pre-open ops, so a dropped
    /// handle's `Remove` never names a layer the tree lacks.
    #[expect(clippy::too_many_lines, reason = "one edit-op dispatch per design")]
    pub fn run_transaction(
        this: &Rc<RefCell<Self>>,
        animation: Option<Animation>,
        start: Option<Instant>,
        body: impl FnOnce(&mut Transaction<'_, T>),
    ) {
        let mut open = {
            let mut shared = this.borrow_mut();
            let nested = shared.transaction_open;
            if !nested {
                // The outermost transaction's stream begins where
                // `pending` currently ends; the stamps it applies are
                // fresh.
                shared.stream_start = shared.pending.len();
                shared.edit_seqs.clear();
            }
            shared.transaction_open = true;
            Open {
                shared: this,
                nested,
                start: (
                    shared.pending.len(),
                    shared.edit_seqs.len(),
                    shared.edit_content.len(),
                ),
                committed: false,
            }
        };
        let mut tx = Transaction {
            edit: LayerEdit {
                layer: LayerId::new(0),
                shared: Rc::clone(this),
                default_animation: animation,
                default_start: start,
                last_edit: None,
            },
            lifetime: std::marker::PhantomData,
        };
        body(&mut tx);
        if open.nested {
            // An inner transaction commits nothing itself: the outermost
            // commit applies its edits.
            open.committed = true;
        } else {
            let (mut replaced, mut removed) = {
                let mut shared = this.borrow_mut();
                let (replaced, removed) = {
                    let state = &mut *shared;
                    let mut replaced = std::mem::take(&mut state.spare_replaced);
                    let mut removed = std::mem::take(&mut state.spare_removed);
                    // Cloned once per transaction: installed contents attach
                    // the surface and its sampling flag to their live states.
                    let animated = state.animated.clone();
                    // The layer set stays empty unless the transaction
                    // dropped a layer, so a steady-state commit pays one
                    // `is_empty`, not a hash probe.
                    let check_removed = !state.removed_layers.is_empty();
                    // The stream's ops already sit in `pending` past
                    // `stream_start`: a commit with no content edits and
                    // no removals touches nothing of it. Otherwise the
                    // tail is resolved in place — each `Content(.., None)`
                    // placeholder takes one queued `LayerContent`, and
                    // the entries a removed layer took with it drop.
                    if check_removed || !state.edit_content.is_empty() {
                        let start = state.stream_start;
                        let mut contents = state.edit_content.drain(..);
                        let mut drop_at: Vec<usize> = Vec::new();
                        let mut i = start;
                        while i < state.pending.len() {
                            let id = state.pending[i].layer();
                            // A layer removed while the transaction was
                            // open drops the edits that name it — its
                            // `Remove` keeps — and the tree ops that name
                            // it as the child: a `Push`/`Insert`/`Detach`
                            // records its *parent* as `layer()`, so the
                            // child's removal is checked on its own field.
                            // A content placeholder's payload goes with it.
                            let child_removed = check_removed
                                && match &state.pending[i] {
                                    Op::Layer(
                                        LayerOp::Push { child, .. }
                                        | LayerOp::Insert { child, .. }
                                        | LayerOp::Detach { child, .. },
                                    ) => state.removed_layers.contains(child),
                                    _ => false,
                                };
                            if check_removed
                                && (state.removed_layers.contains(&id) || child_removed)
                                && !matches!(state.pending[i], Op::Layer(LayerOp::Remove(..)))
                            {
                                if matches!(state.pending[i], Op::Layer(LayerOp::Content(_, None)))
                                {
                                    contents.next();
                                }
                                drop_at.push(i);
                                i += 1;
                                continue;
                            }
                            let Op::Layer(LayerOp::Content(id, None)) = state.pending[i] else {
                                i += 1;
                                continue;
                            };
                            match contents
                                .next()
                                .expect("a content op carries a queued LayerContent")
                            {
                                LayerContent::Content(content) => {
                                    // A fresh `Content` replaces the previous one
                                    // whole (its first `take_change` is a
                                    // `Replace`).
                                    let owner = state
                                        .owner
                                        .get_or_insert_with(|| {
                                            Rc::new(Owner(Rc::downgrade(this))) as Rc<dyn LiveOwner>
                                        })
                                        .clone();
                                    let slot = state.contents.entry(id).or_default();
                                    if let Some(previous) = slot.content.replace(content) {
                                        replaced.push((id, previous));
                                    }
                                    let stored = slot.content.as_mut().expect("just inserted");
                                    stored.attach(Rc::downgrade(&owner), &animated);
                                    if let Some(change) = stored.take_change() {
                                        let content_op = match change {
                                            ContentChange::Replace(list) => {
                                                ContentOp::Replace(list)
                                            }
                                            ContentChange::Update(updates) => {
                                                ContentOp::Update(updates)
                                            }
                                        };
                                        state.pending[i] =
                                            Op::Layer(LayerOp::Content(id, Some(content_op)));
                                    } else {
                                        drop_at.push(i);
                                    }
                                }
                                LayerContent::Picture(picture) => {
                                    if let Some(slot) = state.contents.remove(&id) {
                                        removed.push(slot);
                                    }
                                    state.pending[i] = Op::Layer(LayerOp::Content(
                                        id,
                                        Some(ContentOp::Picture(picture)),
                                    ));
                                }
                                LayerContent::Install(install) => {
                                    if let Some(slot) = state.contents.remove(&id) {
                                        removed.push(slot);
                                    }
                                    state.pending[i] = Op::Install(id, install);
                                }
                                LayerContent::None => {
                                    if let Some(slot) = state.contents.remove(&id) {
                                        removed.push(slot);
                                    }
                                    // The placeholder already is
                                    // `Content(id, None)` — it stays.
                                }
                            }
                            i += 1;
                        }
                        if !drop_at.is_empty() {
                            // Splice the dropped entries out of the tail.
                            let mut tail = state.pending.split_off(start);
                            let mut drops = drop_at.iter();
                            let mut next_drop = drops.next().copied();
                            let mut i = start;
                            tail.retain(|_| {
                                let keep = Some(i) != next_drop;
                                if !keep {
                                    next_drop = drops.next().copied();
                                }
                                i += 1;
                                keep
                            });
                            state.pending.append(&mut tail);
                        }
                    }
                    // The transaction's direct ops — its creates and
                    // removes — land ahead of the stream: pending is
                    // [pre-open ops, direct ops, stream edits]. They move
                    // in only after the scan above: a panic mid-scan must
                    // leave them in `direct_ops`, where the unwind keeps
                    // them, never truncated with the stream.
                    if !state.direct_ops.is_empty() {
                        let n = state.direct_ops.len();
                        let start = state.stream_start;
                        drop(
                            state
                                .pending
                                .splice(start..start, state.direct_ops.drain(..)),
                        );
                        state.stream_start += n;
                    }
                    state.edit_seqs.clear();
                    state.removed_layers.clear();
                    state.resolve_deferred();
                    state.transaction_open = false;
                    (replaced, removed)
                };
                // The open flag is restored before anything else can
                // unwind: a panic past here leaves the surface able to
                // run and commit a later transaction.
                open.committed = true;
                shared.flush();
                (replaced, removed)
            };
            // The surface borrow is released: retiring a replaced content
            // releases its subscriptions, and a dropped `Live::map`
            // closure that owns a layer of this surface re-enters it.
            #[expect(
                clippy::iter_with_drain,
                reason = "drain keeps the vector's allocation for the next commit"
            )]
            for (id, content) in replaced.drain(..) {
                let spare = content.retire();
                // The spare merges only into a slot that still exists:
                // the layer could have been removed while `retire` ran
                // user code, and re-creating its slot would leak it.
                if let Some(slot) = this.borrow_mut().contents.get_mut(&id) {
                    slot.spare.merge(spare);
                }
            }
            // Removed contents drop outside the borrow for the same
            // reason; `clear` keeps the buffer for the next commit.
            removed.clear();
            {
                let mut shared = this.borrow_mut();
                shared.spare_replaced = replaced;
                shared.spare_removed = removed;
            }
        }
    }

    /// Binds `live` so its later changes queue `op(layer, value, animation,
    /// start)` and notify the queue, returning the value the binding starts
    /// from: the signal's value now. Replaces the property's previous
    /// binding. While a transaction is open the changes queue through
    /// [`Shared::deferred`] for the outermost commit to resolve.
    #[inline]
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
        // The watcher carries the generation the binding will be kept
        // under, taken before the watch starts: it fires only while the
        // property's current binding is still the one that registered it.
        let mark = shared.borrow_mut().draw_watcher();
        let generation = mark.generation();
        let (target, guard) = live.watch(Self::watcher(shared, layer, kind, mark, op));
        let watching = Self::kept_watcher(shared, generation);
        Self::keep(shared, layer, kind, generation, watching, guard);
        target
    }

    /// Draws the generation a new binding is kept under, as the
    /// [`WatcherMark`] its watcher carries.
    fn draw_watcher(&mut self) -> WatcherMark {
        self.next_generation += 1;
        WatcherMark {
            dropped: Rc::clone(&self.dropped_watcher),
            generation: self.next_generation,
        }
    }

    /// Whether the signal kept the watcher the bind of `generation` just
    /// handed it: its [`WatcherMark`] wrote `generation` into
    /// `dropped_watcher` exactly when the signal dropped it. A signal that
    /// drops its watcher — a constant, `Fixed` or a nami constant alike —
    /// can never fire: its binding keeps no entry. This asks what the
    /// signal did with this one watcher — not what its guard's type
    /// says, and not a surface-wide count other watchers and handles the
    /// watch may create or drop also move.
    fn kept_watcher(shared: &Rc<RefCell<Self>>, generation: u64) -> bool {
        shared.borrow().dropped_watcher.get() != generation
    }

    /// The watcher a binding of `layer` subscribes to its source: each
    /// change queues `op(layer, value, animation)` — into `pending`,
    /// notifying the queue, outside a transaction; into `deferred`,
    /// stamped with the binding's generation for the outermost commit to
    /// resolve, while one is open. A change fires only while the
    /// property's current binding still holds `generation` — nami
    /// notifies from a watcher snapshot, so a binding replaced or dropped
    /// earlier in the same notify can still fire, and its stale value
    /// must not land.
    fn watcher<V, F>(
        shared: &Rc<RefCell<Self>>,
        layer: LayerId,
        kind: PropKind,
        mark: WatcherMark,
        op: F,
    ) -> impl Fn(Context<V>) + 'static
    where
        V: 'static,
        F: Fn(LayerId, V, Option<Animation>, Option<Instant>) -> LayerOp + 'static,
    {
        let weak = Rc::downgrade(shared);
        move |context: Context<V>| {
            let animation = context.metadata().try_get::<Animation>();
            let start = context.metadata().try_get::<AnimationStart>();
            let target = context.into_value();
            if let Some(shared) = weak.upgrade() {
                // `op` runs user code — a clip's `Shape::into_data` — so
                // the op is built before the surface is borrowed.
                let op = Op::Layer(op(layer, target, animation, start.map(|s| s.0)));
                let mut shared = shared.borrow_mut();
                // `mark.generation()` captures the whole mark: the watcher
                // owns it, so it drops — and records the drop — with the
                // watcher.
                let generation = mark.generation();
                if shared
                    .bindings
                    .get(layer.raw(), kind)
                    .is_none_or(|kept| kept.generation != generation)
                {
                    return;
                }
                if shared.transaction_open {
                    shared.deferred.push(DeferredOp {
                        op,
                        key: (layer.raw(), kind),
                        generation,
                    });
                } else {
                    shared.push(op);
                }
            }
        }
    }

    /// Keeps `layer`'s `kind` binding's `guard` and `generation`,
    /// replacing the property's previous one. A binding whose signal
    /// dropped its watcher — `watching` `false` — keeps no entry and only
    /// removes the previous one; a kept watcher inserts its
    /// [`KeptBinding`] under `generation` even when its watch guard is
    /// zero-sized: the guardless watch still notifies, and the entry is
    /// what lets the watcher see its change. A replaced binding's guard
    /// drops after the borrow ends: dropping it may run a `Live::map`
    /// closure that owns a layer of this surface and re-enters it.
    #[inline]
    fn keep(
        shared: &Rc<RefCell<Self>>,
        layer: LayerId,
        kind: PropKind,
        generation: u64,
        watching: bool,
        guard: Option<Binding>,
    ) {
        let replaced = {
            let mut shared_mut = shared.borrow_mut();
            if watching {
                shared_mut.bindings.insert(
                    layer.raw(),
                    kind,
                    KeptBinding {
                        _guard: guard,
                        generation,
                    },
                )
            } else {
                // Nothing to keep alive, but a previous binding is still
                // replaced by this subscription.
                shared_mut.bindings.remove(layer.raw(), kind)
            }
        };
        drop(replaced);
    }

    /// Drops the subscription bound to `layer`'s `kind`, if any. Its
    /// guard drops after the borrow ends.
    fn unbind(shared: &Rc<RefCell<Self>>, layer: LayerId, kind: PropKind) {
        let removed = shared.borrow_mut().bindings.remove(layer.raw(), kind);
        drop(removed);
    }

    /// Resolves the changes bound signals deferred while the open
    /// transaction was open: each lands in `pending`, in the order it
    /// fired, only while its binding is still the property's current
    /// one — [`Shared::bindings`] still holds its generation. The
    /// outermost end of a transaction calls this: its commit, or an
    /// unwind reaching it. `drain` keeps the buffer's allocation for
    /// the next transaction.
    fn resolve_deferred(&mut self) {
        for entry in self.deferred.drain(..) {
            if let Some(kept) = self.bindings.get(entry.key.0, entry.key.1)
                && kept.generation == entry.generation
            {
                // The op lands past `stream_start`, so it carries a
                // stamp like every stream entry.
                let seq = self.next_edit_seq;
                self.next_edit_seq += 1;
                self.edit_seqs.push(seq);
                self.pending.push(entry.op);
            }
        }
    }
}

/// A bound signal's change queued while a transaction was open. The
/// transaction's outermost end — its commit, or an unwind reaching it —
/// appends it to `pending` after the transaction's edits, in the order
/// it fired, only while [`Shared::bindings`] still holds its
/// generation: a binding replaced or removed since, or a dropped
/// layer's, discards it.
struct DeferredOp<T: Target> {
    /// The queued op.
    op: Op<T>,
    /// The binding's [`Shared::bindings`] key: `(layer.raw(), kind)`.
    key: (u64, PropKind),
    /// The generation the binding fired under.
    generation: u64,
}

/// The generation a bind drew, owned by the watcher it hands its
/// signal: dropping it — with the watcher — writes the generation into
/// the surface's `dropped_watcher`, so the bind can tell whether the
/// signal kept the watcher.
struct WatcherMark {
    dropped: Rc<Cell<u64>>,
    generation: u64,
}

impl WatcherMark {
    /// The bind's generation. Reading it through a method makes a
    /// closure capture the whole mark rather than the copied field.
    const fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for WatcherMark {
    fn drop(&mut self) {
        self.dropped.set(self.generation);
    }
}

/// A subscription kept in [`Shared::bindings`]: the guard — `None`
/// when the signal's guard was zero-sized — and the generation
/// [`Shared::bind`] drew for it before its watch started.
struct KeptBinding {
    /// The subscription guard.
    _guard: Option<Binding>,
    /// The binding's generation.
    generation: u64,
}

/// An open transaction of a surface. Dropped with the transaction, it
/// restores the surface's open state on unwind: the edits the
/// transaction recorded are truncated from the shared edit stream. The
/// deferred changes stay — an unwinding inner transaction leaves them
/// for the outermost transaction, and an unwinding outermost one
/// resolves them the way its commit would, so the layer does not
/// disagree with a signal it is still bound to. A generation can only
/// ever lose to a later write, never to an unwind. The bindings the
/// body's setters already replaced or removed stay replaced or
/// removed, and the `layout_size` writes it already applied stay
/// applied. Ops a panicking body queued directly — layer creates and
/// drops — are not stream edits, so they survive the unwind: an
/// unwinding outermost transaction lands them in `pending` ahead of the
/// deferred changes, and an inner one leaves them for the outermost.
struct Open<'a, T: Target> {
    shared: &'a RefCell<Shared<T>>,
    /// Whether a transaction was already open when this one started: an
    /// inner transaction's commit applies nothing; only the outermost
    /// commit does.
    nested: bool,
    /// The shared streams' lengths when this transaction opened —
    /// `pending`, `edit_seqs`, `edit_content`: the edits it recorded
    /// sit past them.
    start: (usize, usize, usize),
    /// The commit ran: the drop then skips its cleanup, so the normal
    /// path does not borrow `shared` twice.
    committed: bool,
}

impl<T: Target> Drop for Open<'_, T> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Reached only by unwinding a panicking body, whose own borrows
        // are gone: the borrow cannot fail.
        let mut shared = self.shared.borrow_mut();
        shared.pending.truncate(self.start.0);
        shared.edit_seqs.truncate(self.start.1);
        shared.edit_content.truncate(self.start.2);
        if !self.nested {
            // The stream edits are gone; the body's direct ops are
            // not — they were never stream entries. They land in
            // `pending` now, ahead of the deferred changes that are
            // edits.
            let state = &mut *shared;
            state.pending.append(&mut state.direct_ops);
            shared.removed_layers.clear();
            shared.resolve_deferred();
        }
        shared.transaction_open = self.nested;
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
        shared.push_direct(Op::Layer(LayerOp::Create(id)));
        id
    }

    fn remove(&self, id: LayerId) {
        // The dropped layer's bindings, size and content leave their
        // maps under the borrow and drop after it ends: dropping a guard
        // may run a `Live::map` closure that owns a layer of this
        // surface and re-enters it.
        let (removed, slot, size) = {
            let mut shared = self.borrow_mut();
            if shared.transaction_open {
                // The stream's edits to this layer are discarded at the
                // outermost commit; its `Remove` queues now.
                shared.removed_layers.insert(id);
            }
            let removed = shared.bindings.remove_layer(id.raw());
            let slot = shared.contents.remove(&id);
            let size = shared.sizes.remove(&id);
            shared.push_direct(Op::Layer(LayerOp::Remove(id)));
            (removed, slot, size)
        };
        drop((removed, slot, size));
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

/// A layer's edit handle inside a [`Transaction`]: `tx[&layer]` returns
/// it pointed at that layer. Each method queues an op into the open
/// transaction's shared edit stream and returns `&mut Self` for chaining.
///
/// `transform` and its components (`translation`, `rotation`, `scale`,
/// `skew`, `pivot`, `projection`, `tilt`, `depth`), `opacity`,
/// `scroll_offset`, `clip`, `backdrop` and `layout_size` accept a
/// constant or a nami signal (`impl Into<Live<T>>`): a bound signal keeps
/// updating the layer with no further transactions, and a change of an
/// animatable property whose nami `Context` metadata carries an
/// [`Animation`] interpolates while the consumer samples it. A signal
/// binds when its setter is called, starting from its current value —
/// or, for a signal that cannot fire, from the value the `Live` held
/// when it was made, which differs only when an impure map ran over a
/// constant. The last write in program order wins: a bound signal's
/// change made while a transaction is open lands at the outermost
/// commit, after the transaction's edits, only if the binding is still
/// the property's current one. An [`AnimationStart`] next to it starts
/// the track at that instant instead of the first frame that samples it.
pub struct LayerEdit<T: Target> {
    /// The layer the handle currently points at.
    layer: LayerId,
    shared: Rc<RefCell<Shared<T>>>,
    /// The transaction-wide animation, filled for animatable ops that lack
    /// one.
    default_animation: Option<Animation>,
    /// The shared edit stream index of the last op this handle
    /// recorded, with its sequence: what [`animation`](Self::animation)
    /// retargets — and what proves the entry at that index is still the
    /// recorded one.
    last_edit: Option<(usize, u64)>,
    /// The transaction-wide animation start, on the host's clock.
    default_start: Option<Instant>,
}

impl<T: Target> LayerEdit<T> {
    /// Queues `op` into the open transaction's shared edit stream, in
    /// program order: the outermost commit applies the stream.
    #[inline]
    fn queue(&mut self, op: Op<T>) {
        let mut shared = self.shared.borrow_mut();
        let seq = shared.next_edit_seq;
        shared.next_edit_seq += 1;
        self.last_edit = Some((shared.edit_seqs.len(), seq));
        shared.edit_seqs.push(seq);
        shared.pending.push(op);
    }

    /// Queues a content edit: a `Content(.., None)` placeholder enters
    /// the op stream so program order is preserved, and its payload
    /// waits in `edit_content` for the commit's bookkeeping.
    fn queue_content(&mut self, content: LayerContent<T>) {
        self.queue(Op::Layer(LayerOp::Content(self.layer, None)));
        self.shared.borrow_mut().edit_content.push(content);
    }
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
        self.queue(Op::Layer(LayerOp::Projection(self.layer, target)));
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
        self.queue(Op::Layer(LayerOp::Tilt(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Depth(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
        self
    }

    /// Removes projection, tilt and depth, including their subscriptions
    /// and animation tracks. Existing affine components are unchanged.
    pub fn clear_projection(&mut self) -> &mut Self {
        for kind in [PropKind::Projection, PropKind::Tilt, PropKind::Depth] {
            Shared::unbind(&self.shared, self.layer, kind);
        }
        self.queue(Op::Layer(LayerOp::ClearProjection(self.layer)));
        self
    }
}

impl<T: BackdropSampling> LayerEdit<T> {
    /// Makes the layer a member of a backdrop group: it composites the
    /// group's capture as the bottom-most draw inside its clip. A sample
    /// made with [`BackdropSample::with_effect`] carries a per-member
    /// effect evaluated in the member's composite.
    ///
    /// The sample is a constant or a signal. A bound signal keeps updating
    /// the membership with no further transactions: each change replaces
    /// the sample whole. The property is not animatable: a change's
    /// `Animation` metadata is ignored.
    pub fn backdrop(&mut self, sample: impl Into<Live<BackdropSample>>) -> &mut Self {
        let target = Shared::bind(
            &self.shared,
            self.layer,
            PropKind::Backdrop,
            sample.into(),
            |layer, sample, _, _| LayerOp::Backdrop(layer, Some(sample)),
        );
        self.queue(Op::Layer(LayerOp::Backdrop(self.layer, Some(target))));
        self
    }

    /// Clears the layer's backdrop group membership, and the subscription
    /// of a bound sample.
    pub fn clear_backdrop(&mut self) -> &mut Self {
        Shared::unbind(&self.shared, self.layer, PropKind::Backdrop);
        self.queue(Op::Layer(LayerOp::Backdrop(self.layer, None)));
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
        self.queue(Op::Layer(LayerOp::Transform(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Translation(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Rotation(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Scale(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Skew(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Pivot(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Opacity(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::ScrollOffset(
            self.layer,
            Prop {
                target,
                animation: self.default_animation,
                start: self.default_start,
            },
        )));
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
        self.queue(Op::Layer(LayerOp::Clip(
            self.layer,
            Some(target.into_data()),
        )));
        self
    }

    /// Clears the clip, and the subscription of a bound clip shape: the
    /// formerly bound signal's later changes no longer reach the layer.
    pub fn clear_clip(&mut self) -> &mut Self {
        Shared::unbind(&self.shared, self.layer, PropKind::Clip);
        self.queue(Op::Layer(LayerOp::Clip(self.layer, None)));
        self
    }

    /// Sets the blend mode the layer composites onto its parent with.
    pub fn blend(&mut self, blend: BlendMode) -> &mut Self {
        self.queue(Op::Layer(LayerOp::Blend(self.layer, blend)));
        self
    }

    /// Sets the filter applied to this layer's subtree, as its
    /// consumer-registered id.
    pub fn filter(&mut self, filter: FilterId) -> &mut Self {
        self.queue(Op::Layer(LayerOp::Filter(self.layer, Some(filter))));
        self
    }

    /// Clears the layer's filter.
    pub fn clear_filter(&mut self) -> &mut Self {
        self.queue(Op::Layer(LayerOp::Filter(self.layer, None)));
        self
    }

    /// Sets the content.
    pub fn content(&mut self, content: impl Into<LayerContent<T>>) -> &mut Self {
        self.queue_content(content.into());
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
        self.queue_content(LayerContent::Content(Content::record_into(
            spare, &size, body,
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
        let live = size.into();
        // The same rule `Shared::bind` applies: the binding keeps an
        // entry only when its signal kept the watcher.
        let (mark, target) = {
            let mut shared = self.shared.borrow_mut();
            (shared.draw_watcher(), shared.layout_size(self.layer))
        };
        let generation = mark.generation();
        let bound = target.clone();
        let weak = Rc::downgrade(&self.shared);
        let layer = self.layer;
        let (value, guard) = live.watch(move |change| {
            // `set` notifies the recordings bound to the size — user
            // code — so the current-binding check runs under its own
            // borrow, released first.
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let current = shared
                .borrow()
                .bindings
                .get(layer.raw(), PropKind::LayoutSize)
                .is_some_and(|kept| kept.generation == mark.generation());
            if current {
                bound.set(&change);
            }
        });
        let watching = Shared::kept_watcher(&self.shared, generation);
        target.set(&LayoutSize::change(value, self.default_animation));
        Shared::keep(
            &self.shared,
            self.layer,
            PropKind::LayoutSize,
            generation,
            watching,
            guard,
        );
        self
    }

    /// Clears the content.
    pub fn clear_content(&mut self) -> &mut Self {
        self.queue_content(LayerContent::None);
        self
    }

    /// Appends a child layer.
    pub fn push(&mut self, child: &Layer) -> &mut Self {
        self.queue(Op::Layer(LayerOp::Push {
            parent: self.layer,
            child: child.id,
        }));
        self
    }

    /// Inserts a child layer at `index`.
    pub fn insert(&mut self, index: usize, child: &Layer) -> &mut Self {
        self.queue(Op::Layer(LayerOp::Insert {
            parent: self.layer,
            index,
            child: child.id,
        }));
        self
    }

    /// Removes a child layer.
    pub fn remove(&mut self, child: &Layer) -> &mut Self {
        self.queue(Op::Layer(LayerOp::Detach {
            parent: self.layer,
            child: child.id,
        }));
        self
    }

    /// Overrides the animation of the last property op this handle
    /// recorded — its own last edit, not whatever the shared stream
    /// holds last.
    ///
    /// # Panics
    /// Panics unless the last op this handle recorded was a transform
    /// component, `tilt`,
    /// `depth`, `transform`, `opacity` or `scroll_offset` — `.animation(...)` on any other property is an
    /// invariant violation — and panics when `animation` is a
    /// [`Decay`](crate::Decay) on anything but `scroll_offset`.
    pub fn animation(&mut self, animation: impl Into<Animation>) -> &mut Self {
        let animation = animation.into();
        {
            let mut shared = self.shared.borrow_mut();
            // The sequence tells the recorded entry from one that took
            // its index after an unwind truncated it: retargeting that
            // would animate another layer's edit.
            let stream_start = shared.stream_start;
            let last = self.last_edit.and_then(|(index, seq)| {
                (shared.edit_seqs.get(index) == Some(&seq))
                    .then(|| &mut shared.pending[stream_start + index])
            });
            let Some(Op::Layer(op)) = last else {
                panic!("animation() must follow an animatable layer property")
            };
            assert!(
                !matches!(animation, Animation::Decay(_))
                    || matches!(op, LayerOp::ScrollOffset(..)),
                "Decay is only legal on scroll_offset"
            );
            let is_projection = matches!(op, LayerOp::Projection(..));
            match op.animation_mut() {
                Some(slot) => *slot = Some(animation),
                None if is_projection => {
                    panic!(
                        "the projection matrix is not animatable; animate tilt, depth or the components"
                    )
                }
                None => panic!("animation() must follow an animatable layer property"),
            }
        }
        self
    }
}

/// A transaction's edits to a surface's layer tree. `tx[&layer]` returns
/// the [`LayerEdit`] pointed at that layer; each setter queues its op
/// into the open transaction's shared edit stream in program order.
pub struct Transaction<'a, T: Target> {
    /// The shared `LayerEdit` handle, re-pointed at each indexed layer.
    edit: LayerEdit<T>,
    /// The transaction lives no longer than the body's borrow.
    lifetime: std::marker::PhantomData<&'a ()>,
}

impl<T: Target> std::fmt::Debug for Transaction<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction")
            .field("default_animation", &self.edit.default_animation)
            .field("default_start", &self.edit.default_start)
            .finish_non_exhaustive()
    }
}

impl<T: Target> Index<&Layer> for Transaction<'_, T> {
    type Output = LayerEdit<T>;

    fn index(&self, layer: &Layer) -> &Self::Output {
        assert_eq!(
            self.edit.layer, layer.id,
            "the transaction's edit handle points at another layer"
        );
        &self.edit
    }
}

impl<T: Target> IndexMut<&Layer> for Transaction<'_, T> {
    fn index_mut(&mut self, layer: &Layer) -> &mut LayerEdit<T> {
        if self.edit.layer != layer.id {
            // The recorded op belongs to the layer the handle pointed
            // at: `animation` must not retarget another layer's edit.
            self.edit.last_edit = None;
        }
        self.edit.layer = layer.id;
        &mut self.edit
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

    impl BackdropSampling for TestTarget {}

    /// The layer ops of the change set the next drain takes, applied to
    /// `tree` as a consumer applies them.
    fn drain_layer_ops(
        shared: &Rc<RefCell<Shared<TestTarget>>>,
        tree: &mut crate::SurfaceTree,
    ) -> Vec<LayerOp> {
        let Some(changes) = shared.borrow_mut().take_changes(crate::Instant::now()) else {
            return Vec::new();
        };
        changes
            .ops
            .into_iter()
            .map(|op| match op {
                Op::Layer(LayerOp::Remove(id)) => {
                    tree.remove(id);
                    LayerOp::Remove(id)
                }
                Op::Layer(op) => {
                    tree.apply(op.clone());
                    op
                }
                Op::Install(..) => panic!("the test target installs nothing"),
            })
            .collect()
    }

    fn rim(gain: f32) -> crate::Rim {
        crate::Rim {
            width: 4.0,
            color: [1.0, 1.0, 1.0, 1.0],
            gain,
        }
    }

    #[test]
    fn a_bound_backdrop_change_replaces_the_sample() {
        use crate::{BackdropSample, Refraction};

        let refraction = |depth| Refraction {
            depth,
            strength: 8.0,
        };
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let (group, other) = (BackdropId::new(1), BackdropId::new(2));
        let sample = binding(BackdropSample::with_effect(group, rim(1.0)));
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].backdrop(sample.clone());
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Create(_), LayerOp::Backdrop(id, Some(s))]
                    if *id == layer.id() && *s == BackdropSample::with_effect(group, rim(1.0))
            ),
            "the bound sample starts whole: {ops:?}"
        );

        // A new effect, a new reach and a new group each replace the
        // sample whole.
        for next in [
            BackdropSample::with_effect(group, rim(2.0)),
            BackdropSample::with_effect(group, refraction(4.0)),
            BackdropSample::with_effect(other, refraction(2.0)),
        ] {
            sample.set(next.clone());
            let ops = drain_layer_ops(&shared, &mut tree);
            assert!(
                matches!(
                    ops.as_slice(),
                    [LayerOp::Backdrop(id, Some(s))] if *id == layer.id() && *s == next
                ),
                "a bound change replaces the sample: {ops:?}"
            );
            assert_eq!(tree.layer(layer.id()).backdrop, Some(next));
        }

        // Clearing the membership drops the subscription.
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].clear_backdrop();
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(matches!(ops.as_slice(), [LayerOp::Backdrop(_, None)]));
        sample.set(BackdropSample::with_effect(other, refraction(3.0)));
        assert!(drain_layer_ops(&shared, &mut tree).is_empty());
    }

    #[test]
    fn a_signal_set_inside_its_binding_transaction_ends_with_the_newest_value() {
        use crate::BackdropSample;

        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let first = BackdropSample::with_effect(BackdropId::new(1), rim(1.0));
        let newest = BackdropSample::with_effect(BackdropId::new(2), rim(2.0));
        let sample = binding(first.clone());
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].backdrop(sample.clone()).opacity(opacity.clone());
            sample.set(newest.clone());
            opacity.set(0.25);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Create(_),
                    LayerOp::Backdrop(_, Some(a)),
                    LayerOp::Opacity(_, Prop { target: one, .. }),
                    LayerOp::Backdrop(_, Some(b)),
                    LayerOp::Opacity(_, Prop { target: quarter, .. }),
                ] if *a == first
                    && one.to_bits() == 1.0_f32.to_bits()
                    && *b == newest
                    && quarter.to_bits() == 0.25_f32.to_bits()
            ),
            "the transaction's edits queue ahead of its signals' changes: {ops:?}"
        );
        let node = tree.layer(layer.id());
        assert_eq!(node.backdrop, Some(newest));
        assert_eq!(node.opacity.to_bits(), 0.25_f32.to_bits());

        opacity.set(0.5);
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(id, Prop { target, .. })]
                    if *id == layer.id() && target.to_bits() == 0.5_f32.to_bits()
            ),
            "a later change lands after the transaction: {ops:?}"
        );
    }

    /// A hidden target: every change set drains inline, into `applied`.
    struct HiddenTarget;

    impl Target for HiddenTarget {
        type Queue = HiddenQueue;
        type Install = ();
    }

    impl BackdropSampling for HiddenTarget {}

    struct HiddenQueue {
        applied: Rc<RefCell<Vec<Vec<LayerOp>>>>,
    }

    impl Queue<HiddenTarget> for HiddenQueue {
        fn drains_inline(&self) -> bool {
            true
        }

        fn apply(&self, changes: ChangeSet<HiddenTarget>) {
            let ops = changes
                .ops
                .into_iter()
                .map(|op| match op {
                    Op::Layer(op) => op,
                    Op::Install(..) => panic!("the test target installs nothing"),
                })
                .collect();
            self.applied.borrow_mut().push(ops);
        }

        fn wake(&self) {
            unreachable!("a hidden target drains inline");
        }
    }

    #[test]
    fn a_hidden_surface_drains_a_transaction_once_ending_with_the_newest_value() {
        use crate::BackdropSample;

        let applied = Rc::new(RefCell::new(Vec::new()));
        let shared = Rc::new(RefCell::new(Shared::<HiddenTarget>::new(
            SurfaceId::new(1),
            HiddenQueue {
                applied: Rc::clone(&applied),
            },
        )));
        let layer = Shared::layer(&shared);
        let first = BackdropSample::with_effect(BackdropId::new(1), rim(1.0));
        let newest = BackdropSample::with_effect(BackdropId::new(2), rim(2.0));
        let sample = binding(first.clone());
        let opacity = binding(1.0_f32);
        applied.borrow_mut().clear();
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].backdrop(sample.clone()).opacity(opacity.clone());
            sample.set(newest.clone());
            opacity.set(0.25);
            assert!(
                applied.borrow().is_empty(),
                "nothing drains while the transaction is open"
            );
        });
        let applied = std::mem::take(&mut *applied.borrow_mut());
        assert!(
            matches!(
                applied.as_slice(),
                [ops] if matches!(
                    ops.as_slice(),
                    [
                        LayerOp::Backdrop(_, Some(a)),
                        LayerOp::Opacity(_, Prop { target: one, .. }),
                        LayerOp::Backdrop(_, Some(b)),
                        LayerOp::Opacity(_, Prop { target: quarter, .. }),
                    ] if *a == first
                        && one.to_bits() == 1.0_f32.to_bits()
                        && *b == newest
                        && quarter.to_bits() == 0.25_f32.to_bits()
                )
            ),
            "the commit drains once, the newest values last: {applied:?}"
        );
        let mut tree = crate::SurfaceTree::new();
        tree.apply(LayerOp::Create(layer.id()));
        for op in applied.into_iter().flatten() {
            tree.apply(op);
        }
        let node = tree.layer(layer.id());
        assert_eq!(node.backdrop, Some(newest));
        assert_eq!(node.opacity.to_bits(), 0.25_f32.to_bits());
    }

    #[test]
    fn clearing_a_clip_drops_its_binding() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let clip = binding(Rect::new(0.0, 0.0, 10.0, 10.0));
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].clip(clip.clone());
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Create(_), LayerOp::Clip(_, Some(ShapeData::Rect(rect)))]
                    if *rect == Rect::new(0.0, 0.0, 10.0, 10.0)
            ),
            "the bound clip starts from its shape: {ops:?}"
        );

        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].clear_clip();
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(ops.as_slice(), [LayerOp::Clip(id, None)] if *id == layer.id()),
            "clearing queues the clear: {ops:?}"
        );
        clip.set(Rect::new(0.0, 0.0, 20.0, 20.0));
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            ops.is_empty(),
            "a cleared clip's signal queues nothing: {ops:?}"
        );
    }

    #[test]
    fn a_signal_reading_its_surface_binds_without_a_borrow_conflict() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let id = layer.id();
        let surface = Rc::downgrade(&shared);
        let source = binding(1.0_f32);
        // The map reads the surface's layout size each time it runs,
        // including while the setter binds it and while the body changes it.
        let opacity = source.map(move |opacity| {
            if let Some(surface) = surface.upgrade() {
                let _ = surface.borrow_mut().layout_size(id);
            }
            opacity
        });
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity.clone());
            source.set(0.25);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Create(_),
                    LayerOp::Opacity(_, Prop { target: one, .. }),
                    LayerOp::Opacity(_, Prop { target: quarter, .. }),
                ] if one.to_bits() == 1.0_f32.to_bits()
                    && quarter.to_bits() == 0.25_f32.to_bits()
            ),
            "the newest value lands last: {ops:?}"
        );

        source.set(0.5);
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(op, Prop { target, .. })]
                    if *op == id && target.to_bits() == 0.5_f32.to_bits()
            ),
            "a later change lands after the transaction: {ops:?}"
        );
    }

    #[test]
    fn a_stale_signal_change_loses_to_a_later_constant() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        // The set fires before the constant in program order, but the
        // constant's edit replaces the binding: the deferred change is
        // stale and must not land.
        Shared::run_transaction(&shared, None, None, |tx| {
            opacity.set(0.5);
            tx[&layer].opacity(0.25_f32);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(id, Prop { target, .. })]
                    if *id == layer.id() && target.to_bits() == 0.25_f32.to_bits()
            ),
            "the constant is the last write in program order: {ops:?}"
        );
        assert_eq!(tree.layer(layer.id()).opacity.to_bits(), 0.25_f32.to_bits());

        opacity.set(0.9);
        assert!(
            drain_layer_ops(&shared, &mut tree).is_empty(),
            "the replaced binding queues nothing"
        );
    }

    #[test]
    fn a_constant_then_signal_set_keeps_the_constant() {
        // The constant replaces the binding, so the later set has no
        // watcher left to fire.
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(0.25_f32);
            opacity.set(0.5);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(id, Prop { target, .. })]
                    if *id == layer.id() && target.to_bits() == 0.25_f32.to_bits()
            ),
            "the constant ends the transaction's writes: {ops:?}"
        );
        assert_eq!(tree.layer(layer.id()).opacity.to_bits(), 0.25_f32.to_bits());
    }

    #[test]
    fn a_layer_created_and_edited_in_one_transaction_applies_in_order() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let mut handle = None;
        Shared::run_transaction(&shared, None, None, |tx| {
            let layer = Shared::layer(&shared);
            tx[&layer].opacity(0.5_f32);
            handle = Some(layer);
        });
        let layer = handle.expect("the body kept the handle");
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Create(created), LayerOp::Opacity(edited, Prop { target, .. })]
                    if *created == layer.id()
                        && *edited == layer.id()
                        && target.to_bits() == 0.5_f32.to_bits()
            ),
            "the body's create lands before the transaction's edits: {ops:?}"
        );
        assert_eq!(tree.layer(layer.id()).opacity.to_bits(), 0.5_f32.to_bits());
    }

    #[test]
    fn a_dropped_layer_discards_its_deferred_change() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let id = layer.id();
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        // Dropping the layer ends its binding's generation: the change
        // the signal made earlier in the body is discarded with it.
        Shared::run_transaction(&shared, None, None, |_| {
            opacity.set(0.5);
            drop(layer);
        });
        let ops = shared
            .borrow_mut()
            .take_changes(crate::Instant::now())
            .map(|changes| changes.ops)
            .unwrap_or_default();
        assert!(
            matches!(
                ops.as_slice(),
                [Op::Layer(LayerOp::Remove(removed))] if *removed == id
            ),
            "the drop queues only the remove: {ops:?}"
        );
    }

    #[test]
    fn an_unwound_body_keeps_current_bindings_deferred_changes() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Shared::run_transaction(&shared, None, None, |tx| {
                opacity.set(0.5);
                tx[&layer].transform(Affine::scale(2.0));
                panic!("the body is lost");
            });
        }));
        assert!(panicked.is_err());
        {
            let state = shared.borrow();
            assert!(
                !state.transaction_open && state.pending.len() == 1 && state.deferred.is_empty(),
                "the unwind resolves deferred and restores the flag"
            );
        }

        // The body's own edit is truncated, but the signal's deferred
        // change resolves on unwind: the binding is still the
        // property's current one, so the layer does not disagree with
        // its signal.
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(id, Prop { target, .. })]
                    if *id == layer.id() && target.to_bits() == 0.5_f32.to_bits()
            ),
            "the still-current binding's 0.5 lands: {ops:?}"
        );

        // A write since ends the binding: a change deferred under it
        // would have lost, and the signal now queues nothing.
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(0.2_f32);
        });
        drain_layer_ops(&shared, &mut tree);
        opacity.set(0.9);
        assert!(
            drain_layer_ops(&shared, &mut tree).is_empty(),
            "the replaced binding queues nothing"
        );
    }

    #[test]
    fn an_inner_transactions_edits_apply_at_the_outermost_commit() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        let outer_animation: Animation = Curve::linear(Duration::from_millis(400)).into();
        let inner_animation: Animation = Curve::linear(Duration::from_millis(800)).into();
        Shared::run_transaction(&shared, Some(outer_animation), None, |tx| {
            tx[&layer].opacity(0.2_f32);
            Shared::run_transaction(&shared, Some(inner_animation), None, |inner| {
                inner[&layer].opacity(0.5_f32);
            });
            tx[&layer].transform(Affine::translate(Vec2::new(1.0, 2.0)));
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Opacity(id, Prop {
                        target: first,
                        animation: first_animation,
                        ..
                    }),
                    LayerOp::Opacity(_, Prop {
                        target: second,
                        animation: second_animation,
                        ..
                    }),
                    LayerOp::Transform(_, Prop {
                        animation: third_animation,
                        ..
                    }),
                ] if *id == layer.id()
                    && first.to_bits() == 0.2_f32.to_bits()
                    && *first_animation == Some(outer_animation)
                    && second.to_bits() == 0.5_f32.to_bits()
                    && *second_animation == Some(inner_animation)
                    && *third_animation == Some(outer_animation)
            ),
            "each edit carries its transaction's animation, in program order: {ops:?}"
        );
    }

    #[test]
    fn an_outer_write_after_an_inner_transaction_wins() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        let outer_animation: Animation = Curve::linear(Duration::from_millis(400)).into();
        let inner_animation: Animation = Curve::linear(Duration::from_millis(800)).into();
        Shared::run_transaction(&shared, Some(outer_animation), None, |tx| {
            Shared::run_transaction(&shared, Some(inner_animation), None, |inner| {
                inner[&layer].opacity(0.5_f32);
            });
            tx[&layer].opacity(0.2_f32);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Opacity(_, Prop {
                        target: first,
                        animation: first_animation,
                        ..
                    }),
                    LayerOp::Opacity(id, Prop {
                        target: second,
                        animation: second_animation,
                        ..
                    }),
                ] if *id == layer.id()
                    && first.to_bits() == 0.5_f32.to_bits()
                    && *first_animation == Some(inner_animation)
                    && second.to_bits() == 0.2_f32.to_bits()
                    && *second_animation == Some(outer_animation)
            ),
            "the outer's later write lands last: {ops:?}"
        );
    }

    #[test]
    fn an_inner_bodys_created_layer_applies_at_the_outer_commit() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let outer_layer = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        let mut created = None;
        Shared::run_transaction(&shared, None, None, |tx| {
            Shared::run_transaction(&shared, None, None, |inner| {
                let layer = Shared::layer(&shared);
                inner[&layer].opacity(0.5_f32);
                created = Some(layer);
            });
            let state = shared.borrow();
            assert_eq!(
                state.direct_ops.len(),
                1,
                "mid-transaction the create sits in the direct queue, undrained"
            );
            assert_eq!(
                state.pending.len(),
                1,
                "mid-transaction the inner's edit sits in the stream, undrained"
            );
            drop(state);
            tx[&outer_layer].opacity(0.8_f32);
        });
        let created = created.expect("the inner body ran");
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Create(new),
                    LayerOp::Opacity(edited, Prop { target, .. }),
                    LayerOp::Opacity(outer, Prop {
                        target: outer_target, ..
                    }),
                ] if *new == created.id()
                    && *edited == created.id()
                    && target.to_bits() == 0.5_f32.to_bits()
                    && *outer == outer_layer.id()
                    && outer_target.to_bits() == 0.8_f32.to_bits()
            ),
            "the inner's edits land at the outer commit, in order: {ops:?}"
        );
        assert_eq!(
            tree.layer(created.id()).opacity.to_bits(),
            0.5_f32.to_bits()
        );
    }

    #[test]
    fn no_edit_reaches_pending_before_the_outermost_commit() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(0.2_f32);
            Shared::run_transaction(&shared, None, None, |inner| {
                inner[&layer].opacity(0.5_f32);
            });
            let state = shared.borrow();
            assert!(
                state.pending.len() == 2,
                "edits queue in the shared stream, undrained: {:?}",
                state.pending
            );
            drop(state);
            tx[&layer].transform(Affine::IDENTITY);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert_eq!(ops.len(), 3, "one commit applies the whole stream: {ops:?}");
    }

    #[test]
    fn a_panicking_inner_body_unwinds_the_whole_transaction() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let opacity = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Shared::run_transaction(&shared, None, None, |tx| {
                tx[&layer].opacity(0.2_f32);
                Shared::run_transaction(&shared, None, None, |inner| {
                    opacity.set(0.5);
                    inner[&layer].transform(Affine::IDENTITY);
                    panic!("the inner body is lost");
                });
            });
        }));
        assert!(panicked.is_err());
        {
            let state = shared.borrow();
            assert!(
                state.deferred.is_empty() && state.pending.is_empty() && !state.transaction_open,
                "the unwound transactions left nothing queued"
            );
        }
        // The deferred change resolves at the unwind, but the body's
        // `opacity(0.2)` ended its binding's generation: it is dropped.
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(ops.is_empty(), "a lost generation lands nothing: {ops:?}");
        assert_eq!(tree.layer(layer.id()).opacity.to_bits(), 1.0_f32.to_bits());

        // The surface still works: the next transaction applies normally.
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(0.9_f32);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(id, Prop { target, .. })]
                    if *id == layer.id() && target.to_bits() == 0.9_f32.to_bits()
            ),
            "the next transaction applies normally: {ops:?}"
        );
    }

    #[test]
    fn a_stale_signal_change_before_a_rebind_is_discarded() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let a = binding(1.0_f32);
        let b = binding(0.3_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(a.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        // `a`'s change fires before the rebind in program order; the
        // rebind ends its generation, so the deferred change is stale.
        Shared::run_transaction(&shared, None, None, |tx| {
            a.set(0.5);
            tx[&layer].opacity(b.clone());
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(id, Prop { target, .. })]
                    if *id == layer.id() && target.to_bits() == 0.3_f32.to_bits()
            ),
            "only the rebind lands: {ops:?}"
        );

        a.set(0.9);
        assert!(
            drain_layer_ops(&shared, &mut tree).is_empty(),
            "the replaced binding queues nothing"
        );
    }

    /// A watcher on `sig` — registered before the layer's binding, so it
    /// fires first in the notify — rebinds the layer to `sig2`. The
    /// layer's stale watcher still fires off nami's snapshot afterwards:
    /// its value must not land, at depth 0 or inside a transaction.
    fn rebind_during_notify(inside_transaction: bool) -> Vec<LayerOp> {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let id = layer.id();
        let sig = binding(1.0_f32);
        let sig2 = binding(0.2_f32);
        let _user_guard = {
            let shared = Rc::clone(&shared);
            let owner: Rc<dyn LayerOwner> = Rc::clone(&shared) as Rc<dyn LayerOwner>;
            let handle = Layer::new(id, owner, false);
            sig.watch(move |_: Context<f32>| {
                Shared::run_transaction(&shared, None, None, |tx| {
                    tx[&handle].opacity(sig2.clone());
                });
            })
        };
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(sig.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        if inside_transaction {
            Shared::run_transaction(&shared, None, None, |_| {
                sig.set(0.9);
            });
        } else {
            sig.set(0.9);
        }
        drain_layer_ops(&shared, &mut tree)
    }

    #[test]
    fn a_watcher_replaced_during_its_own_notify_drops_the_stale_change() {
        for inside_transaction in [false, true] {
            let ops = rebind_during_notify(inside_transaction);
            assert!(
                matches!(
                    ops.as_slice(),
                    [LayerOp::Opacity(_, Prop { target, .. })]
                        if target.to_bits() == 0.2_f32.to_bits()
                ),
                "inside_transaction={inside_transaction}: only the rebind lands: {ops:?}"
            );
        }
    }

    #[test]
    fn a_replaced_binding_drops_its_map_closure_outside_the_surface_borrow() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        // A `Live::map` closure owning another layer of the same surface:
        // dropping the replaced binding's guard drops the closure, and
        // the layer with it — re-entering the surface, which must be
        // unborrowed then.
        let owned = Shared::layer(&shared);
        let owned_id = owned.id();
        let source = binding(1.0_f32);
        let opacity = Live::from(source).map(move |value| {
            let _ = &owned;
            value
        });
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(opacity);
            tx[&layer].opacity(0.25_f32);
        });
        let _ = &mut tree;
        let ops = shared
            .borrow_mut()
            .take_changes(crate::Instant::now())
            .map(|changes| changes.ops)
            .unwrap_or_default();
        assert!(
            ops.iter().any(|op| matches!(
                op,
                Op::Layer(LayerOp::Remove(id)) if *id == owned_id
            )),
            "the dropped closure's layer is removed without a borrow conflict: {ops:?}"
        );
    }

    #[test]
    fn a_panicking_commit_leaves_the_surface_open_to_later_transactions() {
        // A subscription guard that panics on drop: retiring the content
        // that kept it panics inside the commit's drop of retired
        // contents.
        fn guard_panics() {
            panic!("the content's guard is lost");
        }
        #[derive(Clone)]
        struct PanicGuardSignal;
        impl Signal for PanicGuardSignal {
            type Output = Rect;
            type Guard = nami_core::watcher::OnDrop<fn()>;

            fn snapshot(&self) -> Rect {
                Rect::new(0.0, 0.0, 1.0, 1.0)
            }

            fn watch(&self, _: impl Fn(Context<Rect>) + 'static) -> Self::Guard {
                nami_core::watcher::OnDrop::new(guard_panics)
            }
        }

        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].record(|c| c.fill(PanicGuardSignal, WorkingColor::WHITE));
        });
        drain_layer_ops(&shared, &mut tree);

        // Replacing the content retires it at the commit: the panic
        // unwinds `run_transaction` after `transaction_open` was
        // restored, so the surface is not frozen open.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Shared::run_transaction(&shared, None, None, |tx| {
                tx[&layer].clear_content();
            });
        }));
        assert!(panicked.is_err(), "the dropped guard panicked");
        {
            let state = shared.borrow();
            assert!(
                !state.transaction_open
                    && state.edit_content.is_empty()
                    && state.deferred.is_empty(),
                "the commit left no open state behind"
            );
        }
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(0.4_f32);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            ops.iter().any(|op| matches!(
                op,
                LayerOp::Opacity(id, Prop { target, .. })
                    if *id == layer.id() && target.to_bits() == 0.4_f32.to_bits()
            )),
            "a later transaction commits normally: {ops:?}"
        );
    }

    #[test]
    fn a_caught_inner_panic_resolves_deferred_changes_at_the_outer_commit() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let kept = layer(&shared);
        let rebound = layer(&shared);
        let kept_sig = binding(1.0_f32);
        let rebound_sig = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&kept].opacity(kept_sig.clone());
            tx[&rebound].opacity(rebound_sig.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&kept].transform(Affine::IDENTITY);
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Shared::run_transaction(&shared, None, None, |inner| {
                    // Deferred under both bindings, then the inner body is
                    // lost: its unwind truncates only its own recorded
                    // edits — the deferred changes stay for the
                    // outermost commit to resolve.
                    kept_sig.set(0.5);
                    rebound_sig.set(0.6);
                    inner[&kept].transform(Affine::scale(2.0));
                    panic!("the inner body is lost");
                });
            }));
            assert!(panicked.is_err(), "the outer body caught the panic");
            // Rebinding ends `rebound_sig`'s generation: its deferred
            // change loses to the later write.
            tx[&rebound].opacity(0.3_f32);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Transform(id, ..),
                    LayerOp::Opacity(edited, Prop { target, .. }),
                    LayerOp::Opacity(kept_id, Prop {
                        target: kept_target,
                        ..
                    }),
                ] if *id == kept.id()
                    && *edited == rebound.id()
                    && target.to_bits() == 0.3_f32.to_bits()
                    && *kept_id == kept.id()
                    && kept_target.to_bits() == 0.5_f32.to_bits()
            ),
            "the still-current binding's change lands, the rebound one's does not: {ops:?}"
        );

        // The constant replaced the binding for good: its signal's
        // later changes queue nothing.
        rebound_sig.set(0.7);
        assert!(
            drain_layer_ops(&shared, &mut tree).is_empty(),
            "the replaced binding queues nothing"
        );
    }

    #[test]
    fn a_layout_size_binding_replaced_during_its_own_notify_drops_the_stale_change() {
        let shared = shared();
        let layer = layer(&shared);
        let id = layer.id();
        let sig = binding(Size::new(1.0, 1.0));
        let sig2 = binding(Size::new(2.0, 2.0));
        // A user watcher on `sig`, registered before the layer binds it,
        // rebinds the layer's `layout_size` to `sig2` during the notify:
        // the stale value must not reach the layer's `LayoutSize`.
        let _user_guard = {
            let shared = Rc::clone(&shared);
            let owner: Rc<dyn LayerOwner> = Rc::clone(&shared) as Rc<dyn LayerOwner>;
            let handle = Layer::new(id, owner, false);
            sig.watch(move |_: Context<Size>| {
                Shared::run_transaction(&shared, None, None, |tx| {
                    tx[&handle].layout_size(sig2.clone());
                });
            })
        };
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].layout_size(sig.clone());
        });

        sig.set(Size::new(9.0, 9.0));
        let size = shared.borrow_mut().layout_size(id);
        assert_eq!(
            size.snapshot(),
            Size::new(2.0, 2.0),
            "the rebind's value, not the stale one"
        );
    }

    #[test]
    fn a_removed_layers_content_drops_its_map_closure_outside_the_borrow() {
        let shared = shared();
        let layer = layer(&shared);
        // A content slot's `Live::map` closure owns another layer of the
        // same surface: removing the recorded layer drops the content's
        // guards, the closure, and the owned layer with them — which
        // re-enters the surface and must find it unborrowed.
        let owned = Shared::layer(&shared);
        let owned_id = owned.id();
        let source = binding(Rect::new(0.0, 0.0, 1.0, 1.0));
        let shape = Live::from(source).map(move |rect: Rect| {
            let _ = &owned;
            rect
        });
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].record(|c| c.fill(shape, WorkingColor::WHITE));
        });

        drop(layer);
        let ops = shared
            .borrow_mut()
            .take_changes(crate::Instant::now())
            .map(|changes| changes.ops)
            .unwrap_or_default();
        assert!(
            ops.iter().any(|op| matches!(
                op,
                Op::Layer(LayerOp::Remove(id)) if *id == owned_id
            )),
            "the removed content's closure-owned layer is removed without a borrow conflict: {ops:?}"
        );
    }

    #[test]
    fn animation_retargets_the_handles_own_last_edit() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let a = layer(&shared);
        let b = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        let outer_animation: Animation = Curve::linear(Duration::from_millis(400)).into();
        let inner_animation: Animation = Curve::linear(Duration::from_millis(800)).into();
        let retargeted: Animation = Curve::linear(Duration::from_millis(100)).into();
        Shared::run_transaction(&shared, Some(outer_animation), None, |tx| {
            tx[&a].opacity(0.5_f32);
            Shared::run_transaction(&shared, Some(inner_animation), None, |inner| {
                inner[&b].transform(Affine::IDENTITY);
            });
            tx[&a].animation(retargeted);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Opacity(id, Prop {
                        target,
                        animation: Some(anim),
                        ..
                    }),
                    LayerOp::Transform(other, Prop {
                        animation: Some(anim2),
                        ..
                    }),
                ] if *id == a.id()
                    && target.to_bits() == 0.5_f32.to_bits()
                    && *anim == retargeted
                    && *other == b.id()
                    && *anim2 == inner_animation
            ),
            "the handle's own last edit takes the animation: {ops:?}"
        );
    }

    #[test]
    #[should_panic(expected = "animation() must follow an animatable layer property")]
    fn animation_does_not_retarget_another_layers_edit() {
        let shared = shared();
        let a = layer(&shared);
        let b = layer(&shared);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&a].opacity(0.5_f32);
            // Repointing the handle clears its recorded op: the
            // animation must not silently land on `a`'s edit.
            tx[&b].animation(Curve::linear(Duration::from_millis(100)));
        });
    }

    #[test]
    #[should_panic(expected = "animation() must follow an animatable layer property")]
    fn animation_does_not_retarget_an_edit_truncated_under_the_handle() {
        let shared = shared();
        let a = layer(&shared);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&a].opacity(0.5_f32);
            {
                // What an inner unwind leaves: the recorded entry is
                // truncated and another op takes its index, stamped
                // with its own sequence. The stale sequence keeps the
                // retarget off it.
                let mut shared = shared.borrow_mut();
                shared.pending.pop();
                shared.edit_seqs.pop();
                let seq = shared.next_edit_seq;
                shared.next_edit_seq += 1;
                shared.pending.push(Op::Layer(LayerOp::Transform(
                    a.id(),
                    Prop {
                        target: Affine::IDENTITY,
                        animation: None,
                        start: None,
                    },
                )));
                shared.edit_seqs.push(seq);
            }
            tx[&a].animation(Curve::linear(Duration::from_millis(100)));
        });
    }

    #[test]
    fn a_removed_layers_stream_edits_are_discarded_at_the_commit() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let id = layer.id();
        Shared::run_transaction(&shared, None, None, |_| {
            Shared::run_transaction(&shared, None, None, |inner| {
                inner[&layer].opacity(0.5_f32);
                inner[&layer]
                    .record(|c| c.fill(Rect::new(0.0, 0.0, 1.0, 1.0), WorkingColor::WHITE));
            });
            // Dropping the handle queues the layer's `Remove` while the
            // transaction is still open.
            drop(layer);
        });
        // Applying the ops must not see the removed layer's edits.
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Create(created),
                    LayerOp::Remove(removed),
                ] if *created == id && *removed == id
            ),
            "the commit queued only the create and the remove: {ops:?}"
        );
        assert!(
            !shared.borrow().contents.contains_key(&id),
            "nothing of the removed layer is re-created"
        );
    }

    /// A layer created inside a transaction whose body panics: its
    /// `Create` is a direct op, not a stream edit, so the unwind keeps
    /// it — the handle outlives the transaction. When the handle drops
    /// later, its `Remove` applies against a layer the tree did create.
    #[test]
    fn a_layer_created_in_a_lost_body_outlives_the_unwind() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let mut created = None;
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Shared::run_transaction(&shared, None, None, |tx| {
                created = Some(layer(&shared));
                tx[created.as_ref().expect("created")].opacity(0.5_f32);
                panic!("the body is lost");
            });
        }));
        assert!(panicked.is_err());
        let created = created.expect("the body ran");

        // The create survived; the body's edit to it did not.
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(ops.as_slice(), [LayerOp::Create(id)] if *id == created.id()),
            "only the direct create op landed: {ops:?}"
        );

        // Dropping the outlived handle queues the remove — for a layer
        // the tree has, not one the truncated stream would have made.
        let created_id = created.id();
        drop(created);
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(ops.as_slice(), [LayerOp::Remove(id)] if *id == created_id),
            "the remove lands against the created layer: {ops:?}"
        );
    }

    /// Direct ops a body queues — creates and removes — land ahead of
    /// the body's stream edits at the commit, in queue order.
    #[test]
    fn a_bodys_direct_ops_apply_ahead_of_its_edits() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let anchor = layer(&shared);
        let mut made = None;
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&anchor].opacity(0.5_f32);
            let fresh = layer(&shared);
            made = Some(fresh);
            tx[made.as_ref().expect("made")].opacity(0.8_f32);
        });
        let made = made.expect("the body ran");
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [
                    LayerOp::Create(pre),
                    LayerOp::Create(id),
                    LayerOp::Opacity(a, _),
                    LayerOp::Opacity(m, _),
                ] if *pre == anchor.id()
                    && *id == made.id()
                    && *a == anchor.id()
                    && *m == made.id()
            ),
            "the body's create lands ahead of its edits, behind the \
                     pre-open queue: {ops:?}"
        );
    }

    /// A nested transaction's direct ops survive its own unwind for the
    /// outer commit — the handles outlive the inner unwind as they do
    /// an outer one.
    #[test]
    fn an_inner_unwinds_direct_ops_still_apply() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let mut created = None;
        Shared::run_transaction(&shared, None, None, |_| {
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Shared::run_transaction(&shared, None, None, |inner| {
                    created = Some(layer(&shared));
                    inner[created.as_ref().expect("created")].opacity(0.5_f32);
                    panic!("the inner body is lost");
                });
            }));
            assert!(panicked.is_err());
        });
        let created = created.expect("the inner body ran");
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(ops.as_slice(), [LayerOp::Create(id)] if *id == created.id()),
            "the inner body's create applied, its edit did not: {ops:?}"
        );
        drop(created);
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(ops.as_slice(), [LayerOp::Remove(_)]),
            "the later remove lands: {ops:?}"
        );
    }

    /// A transaction's `start` reaches every prop it edits (dev's test —
    /// the merge dropped it while `Prop` moved fields).
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

    /// A bound change's `AnimationStart` metadata reaches the prop's
    /// `start` (dev's test — the merge dropped it).
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

    /// A bound change that lands while a transaction is open defers to
    /// the outermost commit — and still reads its `AnimationStart` from
    /// the change's metadata.
    #[test]
    fn a_bound_change_deferred_in_a_transaction_reads_its_start() {
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
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].transform(Affine::scale(1.0));
            opacity.set(0.5);
        });
        let Some(changes) = shared.borrow_mut().take_changes(crate::Instant::now()) else {
            panic!("the commit resolved the deferred change");
        };
        let mut seen = 0;
        for op in &changes.ops {
            let Op::Layer(LayerOp::Opacity(_, prop)) = op else {
                continue;
            };
            assert_eq!(prop.target.to_bits(), 0.5_f32.to_bits());
            assert_eq!(prop.start, Some(start));
            assert!(matches!(prop.animation, Some(Animation::Curve(_))));
            seen += 1;
        }
        assert_eq!(seen, 1, "the deferred change kept its metadata");
    }

    #[test]
    #[should_panic(expected = "detaching from the wrong parent")]
    fn a_reparented_child_cannot_be_removed_by_its_first_parent() {
        // `tx[a].push(c); tx[b].push(c); tx[a].remove(c)` records the ops in
        // program order: the second push reparents `child` onto `b`, so the
        // later detach still naming `a` is the invariant violation and the
        // tree refuses it — the commit does not silently resolve it.
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let parent_a = layer(&shared);
        let parent_b = layer(&shared);
        let child = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&parent_a].push(&child);
            tx[&parent_b].push(&child);
            tx[&parent_a].remove(&child);
        });
        drain_layer_ops(&shared, &mut tree);
    }

    /// A signal whose `watch` returns the `()` guard — nami's collection
    /// and constant signals do — while still delivering every change:
    /// the watcher stays registered in the signal itself, so a `None`
    /// guard means "nothing to drop", never "nothing to keep". The
    /// record layer's kept binding must keep its generation so the
    /// change lands (water-rs/waterui#1788).
    #[derive(Clone)]
    struct UnguardedSignal {
        inner: nami::Binding<f32>,
        kept: Rc<RefCell<Vec<Box<dyn std::any::Any>>>>,
    }

    impl Signal for UnguardedSignal {
        type Output = f32;
        type Guard = ();
        fn snapshot(&self) -> f32 {
            self.inner.snapshot()
        }
        fn watch(&self, watcher: impl Fn(Context<f32>) + 'static) {
            let guard = self.inner.watch(watcher);
            self.kept.borrow_mut().push(Box::new(guard));
        }
    }

    #[test]
    fn a_zero_sized_guards_signal_still_delivers_changes() {
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let id = layer.id();
        let signal = UnguardedSignal {
            inner: binding(0.25_f32),
            kept: Rc::new(RefCell::new(Vec::new())),
        };
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(signal.clone());
        });
        drain_layer_ops(&shared, &mut tree);

        // The `()` guard registers the binding entry even so: the
        // signal's next change still queues the edit.
        signal.inner.set(0.75);
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            ops.iter().any(|op| matches!(
                op,
                LayerOp::Opacity(layer, prop)
                    if *layer == id && prop.target.to_bits() == 0.75_f32.to_bits()
            )),
            "the unguarded signal's change lands: {ops:?}"
        );
    }

    #[test]
    fn a_removed_child_detach_op_drops_with_it() {
        // `tx[p].remove(&c)` queues the detach; dropping `c`'s handle in
        // the same body queues its `Remove` — which lands ahead of the
        // stream. The detach still naming the removed child must drop,
        // not panic the tree (water-rs/waterui#1788).
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let parent = layer(&shared);
        let child = layer(&shared);
        let child_id = child.id();
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&parent].push(&child);
        });
        drain_layer_ops(&shared, &mut tree);
        Shared::run_transaction(&shared, None, None, move |tx| {
            tx[&parent].remove(&child);
            drop(child);
        });
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            ops.iter().all(|op| !matches!(op, LayerOp::Detach { .. })),
            "the removed child's detach drops: {ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(
                op,
                LayerOp::Remove(id) if *id == child_id
            )),
            "its remove lands: {ops:?}"
        );
    }

    #[test]
    fn a_layer_created_inserted_and_dropped_in_one_body_lands_only_its_create_and_remove() {
        // A body that creates a layer, inserts it and drops the handle
        // queues [Create, Insert{p,c}, Remove]: the create and the remove
        // are direct ops, the insert a stream edit naming the removed
        // child, so it drops with it — Create + Remove land alone
        // (water-rs/waterui#1788).
        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let parent = layer(&shared);
        drain_layer_ops(&shared, &mut tree);
        let mut child_id = None;
        Shared::run_transaction(&shared, None, None, |tx| {
            let child = Shared::layer(&shared);
            child_id = Some(child.id());
            tx[&parent].insert(0, &child);
            drop(child);
        });
        let child_id = child_id.expect("the body ran");
        let ops = drain_layer_ops(&shared, &mut tree);
        let child_ops: Vec<_> = ops
            .iter()
            .filter(|op| match op {
                LayerOp::Create(id) | LayerOp::Remove(id) => *id == child_id,
                LayerOp::Insert { child, .. } => *child == child_id,
                _ => false,
            })
            .collect();
        assert!(
            matches!(
                child_ops.as_slice(),
                [LayerOp::Create(_), LayerOp::Remove(_)]
            ),
            "only the child's create and remove land: {ops:?}"
        );
    }

    /// A signal holding a `Weak` of its surface that `watch` does not
    /// keep — it reads the surface in `snapshot` and forwards `watch` to
    /// an inner binding. Starting the owned subscription drops the
    /// `Live`'s copy of the signal, and its `Weak` with it, while the
    /// inner binding keeps the watcher: the binding must stay kept and
    /// its changes must land (water-rs/waterui#1788).
    #[test]
    fn a_signal_holding_its_surface_keeps_its_binding() {
        #[derive(Clone)]
        struct ReadsSurface {
            surface: Weak<RefCell<Shared<TestTarget>>>,
            inner: nami::Binding<f32>,
        }

        impl Signal for ReadsSurface {
            type Output = f32;
            type Guard = <nami::Binding<f32> as Signal>::Guard;

            fn snapshot(&self) -> f32 {
                assert!(self.surface.upgrade().is_some(), "the surface is alive");
                self.inner.snapshot()
            }

            fn watch(&self, watcher: impl Fn(Context<f32>) + 'static) -> Self::Guard {
                self.inner.watch(watcher)
            }
        }

        let shared = shared();
        let mut tree = crate::SurfaceTree::new();
        let layer = layer(&shared);
        let id = layer.id();
        let source = binding(1.0_f32);
        Shared::run_transaction(&shared, None, None, |tx| {
            tx[&layer].opacity(ReadsSurface {
                surface: Rc::downgrade(&shared),
                inner: source.clone(),
            });
        });
        drain_layer_ops(&shared, &mut tree);
        source.set(0.25);
        let ops = drain_layer_ops(&shared, &mut tree);
        assert!(
            matches!(
                ops.as_slice(),
                [LayerOp::Opacity(op, Prop { target, .. })]
                    if *op == id && target.to_bits() == 0.25_f32.to_bits()
            ),
            "the inner binding kept the watcher, so the change lands: {ops:?}"
        );
    }
}
