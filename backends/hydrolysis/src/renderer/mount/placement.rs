//! The placement mirror: layer props as retained data.
//!
//! Every [`NodeCell`](super::cell::NodeCell) owns a [`Placement`] that
//! mirrors the props its layers carry — transform, clip bounding box,
//! alpha, removed hit classes, child index — so hit testing, hit-space transforms and
//! GPU pixel coverage resolve them without walking the render tree. Each
//! setter also bumps the window's [`PlacementClock`], the epoch
//! resolutions cache against.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::motion::LayerMotion;
use super::stretch::ShownStretch;
use crate::time::Instant;

/// The per-window epoch that placement resolutions cache against, and the
/// instant they evaluate engine-run motion at.
///
/// Every [`Placement`] setter bumps it, so any placement change invalidates
/// every cached resolution in the window at once — cheaper than tracking
/// per-chain dependencies.
///
/// A wrapper's transform motion (water-rs/waterui#2203) moves its layer on
/// the engine's clock with no placement write, so placement evaluates it at
/// the clock's instant: the last event's ([`Self::advance`]). Moving the
/// instant bumps the epoch only while a track is in flight, and a frame
/// re-evaluates once a track ends ([`Self::settle`]) — never every frame.
pub struct PlacementClock {
    epoch: Cell<u64>,
    /// The instant motion evaluates at; `None` before the first event,
    /// which evaluates every motion at rest.
    instant: Cell<Option<Instant>>,
    /// When each track started since the last evaluation is done.
    settles: RefCell<Vec<Instant>>,
}

impl PlacementClock {
    /// A clock at epoch zero.
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            epoch: Cell::new(0),
            instant: Cell::new(None),
            settles: RefCell::new(Vec::new()),
        })
    }

    /// Advances the epoch; every setter calls it.
    pub fn bump(&self) {
        self.epoch.set(
            self.epoch
                .get()
                .checked_add(1)
                .expect("hydrolysis placement clock overflow"),
        );
    }

    /// The current epoch.
    pub const fn epoch(&self) -> u64 {
        self.epoch.get()
    }

    /// The instant placement evaluates motion at.
    pub const fn instant(&self) -> Option<Instant> {
        self.instant.get()
    }

    /// A motion changed: it moved, and — with `until` — a track runs until
    /// that instant.
    pub fn note_motion(&self, until: Option<Instant>) {
        if let Some(until) = until {
            self.settles.borrow_mut().push(until);
        }
        self.bump();
    }

    /// Evaluates motion at `now` from here on: an event at `now` asks where
    /// things are. Bumps the epoch when a track was in flight at the
    /// instant placement last evaluated at, since it has moved since.
    pub fn advance(&self, now: Instant) {
        let previous = self.instant.replace(Some(now));
        if previous == Some(now) {
            return;
        }
        let mut settles = self.settles.borrow_mut();
        let moved = settles
            .iter()
            .any(|until| previous.is_none_or(|previous| *until > previous));
        settles.retain(|until| *until > now);
        drop(settles);
        if moved {
            self.bump();
        }
    }

    /// A frame at `now`: re-evaluates at `now` only when a track that was
    /// in flight at the last evaluation is done by then, so the rest
    /// placement every track ends at is resolved once.
    pub fn settle(&self, now: Instant) {
        if self.settles.borrow().iter().any(|until| *until <= now) {
            self.advance(now);
        }
    }
}

/// The hit-test ceiling applied to the placement chain: a registration on a
/// chain whose alpha product falls at or below this is resolved as
/// unhittable, matching the per-frame `HIT_TEST_ALPHA_THRESHOLD` sweep it
/// replaced.
pub use crate::renderer::HIT_TEST_ALPHA_THRESHOLD as PLACEMENT_HIT_ALPHA_THRESHOLD;

/// The registration kinds a placement level can remove from
/// materialization — the lists dev's per-frame registration either
/// truncated (`Hittable(false)`, the context-menu preview) or gated on
/// `hit_test_opacity` (inactive navigation pages, exiting overlays,
/// collection entries mid-transition).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HitClasses(u8);

impl HitClasses {
    /// No kind.
    pub const NONE: Self = Self(0);
    /// Pointer targets (occluder and scrollbar presses included), gesture
    /// regions, cursor, hover and scroll targets, text and embedded inputs.
    pub const INPUT: Self = Self(1);
    /// Drop targets.
    pub const DROP: Self = Self(1 << 1);
    /// Context-menu targets.
    pub const CONTEXT_MENU: Self = Self(1 << 2);
    /// Overlay panels' gesture occluders.
    pub const GESTURE_OCCLUDER: Self = Self(1 << 3);
    /// Native-view occlusions.
    pub const NATIVE_OCCLUSION: Self = Self(1 << 4);
    /// The kinds dev registered only while `hit_test_opacity` stayed above
    /// [`PLACEMENT_HIT_ALPHA_THRESHOLD`]; the chain's alpha gates exactly
    /// these.
    pub const OPACITY_GATED: Self =
        Self(Self::INPUT.0 | Self::DROP.0 | Self::CONTEXT_MENU.0 | Self::GESTURE_OCCLUDER.0);

    /// The kinds in either set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether the two sets share a kind.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// The hit gate a scope places over the registrations recorded inside it —
/// one per dev mechanism, so each keeps exactly dev's coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitGate {
    /// `.hittable(false)`: dev truncated the [`HitClasses::INPUT`] lists.
    Unhittable,
    /// An inactive navigation page or an exiting overlay: dev set
    /// `hit_test_opacity` to zero, gating [`HitClasses::OPACITY_GATED`].
    Inactive,
    /// The lifted context-menu preview: dev truncated the input lists plus
    /// drop, context-menu and native-occlusion registrations.
    Preview,
}

impl HitGate {
    /// The hit alpha the gate's scope multiplies into its chain.
    pub const fn alpha(self) -> f32 {
        match self {
            Self::Unhittable | Self::Preview => 1.0,
            Self::Inactive => 0.0,
        }
    }

    /// The kinds the gate's scope removes outright.
    pub const fn removes(self) -> HitClasses {
        match self {
            Self::Unhittable => HitClasses::INPUT,
            Self::Inactive => HitClasses::NONE,
            Self::Preview => HitClasses::INPUT
                .union(HitClasses::DROP)
                .union(HitClasses::CONTEXT_MENU)
                .union(HitClasses::NATIVE_OCCLUSION),
        }
    }
}

/// What a clip/alpha scope contributes to the placement chain, passed
/// explicitly by the caller that opens it: `transform` is a delta in the
/// owning node's record space (never derived from the paint transform),
/// `hit_alpha` the factor the hit gate multiplies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeDelta {
    /// The scope's transform relative to the placement it opens under.
    pub transform: kurbo::Affine,
    /// The alpha folded into the hit gate — `1.0` for Opacity, Scroll and
    /// Clip, as on dev; a collection entry carries its transition factor.
    pub hit_alpha: f32,
}

impl ScopeDelta {
    /// A scope that adds no transform and no hit fade: the clip and the
    /// opacity of an ordinary layer live in the space the node records in.
    pub const RECORD_SPACE: Self = Self {
        transform: kurbo::Affine::IDENTITY,
        hit_alpha: 1.0,
    };
}

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
/// box in the node's *own* space, `alpha`/`removes` the chain factors
/// materialization multiplies and gates on, `index` the position among the
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
    /// The engine-run transform motion over `transform` — a wrapper's
    /// offset, rotation or scale — evaluated at the clock's instant.
    motion: RefCell<Option<Rc<LayerMotion>>>,
    /// Content offset applied inside the node's own frame (scroll).
    content_offset: Cell<kurbo::Vec2>,
    /// A clip bounding box in the node's own space.
    clip: Cell<Option<kurbo::Rect>>,
    /// The alpha the node's subtree multiplies into its chain.
    alpha: Cell<f32>,
    /// The registration kinds this level removes from materialization for
    /// its whole subtree (`Hittable(false)`, the context-menu preview).
    removes: Cell<HitClasses>,
    /// Position among the parent's ordered items — paint order.
    index: Cell<u32>,
    /// The next item position this placement hands out: children link as
    /// items of it, and a registration anchored at it ranks at the cursor
    /// the items took their indices from, so content registered before,
    /// between or after children orders the same as its emission.
    item_cursor: Cell<u32>,
    /// The overscroll stretch the node's content draws with while it
    /// shows: a scroll node's viewport placement carries it, so a platform
    /// view inside resolves the warp its host applies
    /// ([`resolved_stretches`](Self::resolved_stretches)).
    stretch: Cell<Option<ShownStretch>>,
    /// The window clock every write bumps.
    clock: Rc<PlacementClock>,
    /// The structural chain's resolution at the epoch it was computed —
    /// recomputed lazily per clock epoch.
    resolved_hit: RefCell<Option<(u64, ChainResolution)>>,
}

/// One scroll view's overscroll stretch resolved down a placement chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedStretch {
    /// The stretch filter's parameters with the window's logical units as
    /// its device space ([`ShownStretch::params`]).
    pub params: [f32; 10],
    /// The stage's interpolation strength.
    pub strength: f32,
    /// The stretching viewport in window space.
    pub viewport: kurbo::Rect,
}

/// A chain composed down to one placement.
#[derive(Clone, Copy, Debug)]
struct ChainResolution {
    transform: kurbo::Affine,
    clip: Option<kurbo::Rect>,
    alpha: f32,
    removes: HitClasses,
}

impl Placement {
    /// A placement on `clock`, unattached, identity transform, no clip,
    /// full alpha, removes nothing, index 0.
    pub fn new(clock: &Rc<PlacementClock>) -> Rc<Self> {
        Rc::new(Self {
            parent: RefCell::new(None),
            paint_parent: RefCell::new(None),
            transform: Cell::new(kurbo::Affine::IDENTITY),
            motion: RefCell::new(None),
            content_offset: Cell::new(kurbo::Vec2::ZERO),
            clip: Cell::new(None),
            alpha: Cell::new(1.0),
            removes: Cell::new(HitClasses::NONE),
            index: Cell::new(0),
            item_cursor: Cell::new(0),
            stretch: Cell::new(None),
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
        self.resolution(hit).transform
    }

    /// The `(transform, clip, alpha)` the chain resolves to — `hit`
    /// selects the structural chain, otherwise the paint chain — the raw
    /// form behind [`resolve_hit`](Self::resolve_hit) for consumers that
    /// need the clip itself (embedded targets' hit clip) or the transform
    /// (inverse projection).
    pub fn resolved_chain(&self, hit: bool) -> (kurbo::Affine, Option<kurbo::Rect>, f32) {
        let resolved = self.resolution(hit);
        (resolved.transform, resolved.clip, resolved.alpha)
    }

    /// Whether a registration of kind `class` anchored here materializes:
    /// no level of the structural chain removes the kind, and an
    /// [`HitClasses::OPACITY_GATED`] kind additionally needs the chain's
    /// alpha above the hit threshold — dev's two gates, kept distinct.
    pub fn admits(&self, class: HitClasses) -> bool {
        let resolved = self.resolution(true);
        !resolved.removes.intersects(class)
            && (!HitClasses::OPACITY_GATED.intersects(class)
                || resolved.alpha > PLACEMENT_HIT_ALPHA_THRESHOLD)
    }

    /// Whether a level of the structural chain removes `class` outright —
    /// the truncation dev's `Hittable(false)` and preview ran, as opposed
    /// to the alpha gate.
    pub fn removes(&self, class: HitClasses) -> bool {
        self.resolution(true).removes.intersects(class)
    }

    /// The placement's own transform into its parent's space.
    pub const fn transform(&self) -> kurbo::Affine {
        self.transform.get()
    }

    pub fn set_transform(&self, transform: kurbo::Affine) {
        self.transform.set(transform);
        self.clock.bump();
    }

    /// Moves the node's frame by `motion` over its transform: the wrapper
    /// that owns the placement animates its layer on the engine's clock.
    pub fn set_motion(&self, motion: Rc<LayerMotion>) {
        *self.motion.borrow_mut() = Some(motion);
        self.clock.bump();
    }

    /// The node's transform into its parent's space with its motion at
    /// `at` (at rest with `None`) — the layer's transform as the engine
    /// draws it then.
    fn local_transform(&self, at: Option<Instant>) -> kurbo::Affine {
        let transform = self.transform.get();
        self.motion
            .borrow()
            .as_ref()
            .map_or(transform, |motion| motion.matrix(transform, at))
    }

    /// The motions moving this placement's paint chain, itself included:
    /// the wrappers whose tracks carry content drawn here.
    pub fn paint_chain_motions(&self) -> Vec<Rc<LayerMotion>> {
        self.ancestors(false)
            .iter()
            .map(Rc::as_ref)
            .chain(std::iter::once(self))
            .filter_map(|placement| placement.motion.borrow().clone())
            .collect()
    }

    /// The paint chain's `(transform, clip, alpha)` with every motion on it
    /// evaluated at `at`, uncached — what the engine draws for a frame
    /// presented then, for the one consumer placed per frame: a platform
    /// view under a running motion.
    pub fn resolved_paint_chain_at(
        &self,
        at: Instant,
    ) -> (kurbo::Affine, Option<kurbo::Rect>, f32) {
        let resolved = self.resolve_chain(false, Some(at));
        (resolved.transform, resolved.clip, resolved.alpha)
    }

    /// Sets the clip bounding box in the node's own space.
    pub fn set_content_offset(&self, offset: kurbo::Vec2) {
        self.content_offset.set(offset);
        self.clock.bump();
    }

    /// Redirects the paint chain through `parent` (`None` follows the
    /// structural parent again).
    pub fn set_paint_parent(&self, parent: Option<Rc<Self>>) {
        *self.paint_parent.borrow_mut() = parent;
        self.clock.bump();
    }

    /// The paint-space transform from this placement's frame to
    /// `anchor`'s content space: this level's transform, then every
    /// intermediate level's transform and content offset, up to (not
    /// including) `anchor`. A layer frame under `anchor` takes it as its
    /// layer transform.
    ///
    /// # Panics
    /// Panics when `anchor` is not on this placement's paint chain.
    pub fn paint_delta_to(&self, anchor: &Rc<Self>) -> kurbo::Affine {
        let mut transform = self.transform.get();
        let mut parent = self.parent_for(false);
        while let Some(placement) = parent {
            if Rc::ptr_eq(&placement, anchor) {
                return transform;
            }
            // A placement with motion is a layer node's frame: the frames
            // under it anchor on it, so no paint delta crosses it — the
            // engine moves its layer, never a descendant's transform.
            if placement.motion.borrow().is_some() {
                waterui_core::contract_violation!(
                    "hydrolysis placement: a paint delta crosses an engine-animated frame; \
                     the layer is placed at the frame's rest transform"
                );
            }
            transform = placement.transform.get()
                * kurbo::Affine::translate(placement.content_offset.get())
                * transform;
            parent = placement.parent_for(false);
        }
        panic!("hydrolysis placement: a layer's anchor is not on its paint chain")
    }

    pub fn set_clip(&self, clip: Option<kurbo::Rect>) {
        self.clip.set(clip);
        self.clock.bump();
    }

    /// Sets the subtree alpha factor.
    pub fn set_alpha(&self, alpha: f32) {
        self.alpha.set(alpha);
        self.clock.bump();
    }

    /// Sets the registration kinds this subtree removes from
    /// materialization.
    pub fn set_removes(&self, removes: HitClasses) {
        self.removes.set(removes);
        self.clock.bump();
    }

    /// Sets the position among the parent's ordered items.
    pub fn set_index(&self, index: u32) {
        self.index.set(index);
        self.clock.bump();
    }

    /// Resets the props the node writes about itself — content offset,
    /// clip, alpha, removed kinds — to defaults, keeping `transform`,
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
        self.removes.set(HitClasses::NONE);
        self.item_cursor.set(0);
        self.stretch.set(None);
        self.clock.bump();
    }

    /// Sets the overscroll stretch the node's content draws with, `None`
    /// at rest. Only a change bumps the clock: a scroll view re-states its
    /// stretch at every flush.
    pub(super) fn set_stretch(&self, stretch: Option<ShownStretch>) {
        if self.stretch.replace(stretch) != stretch {
            self.clock.bump();
        }
    }

    /// Every overscroll stretch on the paint chain down to this placement,
    /// outermost first, as a platform view at this placement is warped by
    /// them: the window's logical units are the device space the
    /// parameters carry. Motions on the chain evaluate at `at`; `None`
    /// evaluates them at the clock's instant.
    pub fn resolved_stretches(&self, at: Option<Instant>) -> Vec<ResolvedStretch> {
        let at = at.or_else(|| self.clock.instant());
        let ancestors = self.ancestors(false);
        let mut transform = kurbo::Affine::IDENTITY;
        let mut stretches = Vec::new();
        for placement in ancestors
            .iter()
            .rev()
            .map(Rc::as_ref)
            .chain(std::iter::once(self))
        {
            transform *= placement.local_transform(at);
            if let Some(shown) = placement.stretch.get() {
                stretches.push(ResolvedStretch {
                    params: shown.params(transform),
                    strength: waterui_graphics::backend::filtrate::filters::STRETCH_INTERPOLATION_STRENGTH,
                    viewport: crate::renderer::transformed_rect(transform, shown.viewport),
                });
            }
            transform *= kurbo::Affine::translate(placement.content_offset.get());
        }
        stretches
    }

    /// Resolves a rect from this placement's local space to window space
    /// through the structural chain for an [`HitClasses::INPUT`]
    /// registration: `None` when the chain does not
    /// [`admit`](Self::admits) input.
    pub fn resolve_hit(&self, local: kurbo::Rect) -> Option<ResolvedPlacement> {
        if !self.admits(HitClasses::INPUT) {
            return None;
        }
        let resolved = self.resolution(true);
        let mut rect = crate::renderer::transformed_rect(resolved.transform, local);
        if let Some(clip) = resolved.clip {
            rect = rect.intersect(clip);
        }
        Some(ResolvedPlacement { rect })
    }

    /// The chain's resolution; the structural chain's is cached against
    /// the clock epoch, so any placement write anywhere in the window
    /// invalidates it.
    fn resolution(&self, hit: bool) -> ChainResolution {
        let at = self.clock.instant();
        if !hit {
            return self.resolve_chain(false, at);
        }
        let epoch = self.clock.epoch();
        if let Some((cached, resolved)) = *self.resolved_hit.borrow()
            && cached == epoch
        {
            return resolved;
        }
        let resolved = self.resolve_chain(true, at);
        *self.resolved_hit.borrow_mut() = Some((epoch, resolved));
        resolved
    }

    /// Composes transform, clip, alpha and removed kinds down the chain —
    /// `hit` walks the structural parents, otherwise the paint parents —
    /// with every motion on the chain at `at`.
    ///
    /// The walk collects `Rc` clones bottom-up then composes top-down, so
    /// no level can disappear mid-resolve.
    fn resolve_chain(&self, hit: bool, at: Option<Instant>) -> ChainResolution {
        let ancestors = self.ancestors(hit);
        let mut transform = kurbo::Affine::IDENTITY;
        let mut clip: Option<kurbo::Rect> = None;
        let mut alpha = 1.0f32;
        let mut removes = HitClasses::NONE;
        for placement in ancestors
            .iter()
            .rev()
            .map(Rc::as_ref)
            .chain(std::iter::once(self))
        {
            removes = removes.union(placement.removes.get());
            transform *= placement.local_transform(at);
            alpha *= placement.alpha.get();
            if let Some(rect) = placement.clip.get() {
                // The clip lives in the placement's own frame, outside its
                // content offset: compose the chain up to this level, then
                // intersect in window space.
                let window_clip = crate::renderer::transformed_rect(transform, rect);
                clip = Some(clip.map_or(window_clip, |c| c.intersect(window_clip)));
            }
            transform *= kurbo::Affine::translate(placement.content_offset.get());
        }
        ChainResolution {
            transform,
            clip,
            alpha,
            removes,
        }
    }

    /// The placements above this one, nearest first — `hit` walks the
    /// structural parents, otherwise the paint parents.
    fn ancestors(&self, hit: bool) -> Vec<Rc<Self>> {
        let mut ancestors: Vec<Rc<Self>> = Vec::new();
        let mut parent = self.parent_for(hit);
        while let Some(placement) = parent {
            parent = placement.parent_for(hit);
            ancestors.push(placement);
        }
        ancestors
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
