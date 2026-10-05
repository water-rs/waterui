//! The placement mirror: layer props as retained data.
//!
//! Every [`NodeCell`](super::cell::NodeCell) owns a [`Placement`] that
//! mirrors the props its layers carry — transform, clip bounding box,
//! alpha, hittable, child index — so hit testing, hit-space transforms and
//! GPU pixel coverage resolve them without walking the render tree. Each
//! setter also bumps the window's [`PlacementClock`], the epoch
//! resolutions cache against.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The per-window epoch that placement resolutions cache against.
///
/// Every [`Placement`] setter bumps it, so any placement change invalidates
/// every cached resolution in the window at once — cheaper than tracking
/// per-chain dependencies.
pub struct PlacementClock(Cell<u64>);

impl PlacementClock {
    /// A clock at epoch zero.
    pub fn new() -> Rc<Self> {
        Rc::new(Self(Cell::new(0)))
    }

    /// Advances the epoch; every setter calls it.
    pub fn bump(&self) {
        self.0.set(
            self.0
                .get()
                .checked_add(1)
                .expect("hydrolysis placement clock overflow"),
        );
    }

    /// The current epoch.
    pub const fn epoch(&self) -> u64 {
        self.0.get()
    }
}

/// The hit-test ceiling applied to the placement chain: a registration on a
/// chain whose alpha product falls at or below this is resolved as
/// unhittable, matching the per-frame `HIT_TEST_ALPHA_THRESHOLD` sweep it
/// replaced.
pub use crate::renderer::HIT_TEST_ALPHA_THRESHOLD as PLACEMENT_HIT_ALPHA_THRESHOLD;

/// One node's retained mirror of its layer-space placement.
///
/// The structural parent link mirrors the cell tree and is set when the
/// cell attaches; `paint_parent` overrides it for the props that move
/// drawing (a matched-geometry reparent) while hit resolution still walks
/// `parent`. Scopes a node pushes inside its own record get child
/// placements the same way, so a registration always resolves through the
/// chain it was emitted under.
///
/// The props mirror the layer edits the commit walk writes: `transform`
/// is the node's own delta in its parent's space, `clip` a clip bounding
/// box in the node's *own* space, `alpha`/`hittable` the chain factors
/// hit resolution multiplies and gates on, `index` the position among the
/// parent's ordered items.
pub struct Placement {
    /// The structural parent in the placement chain — the parent cell's
    /// placement, set when the cell attaches.
    parent: RefCell<Option<Rc<Self>>>,
    /// The paint-space parent when it differs from `parent` (matched
    /// geometry during a navigation transition). `None` = `parent`.
    paint_parent: RefCell<Option<Rc<Self>>>,
    /// The node's transform in its parent's space.
    transform: Cell<kurbo::Affine>,
    /// Content offset applied inside the node's own frame (scroll).
    content_offset: Cell<kurbo::Vec2>,
    /// A clip bounding box in the node's own space.
    clip: Cell<Option<kurbo::Rect>>,
    /// The alpha the node's subtree multiplies into its chain.
    alpha: Cell<f32>,
    /// Whether this subtree's registrations hit-test. `false` levels are
    /// skipped entirely by hit resolution (transitioning pages, exiting
    /// overlays).
    hittable: Cell<bool>,
    /// Position among the parent's ordered items — paint order.
    index: Cell<u32>,
    /// The next item position this placement hands out: children link as
    /// items of it, and a registration anchored at it ranks at the cursor
    /// the items took their indices from, so content registered before,
    /// between or after children orders the same as its emission.
    item_cursor: Cell<u32>,
    /// The window clock every write bumps.
    clock: Rc<PlacementClock>,
    /// The hit chain's `(epoch, transform, clip, alpha)`, which skips
    /// `hittable == false` levels — recomputed lazily per clock epoch.
    resolved_hit: RefCell<Option<ResolvedCache>>,
}

/// The `(epoch, transform, clip, alpha)` tuple a resolution caches.
type ResolvedCache = (u64, kurbo::Affine, Option<kurbo::Rect>, f32);

impl Placement {
    /// A placement on `clock`, unattached, identity transform, no clip,
    /// full alpha, hittable, index 0.
    pub fn new(clock: &Rc<PlacementClock>) -> Rc<Self> {
        Rc::new(Self {
            parent: RefCell::new(None),
            paint_parent: RefCell::new(None),
            transform: Cell::new(kurbo::Affine::IDENTITY),
            content_offset: Cell::new(kurbo::Vec2::ZERO),
            clip: Cell::new(None),
            alpha: Cell::new(1.0),
            hittable: Cell::new(true),
            index: Cell::new(0),
            item_cursor: Cell::new(0),
            clock: Rc::clone(clock),
            resolved_hit: RefCell::new(None),
        })
    }

    /// Attaches under `parent` (the parent cell's placement). Attaching is
    /// a placement write, so the shared clock bumps.
    pub fn set_parent(&self, parent: Option<Rc<Self>>) {
        *self.parent.borrow_mut() = parent;
        self.clock.bump();
    }

    /// The structural parent, for index-path composition.
    pub fn parent(&self) -> Option<Rc<Self>> {
        self.parent.borrow().clone()
    }

    /// The position among the parent's ordered items.
    pub const fn index(&self) -> u32 {
        self.index.get()
    }

    /// The item position a registration anchored at this placement ranks
    /// at — the cursor the items already linked took their indices from.
    pub(crate) const fn item_cursor(&self) -> u32 {
        self.item_cursor.get()
    }

    /// Claims the next item index on this placement — every child linking
    /// as an item draws its position from here, so a registration emitted
    /// at the same anchor ranks among the items exactly where it emitted.
    pub(crate) fn take_item(&self) -> u32 {
        let index = self.item_cursor.get();
        self.item_cursor.set(
            index
                .checked_add(1)
                .expect("hydrolysis placement: item index overflow"),
        );
        index
    }

    /// The transform the hit chain resolves to — for consumers that need
    /// the chain's own value (embedded targets' inverse projection), not
    /// a resolved rect.
    pub fn resolved_transform(&self, hit: bool) -> kurbo::Affine {
        let (transform, _, _) = self.resolve_chain(hit);
        transform
    }

    /// The `(transform, clip, alpha)` the chain resolves to, `hit == true`
    /// skipping `hittable == false` levels — the raw form behind
    /// [`resolve`](Self::resolve) for consumers that need the clip itself
    /// (embedded targets' hit clip) or the transform (inverse projection).
    pub fn resolved_chain(&self, hit: bool) -> (kurbo::Affine, Option<kurbo::Rect>, f32) {
        self.resolve_chain(hit)
    }

    /// Sets the node's transform in its parent's space.
    pub fn set_transform(&self, transform: kurbo::Affine) {
        self.transform.set(transform);
        self.clock.bump();
    }

    /// Sets the clip bounding box in the node's own space.
    pub fn set_clip(&self, clip: Option<kurbo::Rect>) {
        self.clip.set(clip);
        self.clock.bump();
    }

    /// Sets the subtree alpha factor.
    pub fn set_alpha(&self, alpha: f32) {
        self.alpha.set(alpha);
        self.clock.bump();
    }

    /// Sets whether this subtree hit-tests.
    pub fn set_hittable(&self, hittable: bool) {
        self.hittable.set(hittable);
        self.clock.bump();
    }

    /// Sets the position among the parent's ordered items.
    pub fn set_index(&self, index: u32) {
        self.index.set(index);
        self.clock.bump();
    }

    /// Resets the props the node writes about itself — content offset,
    /// clip, alpha, hittable — to defaults, keeping `transform`,
    /// `parent`, `paint_parent` and `index`: the structural facts the
    /// parent writes at the child boundary or the cell keeps for its
    /// lifetime. The `item_cursor` resets too: the record re-deals its
    /// item positions from zero, mirroring its emission order. Called
    /// when the owning node starts a record, so the self-props mirror
    /// exactly what this record writes.
    pub fn reset_props(&self) {
        self.content_offset.set(kurbo::Vec2::ZERO);
        self.clip.set(None);
        self.alpha.set(1.0);
        self.hittable.set(true);
        self.item_cursor.set(0);
        self.clock.bump();
    }

    /// Resolves a rect from this placement's local space to window space
    /// through the hit chain: `hittable == false` levels are skipped
    /// entirely, and the alpha chain gates the result.
    ///
    /// The result is cached against the clock epoch: any placement write
    /// anywhere in the window invalidates it.
    pub fn resolve_hit(&self, local: kurbo::Rect) -> Option<ResolvedPlacement> {
        let epoch = self.clock.epoch();
        let cached = {
            let cache = self.resolved_hit.borrow();
            cache
                .filter(|entry| entry.0 == epoch)
                .map(|entry| (entry.1, entry.2, entry.3))
        };
        let (transform, clip, alpha) = cached.unwrap_or_else(|| self.resolve_chain(true));
        *self.resolved_hit.borrow_mut() = Some((epoch, transform, clip, alpha));
        if alpha <= PLACEMENT_HIT_ALPHA_THRESHOLD {
            return None;
        }
        let mut rect = crate::renderer::transformed_rect(transform, local);
        if let Some(clip) = clip {
            rect = rect.intersect(clip);
        }
        Some(ResolvedPlacement { rect })
    }

    /// Composes `(transform, clip, alpha)` down the chain, `hit == true`
    /// skipping levels whose `hittable` flag is cleared.
    ///
    /// The walk collects `Rc` clones bottom-up then composes top-down, so
    /// no level can disappear mid-resolve.
    fn resolve_chain(&self, hit: bool) -> (kurbo::Affine, Option<kurbo::Rect>, f32) {
        let mut ancestors: Vec<Rc<Self>> = Vec::new();
        let mut parent = self.parent_for(hit);
        while let Some(placement) = parent {
            parent = placement.parent_for(hit);
            ancestors.push(placement);
        }
        let mut transform = kurbo::Affine::IDENTITY;
        let mut clip: Option<kurbo::Rect> = None;
        let mut alpha = 1.0f32;
        for placement in ancestors
            .iter()
            .rev()
            .map(Rc::as_ref)
            .chain(std::iter::once(self))
        {
            if hit && !placement.hittable.get() {
                // An unhittable level removes the whole subtree's chain.
                return (kurbo::Affine::IDENTITY, None, 0.0);
            }
            transform *= placement.transform.get()
                * kurbo::Affine::translate(placement.content_offset.get());
            alpha *= placement.alpha.get();
            if let Some(rect) = placement.clip.get() {
                // The clip lives in the placement's own space: compose the
                // chain up to this level, then intersect in window space.
                let window_clip = crate::renderer::transformed_rect(transform, rect);
                clip = Some(clip.map_or(window_clip, |c| c.intersect(window_clip)));
            }
        }
        (transform, clip, alpha)
    }

    /// The chain link one level up: `parent`, or `paint_parent` when the
    /// paint chain resolves (`hit == false`) and an override is set.
    fn parent_for(&self, hit: bool) -> Option<Rc<Self>> {
        if hit {
            self.parent.borrow().clone()
        } else {
            let paint = self.paint_parent.borrow();
            // `paint_parent` overrides the structural link for drawing;
            // `None` means the placement follows `parent`.
            paint.clone().or_else(|| self.parent.borrow().clone())
        }
    }
}

/// A resolved placement: the window-space rect after the chain's transform
/// and clip composition.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedPlacement {
    /// The rect in window space, already intersected with the clip chain.
    pub rect: kurbo::Rect,
}
