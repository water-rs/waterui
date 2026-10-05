//! The retained registries: every registration belongs to the node that
//! recorded it.
//!
//! The node's cell holds an [`OwnerRegistrations`] — one bucket per
//! registration kind — filled while it records and replaced wholesale on
//! re-record, so a node retires exactly its own entries in O(its count)
//! with no registry sweep. The window's [`RetainedRegistry`] keeps only
//! the enumeration of owners that have ever registered and a staleness
//! flag; materialization merges the live owners' buckets into the flat,
//! paint-ordered lists the consumers read.
//!
//! Ordering is the [`PaintOrder`] key: the anchor's index path — the
//! placements walked up to the window root — plus the position among the
//! anchor's items the entry emitted at, so a registration emitted before,
//! between or after the record's children ranks exactly as its emission
//! does. Entries that share the position (two registrations with no child
//! link between them) tie-break on a global registration sequence — no
//! two entries compare equal.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use super::cell::NodeCell;
use super::placement::Placement;

/// A hit region registered while a node recorded: the rect in the
/// recording node's own space and the placement it resolves through.
///
/// The placement is `Rc`-retained: a registration keeps its resolve
/// chain alive even after its owner node is gone, so a frame racing a
/// node drop resolves to the last seen position rather than panicking —
/// the owner `Weak` is what drops the entry.
#[derive(Clone)]
pub struct Region {
    /// The rect in the registering node's local space.
    pub local: kurbo::Rect,
    /// The placement chain the rect resolves through.
    pub placement: Rc<Placement>,
}

impl std::fmt::Debug for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Region")
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

/// Where a retained entry sits in paint order: the placement index path
/// from the window root to the entry's anchor, then the anchor's item
/// position the entry emitted at, then a global sequence.
///
/// Children link as items of their parent's placement, drawing their
/// `index` from the same cursor an entry ranks at — so an entry emitted
/// before a child ranks below that child's whole subtree, and one emitted
/// after it ranks above, exactly the order the flat lists assign. `reg`
/// is a per-window monotonic that makes every ordering total: no two
/// entries tie.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PaintOrder {
    /// The index path from the window root to the anchor placement,
    /// outermost first, with the anchor's item position appended.
    path: Vec<u32>,
    /// The global registration sequence — the tie-break among entries at
    /// the same item position.
    reg: u64,
}

impl PaintOrder {
    /// Composes the paint-order key for an entry pushed while `placement`
    /// is the deepest placement of its owner's record.
    pub fn new(placement: &Rc<Placement>, reg: u64) -> Self {
        let mut path = Vec::new();
        let mut node = Some(Rc::clone(placement));
        while let Some(placement) = node {
            path.push(placement.index());
            node = placement.parent();
        }
        path.reverse();
        path.push(placement.item_cursor());
        Self { path, reg }
    }
}

/// One retained registration: today's flat payload plus the fields the
/// per-node model needs — the node-local [`Region`], the owning cell and
/// the paint-order key.
pub struct RetainedEntry<T> {
    /// The registration's data, unchanged from the flat-list type; the
    /// fields holding a resolved window rect are rewritten at
    /// materialization.
    pub payload: T,
    /// The region it resolves through.
    pub region: Region,
    /// The node that emitted it; a dead owner drops the entry at the
    /// next materialization.
    pub owner: Weak<NodeCell>,
    /// Its paint-order key — materialization sorts by it.
    pub order: PaintOrder,
}

/// The per-owner buckets: one `Vec` per registration kind, appended in
/// emission order during the owner's record. Owned by the cell, taken as
/// a unit on re-record or retire.
#[derive(Default)]
pub struct OwnerRegistrations {
    /// Pointer targets, in emission order.
    pub pointer_targets: Vec<RetainedEntry<crate::renderer::PointerTarget>>,
    /// System-back targets, frontmost last after materialization.
    pub back_targets: Vec<RetainedEntry<crate::renderer::NavigationBackTarget>>,
    /// Gesture-recognizer regions, parallel to `pointer_targets`.
    pub gesture_regions: Vec<RetainedEntry<RegisteredGesture>>,
    /// Overlay occluders: `(bounds, order)` in the flat list — the
    /// retained form needs no payload beyond the region.
    pub gesture_occluders: Vec<RetainedEntry<()>>,
    /// Cursor-style targets.
    pub cursor_targets: Vec<RetainedEntry<crate::renderer::CursorTarget>>,
    /// Hover targets.
    pub hover_targets: Vec<RetainedEntry<crate::renderer::HoverTarget>>,
    /// Drop destinations.
    pub drop_targets: Vec<RetainedEntry<crate::renderer::DropTarget>>,
    /// Context-menu targets.
    pub context_menu_targets: Vec<RetainedEntry<crate::renderer::ContextMenuTarget>>,
    /// Scroll targets.
    pub scroll_targets: Vec<RetainedEntry<crate::renderer::ScrollTarget>>,
    /// Text-input targets: the payload keeps its rects in the node's
    /// local space; materialization resolves them through the region.
    pub text_input_targets: Vec<RetainedEntry<crate::renderer::TextInputTarget>>,
    /// Embedded-surface input targets.
    pub embedded_input_targets: Vec<RetainedEntry<crate::renderer::EmbeddedInputTarget>>,
    /// Native-subview occlusions.
    pub native_view_occlusions: Vec<RetainedEntry<crate::renderer::NativeViewOcclusion>>,
    /// Embedded platform-view placements.
    pub platform_view_placements: Vec<RetainedEntry<PlatformViewRegistration>>,
    /// Modal scopes — the single flat `modal_interaction` slot keeps
    /// only the frontmost live registration.
    pub modal_scope: Vec<RetainedEntry<waterui_backend_core::widget::ModalInteraction>>,
}

/// The window-side half of the retained model: which cells have ever
/// registered (the materialization walks them), whether anything changed
/// since the last materialization, and the registration sequence that
/// gives every entry a distinct paint-order tail.
pub struct RetainedRegistry {
    /// Cells that hold registrations, in first-registration order.
    /// Dead cells are pruned where the enumeration is walked.
    owners: Vec<Weak<NodeCell>>,
    /// Set whenever any bucket mutates; the next materialization rebuilds
    /// the flat lists.
    pub stale: Cell<bool>,
    /// The `reg` tail of each [`PaintOrder`]: bumped once per
    /// registration, so no two entries ever tie.
    reg_seq: Cell<u64>,
}

impl RetainedRegistry {
    /// An empty registry.
    pub const fn new() -> Self {
        Self {
            owners: Vec::new(),
            stale: Cell::new(true),
            reg_seq: Cell::new(0),
        }
    }

    /// The next registration sequence — assigned at registration, never
    /// reused.
    pub(crate) fn next_seq(&self) -> u64 {
        let seq = self.reg_seq.get();
        self.reg_seq.set(
            seq.checked_add(1)
                .expect("hydrolysis retained registry: sequence overflow"),
        );
        seq
    }

    /// Lists `cell` among the registering owners; no-op once listed.
    pub(crate) fn enlist(&mut self, cell: &Rc<NodeCell>) {
        if cell.registered.replace(true) {
            return;
        }
        self.owners.push(Rc::downgrade(cell));
    }

    /// The live registering owners — dead cells are pruned from the list
    /// as it is handed out.
    pub(crate) fn owners(&mut self) -> Vec<Rc<NodeCell>> {
        let mut live = Vec::with_capacity(self.owners.len());
        self.owners.retain(|weak| {
            weak.upgrade().is_some_and(|cell| {
                live.push(cell);
                true
            })
        });
        live
    }
}

impl<T> RetainedEntry<T> {
    /// A registration anchored at `placement`, recorded by `owner` at
    /// registration sequence `reg`. `local` is in the anchor's own space.
    pub fn at(
        mut payload: T,
        local: kurbo::Rect,
        placement: &Rc<Placement>,
        owner: &Rc<NodeCell>,
        reg: u64,
    ) -> Self
    where
        T: SetRegistrationOwner,
    {
        payload.set_registration_owner(&Rc::downgrade(owner));
        Self {
            payload,
            region: Region {
                local,
                placement: Rc::clone(placement),
            },
            owner: Rc::downgrade(owner),
            order: PaintOrder::new(placement, reg),
        }
    }
}

/// A gesture registration's retained payload: the engine-side target
/// (sharing the recognizer created at registration) plus the owner chain
/// the press path's ancestry check reads.
pub struct RegisteredGesture {
    /// The node cell that registered the gesture.
    pub owner: Weak<NodeCell>,
    /// The engine-side target; bounds and order are rewritten at
    /// materialization.
    pub target: crate::gesture::GestureTarget,
    /// The owner chain the registration ran under, innermost last.
    pub owners: Vec<crate::renderer::RetainedIdentity>,
}

/// An embedded platform-view leaf's retained payload: the placement record
/// with its geometry in node-local coordinates. Materialization resolves
/// the window-space rect, clip and paint rank; the frame-end step then
/// writes it into `table` once.
pub struct PlatformViewRegistration {
    /// The sink table the record publishes into.
    pub table: Rc<RefCell<crate::platform_view::PlatformViewTable>>,
    /// The leaf's placement, `x`/`y`/`width`/`height`/`clip`/`order`/
    /// `visible` rewritten at materialization.
    pub placement: crate::platform_view::PlatformViewPlacement,
}

/// Tags a retained payload with its registering node so dispatch can mark
/// exactly the cell whose record emitted it. Payloads no dispatch path
/// marks (occlusions, placements, the modal scope) get the no-op impl.
pub trait SetRegistrationOwner {
    /// Stores `owner` on the payload; the no-op impl is for payloads no
    /// dispatch path marks.
    fn set_registration_owner(&mut self, owner: &Weak<NodeCell>) {
        let _ = owner;
    }
}

macro_rules! owner_tagged {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SetRegistrationOwner for $ty {
                fn set_registration_owner(&mut self, owner: &Weak<NodeCell>) {
                    self.owner = owner.clone();
                }
            }
        )+
    };
}

owner_tagged!(
    crate::renderer::input::PointerTarget,
    crate::renderer::input::CursorTarget,
    crate::renderer::input::HoverTarget,
    crate::renderer::input::DropTarget,
    crate::renderer::input::ScrollTarget,
    crate::renderer::input::ContextMenuTarget,
    crate::renderer::NavigationBackTarget,
    crate::renderer::input::TextInputTarget,
    crate::renderer::input::EmbeddedInputTarget,
    RegisteredGesture,
);

impl SetRegistrationOwner for () {}
impl SetRegistrationOwner for crate::renderer::input::NativeViewOcclusion {}
impl SetRegistrationOwner for waterui_backend_core::widget::ModalInteraction {}
impl SetRegistrationOwner for PlatformViewRegistration {}
