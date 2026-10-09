//! Shared material backdrop groups for a [`Mount`](super::Mount).
//!
//! `.material_group()` (water-rs/waterui#1999) gives the members flushing
//! under it one shared filtered capture. The group table is keyed by
//! `(scope, within-window level, colour scheme, install canvas)`: the
//! scope is the group wrapper node's identity — or the member's own frame
//! layer outside every group — and the canvas is the nearest enclosing
//! filtered frame, since members share a capture only inside one
//! compositing canvas. A member is a node's frame layer; its membership
//! lives in the node's `NodeLayers` and ends when the node's layers drop
//! or the node's commit clears it.

use std::collections::hash_map::Entry;
use std::num::NonZeroU64;
use std::rc::{Rc, Weak};

use cherenkov::{Layer, LayerId, Transaction};
use rustc_hash::{FxHashMap, FxHashSet};

use cherenkov_record::{CaptureClass, MaterialEffect, MaterialGrouping, SharedLive};

use crate::renderer::material::{MaterialRuntime, WithinWindowLevel};
use crate::renderer::mount::{MaterialRequest, NodeCell};

/// What a shared backdrop group is scoped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackdropScope {
    /// A member outside every `.material_group()`: it is a group of its
    /// own, keyed by its own frame layer — two ungrouped members never
    /// share.
    Solo(LayerId),
    /// The `.material_group()` wrapper node's identity — the address of
    /// its mount cell. The scope's anchor layer entry holds an [`Rc`]
    /// to the cell for as long as the registration exists, so the
    /// address cannot be freed and reused while it keys a group. Two
    /// modifier instances are two groups.
    Scoped(NonZeroU64),
}

/// Which backdrop group a material member joins: the tuple `(scope,
/// within-window level, colour scheme, install canvas)`. Materials share
/// a capture only inside one compositing canvas and one appearance, so
/// the canvas and the resolved colour scheme are part of the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BackdropGroupKey {
    /// The nearest enclosing `.material_group()` node's identity the
    /// members share, or the member's own frame layer when it wraps in no
    /// group.
    pub(crate) scope: BackdropScope,
    /// The members' within-window level: one group runs one level's
    /// chain, so members of different levels never share a capture.
    pub(crate) level: WithinWindowLevel,
    /// The members' colour scheme, resolved at flush: a subtree may
    /// install its own scheme, so members at one level in one scope can
    /// differ — the group runs one constant scheme's chain.
    pub(crate) scheme: waterui::theme::ColorScheme,
    /// The install canvas the members' content mounts under: `None`
    /// under the surface root, `Some(frame)` under a filtered node's
    /// frame layer.
    pub(crate) canvas: Option<LayerId>,
    /// The layer the scope's members capture beneath, resolved from the
    /// mount's [`ScopeAnchors`] when the key is built: `None` for a solo
    /// group; a scoped group's `(scope, canvas)` registration always
    /// stands when its members commit. It is part of the key so an anchor
    /// change re-keys the member into a new group rather than silently
    /// sampling the stale anchor.
    pub(crate) anchor: Option<LayerId>,
}

impl BackdropGroupKey {
    /// Keys the group `member`'s frame layer joins for `request` under
    /// `canvas` (the nearest enclosing filtered frame's layer, `None` at
    /// the surface root): the scope is the enclosing `.material_group()`
    /// node's identity, or the member's own frame layer outside every
    /// group. A scoped group anchors at the layer its `(scope, canvas)`
    /// pair registered in `anchors` (water-rs/waterui#2097).
    pub(crate) fn new(
        member: LayerId,
        request: &MaterialRequest,
        canvas: Option<LayerId>,
        anchors: &ScopeAnchors,
    ) -> Self {
        let (scope, anchor) = request
            .scope
            .map_or((BackdropScope::Solo(member), None), |scope| {
                (
                    BackdropScope::Scoped(scope),
                    Some(anchors.anchor(scope, canvas)),
                )
            });
        Self {
            scope,
            level: request.level,
            scheme: request.scheme,
            canvas,
            anchor,
        }
    }

    /// The group's treatment terms: the level's blur and colour stage
    /// under the key's scheme, resolved once per join.
    pub(crate) fn runtime(&self) -> MaterialRuntime {
        MaterialRuntime::new(self.level, self.scheme)
    }
}

/// Which chrome group a material member joins: the tuple `(scope,
/// capture class, install canvas)`. Members of one class inside one
/// scope and one compositing canvas share one group — one capture, one
/// chain; `Solo` members and members in different scopes, classes or
/// canvases never share (water-rs/waterui#1788).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChromeGroupKey {
    /// The material scope the members share, or the member's own layer
    /// when it is a group of its own.
    scope: BackdropScope,
    /// The members' capture class: one group runs one class's chain.
    class: CaptureClass,
    /// The install canvas the members' content mounts under: `None`
    /// under the surface root, `Some(frame)` under a filtered node's
    /// frame layer.
    canvas: Option<LayerId>,
    /// The layer the scope's members capture beneath, resolved from the
    /// mount's [`ScopeAnchors`] when the key is built: `None` for a group
    /// of its own, which keeps the first-member rule. Part of the key for
    /// the same reason as [`BackdropGroupKey`]'s anchor.
    anchor: Option<LayerId>,
}

impl ChromeGroupKey {
    /// Keys the group the `member` layer joins under `canvas`: a `Solo`
    /// class or a `SOLO` scope keys the member by its own layer, so it
    /// never shares; a `Shared` or `Union` class under a scope shares it
    /// and anchors at the layer the scope's `(scope, canvas)` pair
    /// registered in `anchors`, like a material group
    /// (water-rs/waterui#2097).
    pub(crate) fn new(
        member: LayerId,
        scope: cherenkov_record::MaterialScope,
        class: CaptureClass,
        grouping: MaterialGrouping,
        canvas: Option<LayerId>,
        anchors: &ScopeAnchors,
    ) -> Self {
        let (scope, anchor) = match scope.id() {
            Some(scope) if grouping != MaterialGrouping::Solo => (
                BackdropScope::Scoped(scope),
                Some(anchors.anchor(scope, canvas)),
            ),
            _ => (BackdropScope::Solo(member), None),
        };
        Self {
            scope,
            class,
            canvas,
            anchor,
        }
    }

    /// The members' capture class.
    pub(crate) const fn class(&self) -> CaptureClass {
        self.class
    }

    /// The layer the group's capture anchors at — `None` under the
    /// first-member rule.
    pub(crate) const fn anchor(&self) -> Option<LayerId> {
        self.anchor
    }
}

/// The membership a member's `NodeLayers` holds: a marker the groups
/// table keeps weakly, so a member whose layers drop — an unmounted
/// subtree — ends its membership at the next sweep without anyone
/// reaching the table — and the member's cell, the link a group rebuild
/// follows to re-point the member's frame layer at the replacement group
/// without waiting for the member's own commit.
pub struct MaterialMembership {
    marker: Rc<MemberMarker>,
    owner: Weak<NodeCell>,
}

impl std::fmt::Debug for MaterialMembership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterialMembership").finish_non_exhaustive()
    }
}

impl MaterialMembership {
    /// A membership for `owner`'s node — `Weak::new()` for the window
    /// layer's own membership, which the mount owns directly.
    pub(crate) fn new(owner: Weak<NodeCell>) -> Self {
        Self {
            marker: Rc::new(MemberMarker),
            owner,
        }
    }
}

/// The liveness token behind a member's `Weak` entry.
struct MemberMarker;

/// One member's table entry: its group key, the weak side of the
/// membership its `NodeLayers` holds, and the cell owning those layers —
/// the link a group rebuild follows to re-point the member's frame layer
/// at the replacement group without waiting for the member's own commit.
struct MemberEntry<K, M> {
    key: K,
    /// The member's own binding payload — its shader handle and live
    /// effect — kept so a group rebuild rebinds each member's own
    /// sample, never the joining member's.
    payload: M,
    /// Set when a group rebuild could not reach the member's live layer
    /// to re-bind it (its cell was mid-commit): the member's next join
    /// must re-bind its own payload instead of reporting `Unchanged`
    /// over a bind that still samples the released group.
    stale: bool,
    marker: Weak<MemberMarker>,
    owner: Weak<NodeCell>,
}

/// One live backdrop group in the table: the target's group object, the
/// parameters its chain was built from, the display scale the chain was
/// built for, and the member layers sampling it. The parameters are the
/// key's terms resolved once — as plain values — so a display-scale
/// rebuild re-runs the same treatment.
struct MountedBackdrop<P, G> {
    /// Held for its lifetime: dropping it unregisters the group.
    group: G,
    /// The terms the group's chain runs: kept so a display-scale
    /// rebuild re-runs the same treatment and the test accessors answer
    /// what the group runs.
    params: P,
    /// `f64::to_bits` of the display scale: a scale change rebuilds the
    /// group, since its chain's parameters are in capture texels.
    display_scale: u64,
    /// The member layers sampling the group: joins and clears edit
    /// it during the commit, and the sweep drops the members whose layers
    /// unmounted. An empty set releases the group at the same commit.
    members: FxHashSet<LayerId>,
}

/// When [`BackdropGroups::join`] binds the joining member's sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberBind {
    /// Always — a lowering, whose member carries a freshly recorded
    /// payload: the payload is built, stored and bound on every join.
    Always,
    /// Only when the membership changed — a commit with no new program:
    /// a new or re-keyed membership, a rebuilt group, or a stale bind.
    /// An unchanged member keeps its bind and its stored payload, and the
    /// payload is never built.
    OnChange,
}

/// The `.material_group()` scope anchors a [`Mount`](super::Mount)
/// registered (water-rs/waterui#2097): the layer id each `(scope,
/// install canvas)` pair's backdrop groups — material and chrome alike —
/// capture beneath, so no member of a scope sees another — one plain,
/// empty layer per `Item::Anchor`, the item the `.material_group()` flush
/// pushes into the enclosing program, and the item a filtered node pushes
/// at its program's start for the innermost enclosing scope, so members
/// mounting inside the filter anchor there. The weak side of the scope's
/// cell rides along: while the entry stands the anchor item pins the
/// `Rc`, so the address cannot be reused and the weak stays live; a scope
/// whose tree dropped releases here at the next sweep. A group of its
/// own — a member outside every scope, or a chrome member of a `Solo`
/// class — keeps the first-member rule.
pub struct ScopeAnchors {
    anchors: FxHashMap<(NonZeroU64, Option<LayerId>), AnchorRegistration>,
    epoch: u64,
}

type AnchorRegistration = (LayerId, Weak<NodeCell>, u64);

/// The identity a `.material_group()` scope keys its groups by: the
/// address of its mount cell. The scope's anchor item holds an [`Rc`] to
/// the cell for as long as its registration stands, so the address
/// cannot be reused while it keys a group.
pub fn scope_id(cell: &NodeCell) -> NonZeroU64 {
    NonZeroU64::new(std::ptr::from_ref(cell).addr() as u64)
        .expect("hydrolysis materials: a live node cell's address is never zero")
}

impl ScopeAnchors {
    pub(crate) fn new() -> Self {
        Self {
            anchors: FxHashMap::default(),
            epoch: 0,
        }
    }

    /// Registers `layer` — the plain, empty layer `cell`'s anchor item
    /// mounts — as the anchor of that scope's groups in `canvas`. While
    /// the entry stands, the anchor item holds the scope's cell, so the
    /// address it keys can never name a different cell. One owner per
    /// key: a second live registration under the same `(scope, canvas)`
    /// names the same layer or it is a bug within one commit; an older
    /// registration is replaced outright.
    pub(crate) fn set(&mut self, cell: &Rc<NodeCell>, canvas: Option<LayerId>, layer: LayerId) {
        let scope = scope_id(cell);
        match self.anchors.entry((scope, canvas)) {
            Entry::Occupied(mut entry) => {
                if entry.get().2 != self.epoch || entry.get().1.upgrade().is_none() {
                    entry.insert((layer, Rc::downgrade(cell), self.epoch));
                } else {
                    assert_eq!(
                        entry.get().0,
                        layer,
                        "hydrolysis mounts: a live anchor registration under \
                         ({scope:#x}, {canvas:?}) names {layer:?} — a second \
                         owner registered {other:?}",
                        other = entry.get().0,
                    );
                }
            }
            Entry::Vacant(entry) => {
                entry.insert((layer, Rc::downgrade(cell), self.epoch));
            }
        }
    }

    /// Drops the registration `(scope, canvas, layer)` installed — a
    /// compare-remove: a stale entry from a scope's previous owner or
    /// lowering removes only the registration it made, never another
    /// list's.
    pub(crate) fn remove(&mut self, scope: NonZeroU64, canvas: Option<LayerId>, layer: LayerId) {
        if self.anchors.get(&(scope, canvas)).map(|entry| entry.0) == Some(layer) {
            self.anchors.remove(&(scope, canvas));
        }
    }

    /// The layer a scoped group — material or chrome — under `(scope,
    /// canvas)` captures beneath.
    ///
    /// # Panics
    /// Panics when no registration stands: the scope's anchor item lowers
    /// before its members, so a scoped member always finds one.
    fn anchor(&self, scope: NonZeroU64, canvas: Option<LayerId>) -> LayerId {
        self.anchors
            .get(&(scope, canvas))
            .expect(
                "hydrolysis mounts: a scoped member's (scope, canvas) \
                 registration stands before the member commits — the \
                 scope's anchor item lowers before its members",
            )
            .0
    }

    /// Releases the registrations whose scope cell dropped — a scope whose
    /// tree unmounted — and opens the next commit's registration epoch.
    /// Runs inside the mount's commit, beside the group tables' sweeps.
    pub(crate) fn sweep(&mut self) {
        self.anchors
            .retain(|_, (_, cell, _)| cell.upgrade().is_some());
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("hydrolysis mounts: backdrop registration epoch overflow");
    }

    /// How many `(scope, canvas)` anchor registrations stand — a
    /// test-facing answer; it must equal the live anchor items' count.
    #[cfg(test)]
    pub(crate) fn registration_count(&self) -> usize {
        self.anchors.len()
    }

    /// The live `(scope, canvas) → layer` registrations — a
    /// test-facing answer.
    #[cfg(test)]
    pub(crate) fn registrations(&self) -> Vec<(NonZeroU64, Option<LayerId>, LayerId)> {
        self.anchors
            .iter()
            .map(|(&(s, c), &(l, _, _))| (s, c, l))
            .collect()
    }
}

/// The spec a group object was created with — the GPU target's
/// [`cherenkov::BackdropGroup`] carries it. A test-facing read.
#[cfg(test)]
pub trait SpecCarrier {
    /// The spec the group was created with.
    fn spec(&self) -> cherenkov::BackdropSpec;
}

#[cfg(test)]
impl SpecCarrier for cherenkov::BackdropGroup {
    fn spec(&self) -> cherenkov::BackdropSpec {
        self.spec()
    }
}

/// The member side of a [`BackdropGroups::join`]: the joining layer, its
/// node's membership hold, and the resolver a group rebuild re-points
/// the other members' live layers with.
pub struct MemberJoin<'a, R, F> {
    /// The member's frame layer.
    pub layer: &'a Layer,
    /// Its node's hold on the group: the rebuild sweeps through it.
    pub membership: &'a MaterialMembership,
    /// Builds the member's own binding payload, stored on its entry so
    /// a group rebuild rebinds each member's own terms. Called only when
    /// the join binds the member.
    pub payload: F,
    /// Finds another member's live layer inside its node's layers.
    pub resolve: R,
}

/// The mount's live backdrop groups and member entries, generic over
/// the group key `K` and its resolved creation parameters `P` — the
/// material groups (`BackdropGroupKey` → `MaterialRuntime`) and the
/// chrome groups (`ChromeGroupKey` → `MaterialCapture`) share one
/// mechanism.
pub struct BackdropGroups<K, P, G, M> {
    groups: FxHashMap<K, MountedBackdrop<P, G>>,
    members: FxHashMap<LayerId, MemberEntry<K, M>>,
}
impl<K: std::fmt::Debug, P: std::fmt::Debug, G, M> std::fmt::Debug for BackdropGroups<K, P, G, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackdropGroups")
            .field("groups", &self.groups.keys())
            .field("members", &self.members.keys())
            .finish()
    }
}

/// The material groups a [`Mount`] keeps — one table per mount.
pub type MaterialBackdropGroups<G> = BackdropGroups<BackdropGroupKey, MaterialRuntime, G, ()>;

/// A chrome member's own binding terms (water-rs/waterui#1788): the
/// shader's engine handle and the member's live effect. Stored on the
/// member's table entry so a group rebuild — or a re-join on the same
/// key — binds the member's own, never the joining member's.
#[derive(Debug)]
pub struct ChromeMemberPayload<S> {
    /// The shader's engine handle, resolved at attach.
    pub shader: S,
    /// The member's live effect: the sample's uniforms re-read as it
    /// changes.
    pub effect: SharedLive<MaterialEffect>,
}

/// The chrome groups a [`Mount`] keeps (water-rs/waterui#1788).
pub type ChromeBackdropGroups<G, S> =
    BackdropGroups<ChromeGroupKey, cherenkov_record::MaterialCapture, G, ChromeMemberPayload<S>>;

impl<K: Copy + Eq + std::hash::Hash, P: Copy, G, M> BackdropGroups<K, P, G, M> {
    pub(crate) fn new() -> Self {
        Self {
            groups: FxHashMap::default(),
            members: FxHashMap::default(),
        }
    }

    /// Makes `member`'s frame layer a member of the group `key` names and
    /// binds its sample: re-keying on any term change, and rebuilding the
    /// group on a display-scale change — its chain's parameters are in
    /// capture texels — re-pointing every member at the rebuilt group
    /// before the old one drops. `membership` is the member's hold the
    /// rebuild resolves the other members' live `NodeLayers` through, so
    /// no member samples a released group even if its own commit does not
    /// run in this transaction.
    ///
    /// `member` packs the member side of the join: its layer, its node's
    /// hold, the builder of its payload, and `resolve`, which finds
    /// another member's live layer inside its node's layers when a rebuilt
    /// group re-points it — the member is the frame layer for material
    /// members, a `ChromeMaterial` member layer for chrome.
    ///
    /// `hooks` packs the target's two callbacks: `create` builds the
    /// target's group object from the parameters the key resolves to and
    /// the display scale; `apply` installs a member layer on a group from
    /// the member's own payload, handed the display scale the group was
    /// built for — a member's device-pixel terms convert at that scale.
    /// A scale rebuild runs it for every *other* member with the payload
    /// its entry holds; `bind` decides whether the joining member's own
    /// payload is built, stored and applied.
    #[expect(
        clippy::too_many_arguments,
        reason = "one join hands the member, the group terms and the target's hooks at once"
    )]
    pub(crate) fn join<T: cherenkov::Target>(
        &mut self,
        tx: &mut Transaction<'_, T>,
        key: K,
        params: P,
        display_scale: f64,
        member: MemberJoin<
            '_,
            impl for<'a> Fn(&'a super::layers::NodeLayers, LayerId) -> Option<&'a Layer>,
            impl FnOnce() -> M,
        >,
        hooks: (
            impl Fn(&P, f64) -> G,
            impl Fn(&mut Transaction<'_, T>, &Layer, &M, &G, f64),
        ),
        bind: MemberBind,
    ) {
        let (create, apply) = hooks;
        let MemberJoin {
            layer: member,
            membership,
            payload,
            resolve,
        } = member;
        let bits = display_scale.to_bits();
        if let Some(entry) = self
            .members
            .get_mut(&member.id())
            .filter(|entry| entry.key == key && !entry.stale)
            && let Some(group) = self
                .groups
                .get(&key)
                .filter(|group| group.display_scale == bits)
        {
            // Membership unchanged: a lowering stores and binds its fresh
            // payload; a commit with no new program keeps the bind and
            // the stored payload, and builds nothing.
            if bind == MemberBind::Always {
                entry.payload = payload();
                apply(tx, member, &entry.payload, &group.group, display_scale);
            }
            return;
        }
        self.leave(member.id());
        let group = match self.groups.entry(key) {
            Entry::Occupied(mut entry) => {
                if entry.get().display_scale != bits {
                    // A display-scale change rebuilds the group: its chain's
                    // parameters are in capture texels. Every member keeps
                    // its entry and is re-pointed at the new group here —
                    // never left to the member's own commit, which a
                    // partial commit may not run — so no member samples a
                    // released group.
                    let params = entry.get().params;
                    let rebuilt = MountedBackdrop {
                        group: create(&params, display_scale),
                        params,
                        display_scale: bits,
                        members: std::mem::take(&mut entry.get_mut().members),
                    };
                    for other in &rebuilt.members {
                        let Some(retained) = self
                            .members
                            .get(other)
                            .and_then(|entry| entry.owner.upgrade())
                            .and_then(|cell| cell.try_retained())
                        else {
                            // An unmounted member's marker is dead; the
                            // sweep drops its entry.
                            continue;
                        };
                        // Each member rebinds its own stored payload,
                        // never the joiner's. A member whose cell is
                        // mid-commit — on this commit's ancestor stack —
                        // holds its layers outside the cell, so its live
                        // layer is out of reach here.
                        let own = self
                            .members
                            .get_mut(other)
                            .expect("a member's entry outlives its membership");
                        if let Some(layers) = &*retained.layers.borrow()
                            && let Some(member) = resolve(layers, *other)
                        {
                            apply(tx, member, &own.payload, &rebuilt.group, display_scale);
                        } else {
                            // Its next join re-binds its own payload rather
                            // than keeping a bind that still samples the
                            // released group.
                            own.stale = true;
                        }
                    }
                    *entry.get_mut() = rebuilt;
                }
                entry.into_mut()
            }
            // First visible member of a frame: the group's chain runs on
            // the key's own runtime — the level and the resolved colour
            // scheme, both plain values. Members under one key share both
            // terms, and an appearance flip re-keys them into a new group.
            Entry::Vacant(entry) => entry.insert(MountedBackdrop {
                group: create(&params, display_scale),
                params,
                display_scale: bits,
                members: FxHashSet::default(),
            }),
        };
        group.members.insert(member.id());
        // A new, re-keyed, rebuilt or stale membership binds under
        // either `bind` — the member's layer samples nothing of this group
        // yet, so even a commit with no new program builds its payload.
        let entry = self
            .members
            .entry(member.id())
            .insert_entry(MemberEntry {
                key,
                payload: payload(),
                stale: false,
                marker: Rc::downgrade(&membership.marker),
                owner: membership.owner.clone(),
            })
            .into_mut();
        apply(tx, member, &entry.payload, &group.group, display_scale);
    }

    /// Takes `member` out of its group's membership. The member's layer
    /// is untouched — the join or clear that follows sets it.
    fn leave(&mut self, member: LayerId) {
        if let Some(entry) = self.members.remove(&member) {
            // Membership always names a live group: the sweep drops a
            // member's table entry before it releases the emptied group.
            self.groups
                .get_mut(&entry.key)
                .expect("hydrolysis mounts: a member's backdrop key must name a live group")
                .members
                .remove(&member);
        }
    }

    /// Releases `member`'s membership in its backdrop group, if it holds
    /// one. The layer's backdrop is the target's to clear —
    /// [`LayerTarget::clear_material`](super::target::LayerTarget::clear_material)
    /// runs beside this call.
    pub(crate) fn clear(&mut self, member: LayerId) {
        self.leave(member);
    }

    /// Ends the memberships whose marker died since the last commit — a
    /// member's marker lives in its `NodeLayers`, so a member whose
    /// layers unmounted leaves here — and releases the groups left empty.
    /// Runs inside the mount's commit, before the transaction applies:
    /// the engine applies a frame's commits before it renders, so the
    /// release and the mounts that replace it land together.
    pub(crate) fn sweep(&mut self) {
        self.members
            .retain(|_, entry| entry.marker.upgrade().is_some());
        self.groups.retain(|_, group| {
            group
                .members
                .retain(|member| self.members.contains_key(member));
            !group.members.is_empty()
        });
    }

    /// The display scale `member`'s backdrop group was built for, `None`
    /// while the member holds no membership.
    #[cfg(test)]
    pub(crate) fn backdrop_display_scale(&self, member: LayerId) -> Option<f64> {
        let key = self.members.get(&member)?.key;
        self.groups
            .get(&key)
            .map(|group| f64::from_bits(group.display_scale))
    }

    /// The anchor layer written into `member`'s group's spec — `None`
    /// for a solo group or while the member holds no membership. A
    /// test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_anchor(&self, member: LayerId) -> Option<LayerId>
    where
        G: SpecCarrier,
    {
        let key = self.members.get(&member)?.key;
        self.groups.get(&key)?.group.spec().anchor_layer()
    }

    /// How many live backdrop groups the table holds — a test-facing
    /// answer.
    #[cfg(test)]
    pub(crate) fn backdrop_group_count(&self) -> usize {
        self.groups.len()
    }

    /// Every member layer and the display scale its group was built for —
    /// the mirror's `backdrops()` answer.
    #[cfg(test)]
    pub(crate) fn member_scales(&self) -> Vec<(LayerId, f64)> {
        self.members
            .iter()
            .filter_map(|(&member, entry)| {
                self.groups
                    .get(&entry.key)
                    .map(|group| (member, f64::from_bits(group.display_scale)))
            })
            .collect()
    }
}

impl BackdropGroups<BackdropGroupKey, MaterialRuntime, cherenkov::BackdropGroup, ()> {
    /// The id of the backdrop group `member` samples — `None` while the
    /// member holds no membership. Two members answering the same id
    /// share one capture. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_group_id(&self, member: LayerId) -> Option<cherenkov::BackdropId> {
        let key = self.members.get(&member)?.key;
        self.groups.get(&key).map(|group| group.group.id())
    }
}

/// Chrome-group test accessors over the members the key maps.
impl<G>
    BackdropGroups<
        ChromeGroupKey,
        cherenkov_record::MaterialCapture,
        G,
        ChromeMemberPayload<cherenkov::BackdropShader>,
    >
{
    /// The scope `member`'s chrome group is keyed by — `None` while the
    /// member holds no membership.
    #[cfg(test)]
    pub(crate) fn chrome_scope(&self, member: LayerId) -> Option<BackdropScope> {
        self.members.get(&member).map(|entry| entry.key.scope)
    }

    /// The display scale `member`'s chrome group was built for — `None`
    /// while the member holds no membership.
    #[cfg(test)]
    pub(crate) fn chrome_display_scale(&self, member: LayerId) -> Option<f64> {
        let key = self.members.get(&member)?.key;
        self.groups
            .get(&key)
            .map(|group| f64::from_bits(group.display_scale))
    }

    /// How many live chrome groups the table holds.
    #[cfg(test)]
    pub(crate) fn chrome_group_count(&self) -> usize {
        self.groups.len()
    }

    /// The group object `member` samples, for identity checks — `None`
    /// while the member holds no membership.
    #[cfg(test)]
    pub(crate) fn chrome_group(&self, member: LayerId) -> Option<&G> {
        let key = self.members.get(&member)?.key;
        self.groups.get(&key).map(|group| &group.group)
    }
}

impl<G> BackdropGroups<BackdropGroupKey, MaterialRuntime, G, ()> {
    /// The scope `member`'s backdrop group is keyed by — `None` while the
    /// member holds no membership. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_scope(&self, member: LayerId) -> Option<BackdropScope> {
        self.members.get(&member).map(|entry| entry.key.scope)
    }

    /// The colour scheme `member`'s backdrop group is keyed by — `None`
    /// while the member holds no membership. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_scheme(&self, member: LayerId) -> Option<waterui::theme::ColorScheme> {
        self.members.get(&member).map(|entry| entry.key.scheme)
    }

    /// The colour-stage parameters `member`'s backdrop group runs —
    /// `None` while the member holds no membership. The answer is the
    /// group's own runtime: the key's level and colour scheme resolved
    /// once, so two groups under different schemes run different tones. A
    /// test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_tone(&self, member: LayerId) -> Option<[f32; 7]> {
        let key = self.members.get(&member)?.key;
        self.groups
            .get(&key)
            .map(|group| group.params.tone_params())
    }

    /// The chain `member`'s backdrop group runs, rebuilt from the runtime
    /// the group holds at the group's own display scale — `None` while
    /// the member holds no membership. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_chain(
        &self,
        member: LayerId,
    ) -> Option<crate::renderer::material::MaterialChain> {
        let key = self.members.get(&member)?.key;
        self.groups
            .get(&key)
            .map(|group| group.params.chain(f64::from_bits(group.display_scale)))
    }
}
