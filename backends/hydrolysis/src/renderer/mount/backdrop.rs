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
use crate::renderer::mount::{MaterialRequest, NodeCell};

/// What a shared backdrop group is scoped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackdropScope {
    /// A member outside every `.material_group()`: it is a group of its
    /// own, keyed by its own frame layer — two ungrouped members never
    /// share.
    Solo(LayerId),
    /// The `.material_group()` wrapper node's identity — the address of
    /// its mount cell — stable for as long as the node stays mounted,
    /// so the pointer cannot be reused mid-frame. Two modifier
    /// instances are two groups.
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
/// reaching the table — and the member's cell, the link a group rebuild
/// follows to re-point the member's frame layer at the replacement group
/// without waiting for the member's own commit.
pub struct MaterialMembership {
    marker: Rc<MemberMarker>,
    owner: Weak<NodeCell>,
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
struct MemberEntry {
    key: BackdropGroupKey,
    marker: Weak<MemberMarker>,
    owner: Weak<NodeCell>,
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
    /// The layer id each `(scope, install canvas)` pair's groups
    /// capture beneath: the `.material_group()` node's own frame for
    /// members sharing its canvas, or the plain leading layer the
    /// canvas's filtered node owns for a scope whose members mount
    /// inside it (water-rs/waterui#2097).
    anchors: FxHashMap<(usize, Option<LayerId>), LayerId>,
    /// Each `.material_group()` scope's own frame, by scope — the entry
    /// `anchors` keeps under the scope's own canvas, kept so
    /// `material_group_anchor` can name the anchor a scope installs.
    scope_frames: FxHashMap<usize, LayerId>,
}

impl<G> BackdropGroups<G> {
    pub(crate) fn new() -> Self {
        Self {
            groups: FxHashMap::default(),
            members: FxHashMap::default(),
            anchors: FxHashMap::default(),
            scope_frames: FxHashMap::default(),
        }
    }

    /// Registers `frame` — the `.material_group()` node's own layer —
    /// as the anchor of `scope`'s groups in the canvas the node mounts
    /// in.
    pub(crate) fn set_scope_anchor(
        &mut self,
        scope: usize,
        canvas: Option<LayerId>,
        frame: LayerId,
    ) {
        self.anchors.insert((scope, canvas), frame);
        self.scope_frames.insert(scope, frame);
    }

    /// Registers `layer` — a plain leading child of `canvas`'s frame —
    /// as the anchor of `scope`'s groups inside the canvas.
    pub(crate) fn set_anchor(&mut self, scope: usize, canvas: LayerId, layer: LayerId) {
        self.anchors.insert((scope, Some(canvas)), layer);
    }

    /// Drops the registration `scope`/`canvas` names: the node owning
    /// its layer retired.
    pub(crate) fn remove_anchor(&mut self, scope: usize, canvas: Option<LayerId>) {
        // The scope's own frame goes out of `scope_frames` with its
        // registration; an in-canvas anchor's removal touches nothing
        // else.
        if self.anchors.remove(&(scope, canvas)) == self.scope_frames.get(&scope).copied() {
            self.scope_frames.remove(&scope);
        }
    }

    /// The layer a group keyed `key` captures beneath — `None` for a
    /// solo group. A scoped group's anchor registers before its first
    /// member commits: the scope's frame for the members sharing its
    /// canvas, the canvas's own anchor layer for the members inside it.
    fn anchor(&self, key: &BackdropGroupKey) -> Option<LayerId> {
        let BackdropScope::Scoped(scope) = key.scope else {
            return None;
        };
        Some(
            self.anchors
                .get(&(scope, key.canvas))
                .copied()
                .unwrap_or_else(|| {
                    panic!(
                        "hydrolysis mounts: a scoped group's anchor layer was not registered: scope={scope:#x} canvas={:?} anchors={:?}",
                        key.canvas,
                        self.anchors.keys().collect::<Vec<_>>()
                    )
                }),
        )
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
        membership: &MaterialMembership,
        hooks: (
            impl Fn(&MaterialRuntime, f64, Option<LayerId>) -> G,
            impl Fn(&mut Transaction<'_, T>, &Layer, &G),
        ),
    ) {
        let (create, apply) = hooks;
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
        // A scoped group's spec anchors at the layer its `(scope,
        // canvas)` pair registered; a solo group names none.
        let anchor = self.anchor(&key);
        let group = match self.groups.entry(key) {
            Entry::Occupied(mut entry) => {
                if entry.get().display_scale != bits {
                    // A display-scale change rebuilds the group: its chain's
                    // parameters are in capture texels. Every member keeps
                    // its entry and is re-pointed at the new group here —
                    // never left to the member's own commit, which a
                    // partial commit may not run — so no member samples a
                    // released group.
                    let runtime = entry.get().runtime;
                    let rebuilt = MountedBackdrop {
                        group: create(&runtime, display_scale, anchor),
                        runtime,
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
                        if let Some(layers) = &*retained.layers.borrow() {
                            apply(tx, layers.frame(), &rebuilt.group);
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
            Entry::Vacant(entry) => {
                let runtime = MaterialRuntime::new(key.level, key.scheme);
                entry.insert(MountedBackdrop {
                    group: create(&runtime, display_scale, anchor),
                    runtime,
                    display_scale: bits,
                    members: FxHashSet::default(),
                })
            }
        };
        apply(tx, member, &group.group);
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

    /// The anchor layer `member`'s group captures beneath — `None` for
    /// a solo group or while the member holds no membership. A
    /// test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_anchor(&self, member: LayerId) -> Option<LayerId> {
        let key = self.members.get(&member)?.key;
        let BackdropScope::Scoped(scope) = key.scope else {
            return None;
        };
        self.anchors.get(&(scope, key.canvas)).copied()
    }

    /// The `.material_group()` scope's own anchor frame — `None` once
    /// the scope unmounted. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn material_group_anchor(&self, scope: usize) -> Option<LayerId> {
        self.scope_frames.get(&scope).copied()
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
