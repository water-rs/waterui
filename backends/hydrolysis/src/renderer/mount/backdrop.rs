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
use std::rc::{Rc, Weak};

use cherenkov::{Layer, LayerId, Transaction};
use rustc_hash::{FxHashMap, FxHashSet};

use std::num::NonZeroU64;

use cherenkov_record::{CaptureClass, MaterialGrouping};

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
    /// its cell, which the scope stack holds an `Rc` to while its
    /// children flush, so the pointer cannot be reused mid-frame. Two
    /// modifier instances are two groups.
    Scoped(usize),
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
    scope: BackdropScope,
    /// The members' within-window level: one group runs one level's
    /// chain, so members of different levels never share a capture.
    level: WithinWindowLevel,
    /// The members' colour scheme, resolved at flush: a subtree may
    /// install its own scheme, so members at one level in one scope can
    /// differ — the group runs one constant scheme's chain.
    scheme: waterui::theme::ColorScheme,
    /// The install canvas the members' content mounts under: `None`
    /// under the surface root, `Some(frame)` under a filtered node's
    /// frame layer.
    canvas: Option<LayerId>,
}

impl BackdropGroupKey {
    /// Keys the group `member`'s frame layer joins for `request` under
    /// `canvas` (the nearest enclosing filtered frame's layer, `None` at
    /// the surface root): the scope is the enclosing `.material_group()`
    /// node's identity, or the member's own frame layer outside every
    /// group.
    pub(crate) fn new(member: LayerId, request: &MaterialRequest, canvas: Option<LayerId>) -> Self {
        Self {
            scope: request
                .scope
                .map_or_else(|| BackdropScope::Solo(member), BackdropScope::Scoped),
            level: request.level,
            scheme: request.scheme,
            canvas,
        }
    }

    /// The group's treatment terms: the level's blur and colour stage
    /// under the key's scheme, resolved once per join.
    pub(crate) fn runtime(&self) -> MaterialRuntime {
        MaterialRuntime::new(self.level, self.scheme)
    }
}

/// What a chrome-material member's shared group is scoped to
/// (water-rs/waterui#1788): the `MaterialScope` the recording was opened
/// under, or the member's own layer when it is a group of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChromeScope {
    /// A member outside every material scope, or of a `Solo` class:
    /// a group of its own, keyed by its own layer.
    Solo(LayerId),
    /// The enclosing `.material_group()` scope's identity the members
    /// share.
    Scoped(NonZeroU64),
}

/// Which chrome group a material member joins: the tuple `(scope,
/// capture class, install canvas)`. Members of one class inside one
/// scope and one compositing canvas share one group — one capture, one
/// chain; `Solo` members and members in different scopes, classes or
/// canvases never share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChromeGroupKey {
    /// The material scope the members share, or the member's own layer
    /// when it is a group of its own.
    scope: ChromeScope,
    /// The members' capture class: one group runs one class's chain.
    class: CaptureClass,
    /// The install canvas the members' content mounts under: `None`
    /// under the surface root, `Some(frame)` under a filtered node's
    /// frame layer.
    canvas: Option<LayerId>,
}

impl ChromeGroupKey {
    /// Keys the group the `member` layer joins under `canvas`: a `Solo`
    /// class or a `SOLO` scope keys the member by its own layer, so it
    /// never shares; a `Shared` or `Union` class under a scope shares
    /// it.
    pub(crate) fn new(
        member: LayerId,
        scope: cherenkov_record::MaterialScope,
        class: CaptureClass,
        grouping: MaterialGrouping,
        canvas: Option<LayerId>,
    ) -> Self {
        Self {
            scope: match scope.id() {
                Some(scope) if grouping != MaterialGrouping::Solo => ChromeScope::Scoped(scope),
                _ => ChromeScope::Solo(member),
            },
            class,
            canvas,
        }
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
struct MemberEntry<K> {
    key: K,
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

/// The member side of a [`BackdropGroups::join`]: the joining layer, its
/// node's membership hold, and the resolver a group rebuild re-points
/// the other members' live layers with.
pub struct MemberJoin<'a, R> {
    /// The member's frame layer.
    pub layer: &'a Layer,
    /// Its node's hold on the group: the rebuild sweeps through it.
    pub membership: &'a MaterialMembership,
    /// Finds another member's live layer inside its node's layers.
    pub resolve: R,
}

/// The mount's live backdrop groups and member entries, generic over
/// the group key `K` and its resolved creation parameters `P` — the
/// material groups (`BackdropGroupKey` → `MaterialRuntime`) and the
/// chrome groups (`ChromeGroupKey` → `MaterialCapture`) share one
/// mechanism.
pub struct BackdropGroups<K, P, G> {
    groups: FxHashMap<K, MountedBackdrop<P, G>>,
    members: FxHashMap<LayerId, MemberEntry<K>>,
}
impl<K: std::fmt::Debug, P: std::fmt::Debug, G> std::fmt::Debug for BackdropGroups<K, P, G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackdropGroups")
            .field("groups", &self.groups.keys())
            .field("members", &self.members.keys())
            .finish()
    }
}

/// The material groups a [`Mount`] keeps — one table per mount.
pub type MaterialBackdropGroups<G> = BackdropGroups<BackdropGroupKey, MaterialRuntime, G>;

/// The chrome groups a [`Mount`] keeps (water-rs/waterui#1788).
pub type ChromeBackdropGroups<G> =
    BackdropGroups<ChromeGroupKey, cherenkov_record::MaterialCapture, G>;

impl<K: Copy + Eq + std::hash::Hash, P: Copy, G> BackdropGroups<K, P, G> {
    pub(crate) fn new() -> Self {
        Self {
            groups: FxHashMap::default(),
            members: FxHashMap::default(),
        }
    }

    /// Makes `member`'s frame layer a member of the group `key` names:
    /// re-keying on any term change, and rebuilding the group on a
    /// display-scale change — its chain's parameters are in capture
    /// texels — re-pointing every member at the rebuilt group before the
    /// old one drops. `membership` is the member's hold the rebuild
    /// resolves the other members' live `NodeLayers` through, so no
    /// member samples a released group even if its own commit does not
    /// run in this transaction.
    ///
    /// `member` packs the member side of the join: its layer, its node's
    /// hold, and `resolve`, which finds another member's live layer
    /// inside its node's layers when a rebuilt group re-points it — the
    /// member is the frame layer for material members, a
    /// `ChromeMaterial` member layer for chrome.
    ///
    /// `hooks` packs the target's two callbacks: `create` builds the
    /// target's group object from the parameters the key resolves to and
    /// the display scale; `apply` installs a member layer on a group,
    /// handed the display scale the group was built for — a member's
    /// device-pixel terms convert at that scale, and a scale rebuild
    /// re-runs it for every member.
    pub(crate) fn join<T: cherenkov::Target>(
        &mut self,
        tx: &mut Transaction<'_, T>,
        key: K,
        params: P,
        display_scale: f64,
        member: MemberJoin<
            '_,
            impl for<'a> Fn(&'a super::layers::NodeLayers, LayerId) -> Option<&'a Layer>,
        >,
        hooks: (
            impl Fn(&P, f64) -> G,
            impl Fn(&mut Transaction<'_, T>, &Layer, &G, f64),
        ),
    ) {
        let (create, apply) = hooks;
        let MemberJoin {
            layer: member,
            membership,
            resolve,
        } = member;
        let bits = display_scale.to_bits();
        if self
            .members
            .get(&member.id())
            .is_some_and(|entry| entry.key == key)
            && self
                .groups
                .get(&key)
                .is_some_and(|group| group.display_scale == bits)
        {
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
                        // A member mid-commit — on this commit's
                        // ancestor stack — holds its layers outside the
                        // cell; only the joining member can be one, and
                        // its `apply` below covers it.
                        if let Some(layers) = &*retained.layers.borrow()
                            && let Some(member) = resolve(layers, *other)
                        {
                            apply(tx, member, &rebuilt.group, display_scale);
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
        apply(tx, member, &group.group, display_scale);
        group.members.insert(member.id());
        self.members.insert(
            member.id(),
            MemberEntry {
                key,
                marker: Rc::downgrade(&membership.marker),
                owner: membership.owner.clone(),
            },
        );
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

impl BackdropGroups<BackdropGroupKey, MaterialRuntime, cherenkov::BackdropGroup> {
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
impl<G> BackdropGroups<ChromeGroupKey, cherenkov_record::MaterialCapture, G> {
    /// The scope `member`'s chrome group is keyed by — `None` while the
    /// member holds no membership.
    #[cfg(test)]
    pub(crate) fn chrome_scope(&self, member: LayerId) -> Option<ChromeScope> {
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

impl<G> BackdropGroups<BackdropGroupKey, MaterialRuntime, G> {
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
