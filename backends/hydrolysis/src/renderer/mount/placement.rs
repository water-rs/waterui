//! The placement mirror: layer props as retained data.
//!
//! Every [`NodeCell`](super::cell::NodeCell) owns a [`Placement`] that
//! mirrors the props its layers carry — transform, content offset, clip
//! bounding box, alpha, hittable, child index — so hit testing, hit-space
//! transforms and GPU pixel coverage resolve them without walking the
//! render tree. Each setter also bumps the window's [`PlacementClock`], the
//! epoch resolutions cache against.
//!
//! The commit adds the type and its structural parent chain; the prop
//! fields, setters and `resolve`/`resolve_hit` composition join with the
//! placement-writing walk that first reads them.

use std::cell::RefCell;
use std::rc::Rc;

/// The per-window epoch that placement resolutions cache against.
///
/// Every [`Placement`] setter bumps it, so any placement change invalidates
/// every cached resolution in the window at once — cheaper than tracking
/// per-chain dependencies.
pub struct PlacementClock(std::cell::Cell<u64>);

impl PlacementClock {
    /// A clock at epoch zero.
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self(std::cell::Cell::new(0)))
    }

    /// Advances the epoch; every setter calls it.
    pub(crate) fn bump(&self) {
        self.0.set(
            self.0
                .get()
                .checked_add(1)
                .expect("hydrolysis placement clock overflow"),
        );
    }
}

/// One node's retained mirror of its layer-space placement.
///
/// The structural parent link mirrors the cell tree and is set when the
/// cell attaches. The prop mirrors themselves (transform, clip, alpha,
/// hittable, index) and the top-down `resolve`/`resolve_hit` composition
/// join with the placement-writing walk that first reads them.
pub struct Placement {
    /// The structural parent in the placement chain — the parent cell's
    /// placement, set when the cell attaches.
    parent: RefCell<Option<Rc<Self>>>,
    /// The window clock every write bumps.
    clock: Rc<PlacementClock>,
}

impl Placement {
    /// A placement on `clock`, unattached.
    pub(crate) fn new(clock: &Rc<PlacementClock>) -> Rc<Self> {
        Rc::new(Self {
            parent: RefCell::new(None),
            clock: Rc::clone(clock),
        })
    }

    /// Attaches under `parent` (the parent cell's placement). Attaching is
    /// a placement write, so the shared clock bumps.
    pub(crate) fn set_parent(&self, parent: Option<Rc<Self>>) {
        *self.parent.borrow_mut() = parent;
        self.clock.bump();
    }
}
