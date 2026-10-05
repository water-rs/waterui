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
use std::rc::Rc;

use waterui_core::Retain;

use waterui_backend_core::frame_signals::FrameSignals;

use super::placement::Placement;

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
    pub(crate) const NONE: Self = Self(0);
    /// The subtree shape changed: children must be rebuilt.
    pub(crate) const STRUCTURE: Self = Self(1 << 0);
    /// Layout inputs changed: the node (and every ancestor's `own`) must
    /// re-measure.
    pub(crate) const LAYOUT: Self = Self(1 << 1);
    /// Recorded drawing changed: the node re-records its runs.
    pub(crate) const PAINT: Self = Self(1 << 2);
    /// A GPU/external-frame producer published work for the install layer.
    pub(crate) const PRODUCER: Self = Self(1 << 4);

    /// Whether every bit in `other` is set in `self`.
    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no bit is set.
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
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
}

impl NodeCell {
    /// A cell with no parent and no marks. The caller attaches it to its
    /// parent via [`set_parent`](Self::set_parent).
    pub(crate) fn new(frames: FrameSignals, placement: Rc<Placement>) -> Rc<Self> {
        Rc::new(Self {
            own: Cell::new(Dirty::NONE),
            below: Cell::new(Dirty::NONE),
            parent: RefCell::new(None),
            frames,
            placement,
        })
    }

    /// The node's placement mirror.
    pub(crate) const fn placement(&self) -> &Rc<Placement> {
        &self.placement
    }

    /// Attaches the cell to `parent`. Called while the parent builds its
    /// children, on subview/entry insertion, and when a reparent (matched
    /// geometry) moves the node. The placement mirror follows the same
    /// link. Marks raised while the cell was detached propagate up the new
    /// chain, so a mark can never be stranded below the attach point.
    pub(crate) fn set_parent(&self, parent: &Rc<Self>) {
        *self.parent.borrow_mut() = Some(Rc::clone(parent));
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

    /// Marks `dirty` on the node and propagates it as `below` up the
    /// ancestor chain, stopping at the first ancestor that already carries
    /// it. Wakes the host so the pump runs.
    pub(crate) fn mark(&self, dirty: Dirty) {
        self.own.set(self.own.get() | dirty);
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
        self.frames.request_refresh();
    }

    /// Marks `LAYOUT` on the node and on every ancestor's `own` — a size or
    /// layout-input change invalidates the whole chain above it.
    pub(crate) fn mark_layout(&self) {
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
    pub(crate) const fn own(&self) -> Dirty {
        self.own.get()
    }

    /// Marks pending anywhere below this node.
    pub(crate) const fn below(&self) -> Dirty {
        self.below.get()
    }

    /// Clears this cell's marks; the frame's flush walk calls it on every
    /// node it visits so a handled mark never re-arms.
    pub(crate) fn clear_marks(&self) {
        self.own.set(Dirty::NONE);
        self.below.set(Dirty::NONE);
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
    pub(crate) cell: Rc<NodeCell>,
    /// Signal guards from the node's last record (paint phase). `Rc` so the
    /// live reader can point a store clone at it while the guard closures
    /// run.
    pub(crate) subscriptions: Rc<RefCell<Vec<Retain>>>,
    /// Signal guards from the node's last measure/layout (layout phase).
    pub(crate) layout_subscriptions: Rc<RefCell<Vec<Retain>>>,
}

impl NodeCore {
    /// A fresh core with an unattached cell.
    pub(crate) fn new(frames: FrameSignals, placement: Rc<Placement>) -> Self {
        Self {
            cell: NodeCell::new(frames, placement),
            subscriptions: Rc::new(RefCell::new(Vec::new())),
            layout_subscriptions: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A detached core for tests that build node structs by hand: its own
    /// frame signals and placement clock, no parent. Marks raised on it
    /// reach no window.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self::new(
            FrameSignals::new(std::time::Instant::now()),
            Placement::new(&super::placement::PlacementClock::new()),
        )
    }
}
