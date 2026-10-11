//! Per-node retained state: the [`NodeCell`] every render node owns, the
//! [`Dirty`] marks that attribute frame work to it, and the [`NodeCore`]
//! embedded in every node struct.
//!
//! A cell replaces the frame-wide refresh request: a signal watcher, an
//! animation tick, a layout invalidation or an interaction transition marks
//! the *owning* node instead of the window. Marks propagate [`Dirty`] bits to
//! ancestors so the pump can see pending work without walking the tree, and
//! each frame touches only the marked nodes.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, NodeId as AccessibilityNodeId};

use waterui_core::Retain;

use waterui_backend_core::frame_signals::FrameSignals;

use super::placement::Placement;
use super::registry::OwnerRegistrations;
use super::scopes::RetainedScopes;

/// The frame-work a cell is dirty for, one bit per cause.
///
/// `STRUCTURE` — the node's subtree shape changed (a `Dynamic` patch, a
/// collection membership update). `LAYOUT` — the node's layout inputs
/// changed; the layout pass must re-measure it and every ancestor already
/// carries the bit. `PAINT` — its recorded content is stale. `PRODUCER` —
/// its GPU/external-frame producer published a new frame or plane size.
/// The layer-edit bits (`PLACE`, `COMMIT`) join with the mount that first
/// writes them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dirty(u8);

impl Dirty {
    /// No work pending.
    pub const NONE: Self = Self(0);
    /// The subtree shape changed: children must be rebuilt.
    pub const STRUCTURE: Self = Self(1 << 0);
    /// Layout inputs changed: the node (and every ancestor's `own`) must
    /// re-measure.
    pub const LAYOUT: Self = Self(1 << 1);
    /// Recorded drawing changed: the node re-records its runs.
    pub const PAINT: Self = Self(1 << 2);
    /// The node's placement props changed: its frame's layer props are
    /// rewritten without re-recording.
    pub const PLACE: Self = Self(1 << 3);
    /// A GPU/external-frame producer published work for the install layer.
    pub const PRODUCER: Self = Self(1 << 4);
    /// The node holds a finished program the commit has not lowered yet.
    pub const COMMIT: Self = Self(1 << 5);

    /// Whether every bit in `other` is set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no bit is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `self` with every bit in `other` cleared.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::ops::BitOr for Dirty {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for Dirty {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for Dirty {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// The retained frame-work identity of one render node.
///
/// A cell lives exactly as long as the node that owns it: built in the
/// node's constructor, dropped with it. `own` carries marks raised on the
/// node itself; `below` carries any mark raised in its subtree, so pending
/// work is visible from the root without a tree walk. `parent` is a strong
/// link set on attach — it keeps ancestors' cells alive while a descendant
/// is held somewhere (a stale watcher, a deferred overlay), and releases
/// when the subtree is actually dropped, since cells never point back down.
pub struct NodeCell {
    /// Marks raised on this node itself.
    own: Cell<Dirty>,
    /// Marks raised anywhere in this node's subtree.
    below: Cell<Dirty>,
    /// The parent node's cell, set on attach (`None` before and on the
    /// window root cell).
    parent: RefCell<Option<Rc<Self>>>,
    /// This window's frame-signal handle: a mark wakes the host.
    frames: FrameSignals,
    /// The node's placement mirror (layer props for hit and paint
    /// resolution).
    placement: Rc<Placement>,
    /// The scopes the node's last record pushed around its descendants
    /// (§B.3) — a partial descent replays them so a re-recording child sees
    /// the same ancestor state.
    pub scopes: RefCell<RetainedScopes>,
    /// The owning node's retained layer state (held strongly by its
    /// [`NodeCore`], so the layers drop with the node, not the cell).
    retained: RefCell<std::rc::Weak<super::layers::NodeRetained>>,
    /// The registrations the node's last record emitted — the retained
    /// entries its materialization replays (`None` until the first record
    /// registers, or after a purge). Purging the cell's bucket is the
    /// per-owner retire: O(entries of that owner), no registry sweep.
    pub(crate) registrations: RefCell<Option<Box<OwnerRegistrations>>>,
    /// The cell is listed in the window's `retained.owners` enumeration —
    /// set the first time it registers, so materialization visits each
    /// owner once.
    pub(crate) registered: Cell<bool>,
    /// Cells this cell's records parented under it — filled by
    /// `set_parent`, pruned of dead entries where it is walked.
    children: RefCell<Vec<Weak<Self>>>,
    /// The `RetainedSubview` roots this cell's records own — every subview
    /// `attach`ed under it. At record end a subview the record did not
    /// place has its subtree retired (§D's unplaced rule).
    pub(crate) subviews: RefCell<Vec<Weak<Self>>>,
    /// The node is a `.material_group()` scope marker: its record pushes
    /// this cell onto `material_group_scopes` while its child flushes.
    /// Set once at build (the effect never changes) so a partial descent
    /// can find every enclosing scope from the subtree root's ancestry.
    pub(crate) material_group_scope: Cell<bool>,
    /// The record sequence this cell's placement was last linked under —
    /// the "placed during that record" stamp `subviews` retires against.
    placed_seq: Cell<u64>,
    /// The cell's subtree was retired as unplaced and nothing has placed
    /// the cell since — `retire_subtree` skips it rather than re-walking a
    /// hidden subtree every frame. Placing the cell clears it.
    pub(crate) retired: Cell<bool>,
    /// §B.4: the node is an accessibility unit — its record opened an
    /// accessibility container or claimed a gesture scope, so a descendant's
    /// emissions diff escalates here. Reset at each record; set only when
    /// the record actually claims a scope.
    #[cfg(feature = "accessibility")]
    pub a11y_unit: Cell<bool>,
    /// An accessibility-emitting record is open on this cell right now.
    /// The unit rule skips its PAINT mark while the unit ancestor is
    /// already mid-record — the mark exists to wake an ancestor a partial
    /// re-record did not cover, not one whose record is in flight.
    #[cfg(feature = "accessibility")]
    pub a11y_recording: Cell<bool>,
    /// The accessibility ids the cell's open (or last) record emitted —
    /// the emitted set the unit rule diffs and `retire_subtree` sweeps
    /// out of the shared maps.
    #[cfg(feature = "accessibility")]
    pub(crate) a11y_emitted: RefCell<Vec<AccessibilityNodeId>>,
    /// The emitted set the cell's last record left behind — the baseline
    /// the unit rule's next record diffs against, `None` until its first
    /// record completes. Stored on the cell so it lives and dies with the
    /// node.
    #[cfg(feature = "accessibility")]
    pub(crate) a11y_retired: RefCell<Option<Vec<(AccessibilityNodeId, AccessibilityNode)>>>,
}

impl NodeCell {
    /// The owning node's retained layer state.
    ///
    /// # Panics
    /// Panics when the node was dropped while its cell is still listed in
    /// a program — a program never outlives the frame that recorded it.
    pub(crate) fn retained(&self) -> Rc<super::layers::NodeRetained> {
        self.retained
            .borrow()
            .upgrade()
            .expect("hydrolysis commit: a program lists a node that was dropped before its commit")
    }

    /// The owning node's retained layer state while the node lives —
    /// `None` once it dropped. A child a `committed` list reaches may be
    /// dead already: its drop queued the layer removal before this commit.
    pub(crate) fn try_retained(&self) -> Option<Rc<super::layers::NodeRetained>> {
        self.retained.borrow().upgrade()
    }

    /// A cell with no parent and no marks. The caller attaches it to its
    /// parent via [`set_parent`](Self::set_parent).
    pub fn new(frames: FrameSignals, placement: Rc<Placement>) -> Rc<Self> {
        Rc::new(Self {
            own: Cell::new(Dirty::NONE),
            below: Cell::new(Dirty::NONE),
            parent: RefCell::new(None),
            frames,
            placement,
            scopes: RefCell::new(RetainedScopes::default()),
            retained: RefCell::new(std::rc::Weak::new()),
            registrations: RefCell::new(None),
            registered: Cell::new(false),
            children: RefCell::new(Vec::new()),
            subviews: RefCell::new(Vec::new()),
            material_group_scope: Cell::new(false),
            placed_seq: Cell::new(0),
            retired: Cell::new(false),
            #[cfg(feature = "accessibility")]
            a11y_unit: Cell::new(false),
            #[cfg(feature = "accessibility")]
            a11y_recording: Cell::new(false),
            #[cfg(feature = "accessibility")]
            a11y_emitted: RefCell::new(Vec::new()),
            #[cfg(feature = "accessibility")]
            a11y_retired: RefCell::new(None),
        })
    }

    /// The node's placement mirror.
    pub const fn placement(&self) -> &Rc<Placement> {
        &self.placement
    }

    /// The cell this node's parent attached it under — `None` at the root.
    /// The `.focused()` scope walk and the a11y unit rule climb it.
    pub fn parent(&self) -> Option<Rc<Self>> {
        self.parent.borrow().clone()
    }

    /// The `.material_group()` scope cells enclosing this node, outermost
    /// first — the state a partial descent replays onto
    /// `material_group_scopes` before it re-records the subtree so a member
    /// reads the same scope as under a full flush (#2268).
    pub(crate) fn enclosing_material_group_scopes(&self) -> Vec<Rc<Self>> {
        let mut scopes = Vec::new();
        let mut cursor = self.parent();
        while let Some(cell) = cursor {
            if cell.material_group_scope.get() {
                scopes.push(Rc::clone(&cell));
            }
            cursor = cell.parent();
        }
        scopes.reverse();
        scopes
    }

    /// Live child cells — pruned of dead entries as it is handed out, so
    /// `retire_subtree` walks only what still exists.
    pub(crate) fn children(&self, out: &mut Vec<Rc<Self>>) {
        self.children.borrow_mut().retain(|weak| {
            weak.upgrade().is_some_and(|child| {
                out.push(child);
                true
            })
        });
    }

    /// Decision 3's teardown for one cell: drops its `NodeLayers` —
    /// every `Layer`'s drop queues that layer's `Remove` on the shared
    /// surface, so this runs in the update phase and never inside a commit
    /// transaction body (§A.2) — and sets `PAINT|COMMIT` on the cell, so a
    /// later record remounts the node from scratch. The bits stay `own`:
    /// an unmounted subtree is not pending work for its live ancestors —
    /// raising `below` would hold the root dirty forever on a cell no
    /// flush visits — so the bits wait for the cell's own next record or
    /// re-attach, where `set_parent` propagates them up the new chain.
    /// The cell keeps its node and its retained scopes: only the engine
    /// memory follows off screen. A cell whose `NodeRetained` already died
    /// has no layers left to drop.
    pub(crate) fn unmount(&self) {
        if let Some(retained) = self.retained.borrow().upgrade() {
            let _ = retained.layers.borrow_mut().take();
        }
        self.own.set(self.own.get() | Dirty::PAINT | Dirty::COMMIT);
    }

    /// [`unmount`](Self::unmount) over this cell's whole subtree: every
    /// descendant the `set_parent` links reach — render children, and the
    /// `RetainedSubview` roots attached under it, which live in
    /// `children` alongside them.
    pub(crate) fn unmount_subtree(self: &Rc<Self>) {
        let mut stack = vec![Rc::clone(self)];
        while let Some(cell) = stack.pop() {
            cell.unmount();
            cell.children(&mut stack);
        }
    }

    /// Stamps that this cell's placement was linked under record `seq` —
    /// called by the placement links every flush of the node runs.
    pub(crate) fn mark_placed(&self, seq: u64) {
        self.placed_seq.set(seq);
        self.retired.set(false);
    }

    /// The record sequence this cell's placement was last linked under.
    pub(crate) const fn placed_seq(&self) -> u64 {
        self.placed_seq.get()
    }

    /// §B.4's unit escalation: the nearest strict ancestor flagged
    /// `a11y_unit`, else the root cell — the ultimate owner of collapse and
    /// claim decisions.
    #[cfg(feature = "accessibility")]
    pub fn nearest_a11y_unit_ancestor(self: &Rc<Self>) -> Rc<Self> {
        let mut cursor = self.parent();
        while let Some(cell) = cursor {
            if cell.a11y_unit.get() {
                return cell;
            }
            cursor = cell.parent();
        }
        let mut top = Rc::clone(self);
        while let Some(parent) = top.parent() {
            top = parent;
        }
        top
    }

    /// Attaches the cell to `parent`. Called while the parent builds its
    /// children, on subview/entry insertion, and when a reparent (matched
    /// geometry) moves the node. The placement mirror follows the same
    /// link. Marks raised while the cell was detached propagate up the new
    /// chain, so a mark can never be stranded below the attach point.
    pub fn set_parent(self: &Rc<Self>, parent: &Rc<Self>) {
        *self.parent.borrow_mut() = Some(Rc::clone(parent));
        parent.children.borrow_mut().push(Rc::downgrade(self));
        self.placement
            .set_parent(Some(Rc::clone(parent.placement())));
        let raised = self.own.get() | self.below.get();
        let raised_layout = self.own.get() & Dirty::LAYOUT;
        if raised.is_empty() && raised_layout.is_empty() {
            return;
        }
        // A `LAYOUT` in `own` rides ancestors' `own` (as `mark_layout`
        // raises it), not their `below`: a later `mark_layout` from below
        // must still climb past this attach point.
        let mut cursor = Some(Rc::clone(parent));
        while let Some(ancestor) = cursor {
            let below = ancestor.below.get();
            let own = ancestor.own.get();
            if below.contains(raised) && own.contains(raised_layout) {
                break;
            }
            ancestor.below.set(below | raised);
            ancestor.own.set(own | raised_layout);
            let next = ancestor.parent.borrow().clone();
            cursor = next;
        }
    }

    /// Propagates `dirty` as `below` up the ancestor chain, stopping at
    /// the first ancestor that already carries it — the loop [`Self::mark`]
    /// and [`Self::mark_quiet`] share.
    fn propagate_below(&self, dirty: Dirty) {
        let mut cursor = self.parent.borrow().clone();
        while let Some(ancestor) = cursor {
            let below = ancestor.below.get();
            if below.contains(dirty) {
                break;
            }
            ancestor.below.set(below | dirty);
            let next = ancestor.parent.borrow().clone();
            cursor = next;
        }
    }

    /// Marks `dirty` on the node and propagates it as `below` up the
    /// ancestor chain, stopping at the first ancestor that already carries
    /// it. Wakes the host so the pump runs.
    pub fn mark(&self, dirty: Dirty) {
        self.own.set(self.own.get() | dirty);
        self.propagate_below(dirty);
        self.frames.request_refresh();
    }

    /// [`Self::mark`] without the frame request: the dirty bits land for the
    /// descent to consume, but nothing is scheduled — used by marks raised
    /// inside work the pump already schedules as continuation (`Animate`),
    /// like the animation tick. Arming `request_refresh` there would read
    /// every animation frame as an unapplied change.
    pub fn mark_quiet(&self, dirty: Dirty) {
        self.own.set(self.own.get() | dirty);
        self.propagate_below(dirty);
    }

    /// Marks `LAYOUT` on the node and on every ancestor's `own` — a size or
    /// layout-input change invalidates the whole chain above it.
    pub fn mark_layout(&self) {
        self.own.set(self.own.get() | Dirty::LAYOUT);
        let mut cursor = self.parent.borrow().clone();
        while let Some(ancestor) = cursor {
            let own = ancestor.own.get();
            if own.contains(Dirty::LAYOUT) {
                break;
            }
            ancestor.own.set(own | Dirty::LAYOUT);
            let next = ancestor.parent.borrow().clone();
            cursor = next;
        }
        self.frames.request_refresh();
    }

    /// This node's own pending marks.
    pub const fn own(&self) -> Dirty {
        self.own.get()
    }

    /// Marks pending anywhere below this node.
    pub const fn below(&self) -> Dirty {
        self.below.get()
    }

    /// Clears this cell's marks; the frame's flush walk calls it on every
    /// node it visits so a handled mark never re-arms.
    pub fn clear_marks(&self) {
        self.own.set(Dirty::NONE);
        self.below.set(Dirty::NONE);
    }

    /// Clears the `bits` the mount commit consumed, keeping every other
    /// pending mark.
    pub fn clear_bits(&self, bits: Dirty) {
        self.own.set(self.own.get().without(bits));
        self.below.set(self.below.get().without(bits));
    }
}

/// The mount state every render node embeds: its cell and the signal
/// guards its last record and layout passes subscribed.
///
/// Every node struct carries `core: NodeCore`. `subscriptions` holds the
/// paint-phase signal guards — cleared at the start of each record and
/// dropped with the node — and `layout_subscriptions` the guards its
/// measure/layout calls installed, same lifecycle. The layer set
/// (`layers`), pending program (`pending`), retained scopes (`scopes`) and
/// the cell's `frame`/`a11y_unit` join in the commits that first write
/// them.
///
/// `Clone` is a same-identity copy — every field is an `Rc`, so a cloned
/// core names the same cell and stores (used where a `&mut` node borrow
/// would otherwise conflict with the reader the core anchors).
#[derive(Clone)]
pub struct NodeCore {
    /// The node's cell: identity, marks and placement mirror.
    pub cell: Rc<NodeCell>,
    /// The node's retained layers and pending program (§A).
    pub retained: Rc<super::layers::NodeRetained>,
    /// Signal guards from the node's last record (paint phase). `Rc` so the
    /// live reader can point a store clone at it while the guard closures
    /// run.
    pub subscriptions: Rc<RefCell<Vec<Retain>>>,
    /// Signal guards from the node's last measure/layout (layout phase).
    pub layout_subscriptions: Rc<RefCell<Vec<Retain>>>,
}

impl NodeCore {
    /// A fresh core with an unattached cell.
    pub fn new(frames: FrameSignals, placement: Rc<Placement>) -> Self {
        let cell = NodeCell::new(frames, placement);
        let retained = Rc::new(super::layers::NodeRetained::default());
        *cell.retained.borrow_mut() = Rc::downgrade(&retained);
        Self {
            cell,
            retained,
            subscriptions: Rc::new(RefCell::new(Vec::new())),
            layout_subscriptions: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A second core naming `cell` — the presentation hosts and the
    /// window root's own record read through it, each with their own
    /// subscription stores.
    pub(crate) fn for_cell(cell: &Rc<NodeCell>) -> Self {
        let live = cell.retained.borrow().upgrade();
        let retained = live.unwrap_or_else(|| {
            let retained = Rc::new(super::layers::NodeRetained::default());
            *cell.retained.borrow_mut() = Rc::downgrade(&retained);
            retained
        });
        Self {
            cell: Rc::clone(cell),
            retained,
            subscriptions: Rc::new(RefCell::new(Vec::new())),
            layout_subscriptions: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A detached core for tests that build node structs by hand: its own
    /// frame signals and placement clock, no parent. Marks raised on it
    /// reach no window.
    #[cfg(test)]
    pub fn detached() -> Self {
        Self::new(
            FrameSignals::new(std::time::Instant::now()),
            Placement::new(&super::placement::PlacementClock::new()),
        )
    }
}
