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

use crate::renderer::material::{MaterialRuntime, WithinWindowLevel};
use crate::renderer::mount::MaterialRequest;

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
}

/// The membership a member's `NodeLayers` holds: a marker the groups
/// table keeps weakly, so a member whose layers drop — an unmounted
/// subtree — ends its membership at the next sweep without anyone
/// reaching the table.
pub struct MaterialMembership {
    _marker: Rc<MemberMarker>,
}

/// The liveness token behind a member's `Weak` entry.
struct MemberMarker;

/// One member's table entry: its group key and the weak side of the
/// membership its `NodeLayers` holds.
struct MemberEntry {
    key: BackdropGroupKey,
    marker: Weak<MemberMarker>,
}

/// One live backdrop group in the table: the target's group object, the
/// runtime its chain was built from, the display scale the chain was
/// built for, and the member frame layers sampling it. The runtime is the
/// key's level and colour scheme resolved once — as plain values — and an
/// appearance flip re-keys the member's install instead, so the scheme
/// change reaches the filter through a new group.
struct MountedBackdrop<G> {
    /// Held for its lifetime: dropping it unregisters the group.
    group: G,
    /// The runtime the group's chain runs: kept so a display-scale
    /// rebuild re-runs the same treatment and the test accessors answer
    /// what the group runs.
    runtime: MaterialRuntime,
    /// `f64::to_bits` of the display scale: a scale change rebuilds the
    /// group, since its chain's parameters are in capture texels.
    display_scale: u64,
    /// The member frame layers sampling the group: joins and clears edit
    /// it during the commit, and the sweep drops the members whose layers
    /// unmounted. An empty set releases the group at the same commit.
    members: FxHashSet<LayerId>,
}

/// The mount's live backdrop groups and member entries.
pub struct BackdropGroups<G> {
    groups: FxHashMap<BackdropGroupKey, MountedBackdrop<G>>,
    members: FxHashMap<LayerId, MemberEntry>,
}

impl<G> BackdropGroups<G> {
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
    /// old one drops.
    ///
    /// `hooks` packs the target's two callbacks: `create` builds the
    /// target's group object from the runtime the key resolves to and the
    /// display scale; `apply` installs a member layer on a group. On the
    /// GPU target they create a [`cherenkov::BackdropGroup`] and sample
    /// it onto the layer.
    pub(crate) fn join<T: cherenkov::Target>(
        &mut self,
        tx: &mut Transaction<'_, T>,
        member: &Layer,
        key: BackdropGroupKey,
        display_scale: f64,
        membership: &mut Option<MaterialMembership>,
        hooks: (
            impl Fn(&MaterialRuntime, f64) -> G,
            impl Fn(&mut Transaction<'_, T>, &Layer, &G),
        ),
    ) {
        let (create, apply) = hooks;
        let bits = display_scale.to_bits();
        if membership.is_some()
            && self
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
                    // parameters are in capture texels. The other members'
                    // entries drop here — each member's own join re-points
                    // it at the new group as the same commit walks on, so
                    // no member samples a released group.
                    let runtime = entry.get().runtime;
                    *entry.get_mut() = MountedBackdrop {
                        group: create(&runtime, display_scale),
                        runtime,
                        display_scale: bits,
                        members: FxHashSet::default(),
                    };
                    self.members.retain(|_, member| member.key != key);
                }
                entry.into_mut()
            }
            // First visible member of a frame: the group's chain runs on
            // the key's own runtime — the level and the resolved colour
            // scheme, both plain values. Members under one key share both
            // terms, and an appearance flip re-keys them into a new group.
            Entry::Vacant(entry) => {
                let runtime = MaterialRuntime::new(key.level, key.scheme);
                entry.insert(MountedBackdrop {
                    group: create(&runtime, display_scale),
                    runtime,
                    display_scale: bits,
                    members: FxHashSet::default(),
                })
            }
        };
        apply(tx, member, &group.group);
        group.members.insert(member.id());
        let marker = Rc::new(MemberMarker);
        self.members.insert(
            member.id(),
            MemberEntry {
                key,
                marker: Rc::downgrade(&marker),
            },
        );
        *membership = Some(MaterialMembership { _marker: marker });
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

    /// The scope `member`'s backdrop group is keyed by — `None` while the
    /// member holds no membership. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_scope(&self, member: LayerId) -> Option<BackdropScope> {
        self.members.get(&member).map(|entry| entry.key.scope)
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
            .map(|group| group.runtime.tone_params())
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
            .map(|group| group.runtime.chain(f64::from_bits(group.display_scale)))
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

impl BackdropGroups<cherenkov::BackdropGroup> {
    /// The id of the backdrop group `member` samples — `None` while the
    /// member holds no membership. Two members answering the same id
    /// share one capture. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_group_id(&self, member: LayerId) -> Option<cherenkov::BackdropId> {
        let key = self.members.get(&member)?.key;
        self.groups.get(&key).map(|group| group.group.id())
    }
}
