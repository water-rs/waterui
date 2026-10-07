//! Stable engine mounts under a window surface (water-rs/hydrolysis#205, H1).
//!
//! The frame's ordered `RenderLayer`s map onto persistent engine layers
//! mounted under the surface root. Two mount kinds exist:
//!
//! - **Segment mounts** are positional: each contiguous run of recorded
//!   scene ops drained by `flush_scene_layer` shows on the segment
//!   layer at that stack position. Membership shifts every frame, so the
//!   mount carries no identity — it only keeps the engine layer alive
//!   while the content payload is replaced.
//! - **Keyed mounts** are identity-bearing: a GPU content view's or filtered
//!   group's mount lives under its [`RenderKey`], created on first appearance
//!   and dropped — detached at the next commit — the first frame it is
//!   absent. Clip and opacity scopes the view is drawn under become a
//!   persistent chain of wrapper layers above its content mount, each
//!   carrying one clip and one alpha. This is the mount "reused across
//!   frames, never rebuilt per frame".
//!
//! A keyed mount may also own a **group body**: the positional segment
//! layers and committed order of the children a filtered mount draws under
//! its content layer. Group children are mounts like any other — a keyed
//! child simply has its ordered layer pushed under the group's content
//! layer rather than the surface root — so the same `RenderKey` identity
//! works at any depth.
//!
//! Material members hold **backdrop group** memberships in a table beside
//! the mounts rather than on them: a `Material` layer's key is `(scope,
//! within-window level, colour scheme, install canvas)` — the
//! `.material_group()` node's render identity, or the member's own mount
//! when it wraps in no group — so the members of one modifier instance at
//! one level under one scheme on one canvas share one `BackdropGroup`, one
//! capture and one chain, and a member in a filtered view's canvas, under a
//! subtree-installed appearance, or in an anchored overlay (whose flush
//! starts with an empty scope stack) never joins them. A group builds on
//! its first visible member of a frame, rebuilds on a display-scale change
//! by re-pointing every member before the old group drops, and releases at
//! the commit of the first frame with no visible member.
//!
//! Wrapper layers are never destroyed while their mount lives: a
//! shrinking ancestry detaches its excess wrappers and parks them for
//! reuse instead of dropping them. A shrunken chain's handles stay alive
//! under the mount.
//!
//! One persistent overlay layer sits above every other child for the
//! frame's transient scene (popups, menus and capture/transition content).

use std::collections::hash_map::Entry;

use super::identity::{RenderId, RenderKey};
use rustc_hash::{FxHashMap, FxHashSet};
use waterui_graphics::HeldResources;

/// One slot in the surface root's desired child order for a presented frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountSlot {
    /// Positional scene-segment mount.
    Segment(usize),
    /// Identity-bearing mount for one [`RenderKey`].
    Keyed(RenderKey),
    /// The single overlay mount above every other child.
    Overlay,
}

/// One scope of a keyed mount's clip/opacity ancestry.
///
/// `clip` is the scope's shape already mapped into root-surface
/// coordinates; `opacity` is the scope's alpha.
pub struct AncestryScope {
    /// The scope's clip shape in root coordinates, when it clips.
    pub(crate) clip: Option<waterui_graphics::draw::ShapeData>,
    /// The scope's alpha.
    pub(crate) opacity: f32,
}

/// The child mounts under a filtered mount's content layer: positional
/// segment layers plus the order committed last frame. Group children are
/// ordinary mounts — only their parent link differs — so this carries no
/// keyed state of its own.
#[derive(Default)]
struct GroupBody {
    /// Positional segment layers, grown to the group's segment count and
    /// truncated when it falls.
    segments: Vec<cherenkov::Layer>,
    /// The registrations each segment's installed content names, kept until
    /// the segment's content is replaced or the layer pruned — parallel to
    /// `segments`.
    held: Vec<Option<HeldResources>>,
    /// The resolved child order committed under the content layer last
    /// frame — engine layer ids, so an ancestry change still counts.
    order_ids: Vec<cherenkov::LayerId>,
}

/// The engine layers one [`RenderKey`] owns: an ancestry chain of clip and
/// opacity wrappers (outermost first) above the content mount.
struct KeyedMount {
    /// Attached ancestry wrappers, outermost first — `wrappers.len()` is
    /// the committed scope count.
    wrappers: Vec<cherenkov::Layer>,
    /// Detached wrappers kept alive for reuse: a shrinking ancestry parks
    /// its excess wrappers instead of destroying layers it may need again.
    parked: Vec<cherenkov::Layer>,
    /// The content layer: the frame's produced texture, drawing or filter
    /// attaches here, innermost under the wrapper chain. A filtered mount's
    /// children parent under this layer, so the filter covers them all.
    content: cherenkov::Layer,
    /// The registrations the recording installed on `content` names — kept
    /// until the recording that replaces it is installed, or the mount drops.
    held: Option<HeldResources>,
    /// The group's segment layers and committed order, allocated when the
    /// first filtered child mounts under `content`.
    group: Option<GroupBody>,
}

/// Which backdrop group a material member joins: the tuple `(scope,
/// within-window level, colour scheme, install canvas)`. Materials share a
/// capture only inside one compositing canvas and one appearance, so the
/// canvas and the resolved colour scheme are part of the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BackdropGroupKey {
    /// The nearest enclosing `.material_group()` node's render identity the
    /// members share, or the member's own mount when it wraps in no group.
    scope: BackdropScope,
    /// The members' within-window level: one group runs one level's chain,
    /// so members of different levels never share a capture.
    level: crate::renderer::material::WithinWindowLevel,
    /// The members' colour scheme, resolved at flush: a subtree may
    /// install its own scheme, so members at one level in one scope can
    /// differ — the group runs one constant scheme's chain.
    scheme: waterui::theme::ColorScheme,
    /// The install canvas the members' content layers mount on: `None`
    /// under the surface root, `Some(parent)` under a filtered group's
    /// mount.
    canvas: Option<RenderKey>,
}

/// The member-side terms of a [`BackdropGroupKey`]: the scope, the
/// resolved colour scheme and the install canvas a material member mounts
/// under. The remaining term — the within-window level — is the member's
/// `MaterialLayer` level, a separate [`set_backdrop`][Self::set_backdrop]
/// argument.
#[derive(Clone, Copy, Debug)]
pub struct MemberScope {
    /// The nearest enclosing `.material_group()` node's render identity
    /// the member flushed under, or `None` outside every group.
    pub scope: Option<RenderId>,
    /// The member's resolved colour scheme at flush: a subtree may install
    /// its own scheme, so members at one level in one scope can differ —
    /// each scheme keys its own group.
    pub scheme: waterui::theme::ColorScheme,
    /// The install canvas the member's content layer mounts on: `None`
    /// under the surface root, `Some(parent)` under a filtered group's
    /// mount. Materials share a capture only inside one compositing
    /// canvas, so the canvas is part of the group key.
    pub canvas: Option<RenderKey>,
}

/// What a shared backdrop group is scoped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BackdropScope {
    /// A member outside every `.material_group()`: it is a group of its
    /// own, keyed by its own mount — two ungrouped members never share.
    Solo(RenderKey),
    /// The `.material_group()` wrapper node's render identity: two
    /// modifier instances are two groups.
    Scoped(RenderId),
}

/// One live backdrop group in the mount table: the group itself, the
/// runtime its chain was built from, the display scale the chain was
/// built for, and the member mounts sampling it. The runtime is the key's
/// level and colour scheme resolved once — as plain values — and an
/// appearance flip re-keys the member's install instead, so the scheme
/// change reaches the filter through a new group.
struct MountedBackdrop {
    /// Held for its lifetime: dropping it unregisters the group.
    group: cherenkov::BackdropGroup,
    /// The runtime the group's chain runs: kept so a display-scale rebuild
    /// re-runs the same treatment and the test accessors answer what the
    /// group runs.
    runtime: crate::renderer::material::MaterialRuntime,
    /// `f64::to_bits` of the display scale: a scale change rebuilds the
    /// group, since its chain's parameters are in capture texels.
    display_scale: u64,
    /// The member mounts sampling the group: joins and visibility clears
    /// edit it during install, and the order sync drops the members whose
    /// mounts left the frame. An empty set releases the group at the
    /// sync's commit.
    members: FxHashSet<RenderKey>,
}

impl MountedBackdrop {
    /// Builds the group running `runtime`'s chain at `display_scale` —
    /// `f64::to_bits` of the display scale — for `members`.
    fn new(
        surface: &cherenkov::Surface<cherenkov_gpu::Gpu>,
        runtime: crate::renderer::material::MaterialRuntime,
        display_scale: u64,
        members: FxHashSet<RenderKey>,
    ) -> Self {
        Self {
            group: surface.backdrop_group(
                runtime.chain(f64::from_bits(display_scale)),
                crate::renderer::material::capture_scale(),
            ),
            runtime,
            display_scale,
            members,
        }
    }
}

impl KeyedMount {
    /// The layer the parent orders under — the outermost wrapper when the
    /// ancestry is non-empty.
    fn ordered(&self) -> &cherenkov::Layer {
        self.wrappers.first().unwrap_or(&self.content)
    }
}

/// The persistent engine layers a window presents through.
///
/// Handles are `Layer`s: dropping one removes only it — its children stay
/// in the tree, detached — at the next commit, so pruning an absent keyed
/// mount is a map removal and nothing else.
pub struct Mounts {
    /// Positional segment layers, grown to the frame's segment count and
    /// shrunk — truncated — when it falls. Segment layers never carry
    /// children, so truncation destroys no mount state.
    segments: Vec<cherenkov::Layer>,
    /// The registrations each segment's installed content names, parallel to
    /// `segments` — released when the segment's content is replaced or the
    /// layer truncated.
    segment_held: Vec<Option<HeldResources>>,
    /// Identity-bearing mounts, keyed by the visual node's [`RenderKey`].
    keyed: FxHashMap<RenderKey, KeyedMount>,
    /// Every live backdrop group on this surface, solo and scoped alike.
    /// A group builds on its first visible member of a frame and releases
    /// at the commit of the first frame with no visible member — the same
    /// commit the members' detach or clear rides, so no frame samples the
    /// released group.
    backdrop_groups: FxHashMap<BackdropGroupKey, MountedBackdrop>,
    /// Which backdrop group each member mount currently samples.
    backdrop_members: FxHashMap<RenderKey, BackdropGroupKey>,
    /// The overlay layer, created on first transient scene and kept.
    overlay: Option<cherenkov::Layer>,
    /// The registrations the overlay's installed content names.
    overlay_held: Option<HeldResources>,
    /// The resolved child order committed last frame — engine layer ids,
    /// not slots, so an ancestry change that swaps a mount's ordered layer
    /// still counts as an order change.
    order_ids: Vec<cherenkov::LayerId>,
    /// Layers created since the last [`Self::take_frame_stats`].
    frame_created: u64,
    /// Layers dropped since the last [`Self::take_frame_stats`].
    frame_removed: u64,
}

impl core::fmt::Debug for Mounts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mounts")
            .field("segments", &self.segments.len())
            .field("keyed", &self.keyed.len())
            .field("overlay", &self.overlay.is_some())
            .finish_non_exhaustive()
    }
}

impl Mounts {
    /// An empty mount set: no engine layers exist until the first frame
    /// asks for them.
    pub(crate) fn new() -> Self {
        Self {
            segments: Vec::new(),
            segment_held: Vec::new(),
            keyed: FxHashMap::default(),
            backdrop_groups: FxHashMap::default(),
            backdrop_members: FxHashMap::default(),
            overlay: None,
            overlay_held: None,
            order_ids: Vec::new(),
            frame_created: 0,
            frame_removed: 0,
        }
    }

    /// The layer create/remove counts since the last call, consumed by the
    /// frame's work counters.
    pub(crate) const fn take_frame_stats(&mut self) -> (u64, u64) {
        let stats = (self.frame_created, self.frame_removed);
        self.frame_created = 0;
        self.frame_removed = 0;
        stats
    }

    /// The content-mount engine layer `slot` resolves to, creating its
    /// mount on first use. For a keyed mount this is the innermost layer —
    /// the one frame content and its placement transform attach to; the
    /// ordered parent-side child is [`Self::ordered`].
    pub(crate) fn layer(
        &mut self,
        surface: &cherenkov::Surface<cherenkov_gpu::Gpu>,
        slot: MountSlot,
    ) -> &cherenkov::Layer {
        match slot {
            MountSlot::Segment(index) => {
                while self.segments.len() <= index {
                    self.segments.push(surface.layer());
                    self.frame_created += 1;
                }
                &self.segments[index]
            }
            MountSlot::Keyed(key) => {
                &self
                    .keyed
                    .entry(key)
                    .or_insert_with(|| {
                        self.frame_created += 1;
                        KeyedMount {
                            wrappers: Vec::new(),
                            parked: Vec::new(),
                            content: surface.layer(),
                            held: None,
                            group: None,
                        }
                    })
                    .content
            }
            MountSlot::Overlay => self.overlay.get_or_insert_with(|| {
                self.frame_created += 1;
                surface.layer()
            }),
        }
    }

    /// The layer the parent orders for `slot`: a keyed mount's outermost
    /// wrapper, or its content layer when the ancestry is empty.
    fn ordered(&self, slot: MountSlot) -> &cherenkov::Layer {
        match slot {
            MountSlot::Segment(index) => &self.segments[index],
            MountSlot::Keyed(key) => self
                .keyed
                .get(&key)
                .expect("hydrolysis mounts: ordered child for an uncreated mount")
                .ordered(),
            MountSlot::Overlay => self
                .overlay
                .as_ref()
                .expect("hydrolysis mounts: ordered overlay before creation"),
        }
    }

    /// Sets a keyed mount's clip/opacity ancestry: `scopes` wrapper layers
    /// outermost-first, each carrying one clip and one alpha.
    ///
    /// Parent links only re-emit when the scope count changes; equal counts
    /// refresh the per-scope edits in place.
    pub(crate) fn set_ancestry(
        &mut self,
        surface: &cherenkov::Surface<cherenkov_gpu::Gpu>,
        tx: &mut cherenkov::Transaction<'_, cherenkov_gpu::Gpu>,
        key: RenderKey,
        scopes: &[AncestryScope],
    ) {
        let mount = self
            .keyed
            .get_mut(&key)
            .expect("hydrolysis mounts: ancestry for an uncreated mount");
        let count_changed = mount.wrappers.len() != scopes.len();

        if count_changed {
            while mount.wrappers.len() < scopes.len() {
                let wrapper = mount.parked.pop().unwrap_or_else(|| {
                    self.frame_created += 1;
                    surface.layer()
                });
                mount.wrappers.push(wrapper);
            }
            if mount.wrappers.len() > scopes.len() {
                // Detach the top excess wrapper, then park the chain below
                // it: the parked handles keep their layers alive and
                // reusable for the next growth.
                let parent = mount
                    .wrappers
                    .get(scopes.len().wrapping_sub(1))
                    .unwrap_or_else(|| surface.root());
                tx[parent].remove(&mount.wrappers[scopes.len()]);
                mount.parked.extend(mount.wrappers.drain(scopes.len()..));
            }
            // Chain links: each wrapper onto its parent, the content layer
            // onto the innermost wrapper. The outermost wrapper's parent
            // link is the order sync's.
            for index in 1..mount.wrappers.len() {
                let (before, after) = mount.wrappers.split_at(index);
                tx[&before[index - 1]].push(&after[0]);
            }
            if let Some(innermost) = mount.wrappers.last() {
                tx[innermost].push(&mount.content);
            }
        }

        for (wrapper, scope) in mount.wrappers.iter().zip(scopes) {
            match &scope.clip {
                Some(clip) => {
                    tx[wrapper].clip(clip.clone());
                }
                None => {
                    tx[wrapper].clear_clip();
                }
            }
            tx[wrapper].opacity(scope.opacity);
        }
    }

    /// Installs `member`'s membership in the backdrop group its `(scope,
    /// level, colour scheme, install canvas)` key names, building the
    /// group on the key's first visible member of a frame, on the surface
    /// the mount belongs to.
    ///
    /// `member_scope.scope` is the render identity of the
    /// `.material_group()` node nearest enclosing the member, `None` when
    /// it wraps in no group — a scope-less member is a group of its own,
    /// keyed by its own mount. `member_scope.scheme` is the member's
    /// colour scheme, resolved at flush: a subtree may install its own
    /// scheme, so two members at one level in one scope can differ — each
    /// scheme keys its own group. `member_scope.canvas`
    /// is the member's install canvas ([`InstallScope::parent_key`]):
    /// materials share a capture only inside one compositing canvas, so
    /// it is part of the key. `level` is the member's within-window
    /// level, the `Material` wrapper's payload: members of different
    /// levels never share a capture.
    ///
    /// A member still holding membership in the same key takes no work. A
    /// member joining a different key leaves its old group first, and a
    /// group whose membership empties releases at the order sync's commit.
    /// A display-scale change rebuilds the group — its chain's parameters
    /// are in capture texels — and re-points every member's content layer
    /// at the new group before the old one drops. Ordering is safe: the
    /// engine applies a frame's commits before it renders, so no frame
    /// samples the released group. The group's chain is a `MaterialRuntime`
    /// built from the key's level and colour scheme — both plain values;
    /// an appearance flip re-keys the member's install, so the change
    /// reaches the filter through the new group at the commit, snapping
    /// rather than animating.
    ///
    /// [`InstallScope::parent_key`]:
    ///     crate::renderer::render::compositor::InstallScope::parent_key
    pub(crate) fn set_backdrop(
        &mut self,
        surface: &cherenkov::Surface<cherenkov_gpu::Gpu>,
        tx: &mut cherenkov::Transaction<'_, cherenkov_gpu::Gpu>,
        member: RenderKey,
        member_scope: MemberScope,
        display_scale: f64,
        level: crate::renderer::material::WithinWindowLevel,
    ) {
        assert!(
            self.keyed.contains_key(&member),
            "hydrolysis mounts: backdrop for an uncreated mount"
        );
        let key = BackdropGroupKey {
            scope: member_scope
                .scope
                .map_or(BackdropScope::Solo(member), BackdropScope::Scoped),
            level,
            scheme: member_scope.scheme,
            canvas: member_scope.canvas,
        };
        if self.backdrop_members.get(&member) == Some(&key)
            && self
                .backdrop_groups
                .get(&key)
                .is_some_and(|group| group.display_scale == display_scale.to_bits())
        {
            return;
        }
        self.leave_backdrop(member);
        let display_scale = display_scale.to_bits();
        let group = match self.backdrop_groups.entry(key) {
            Entry::Occupied(mut entry) => {
                if entry.get().display_scale != display_scale {
                    // A display-scale change rebuilds the group — its
                    // chain's parameters are in capture texels — and
                    // re-points every member at the new group before the
                    // old one drops. The chain re-runs on the group's own
                    // constant-scheme runtime.
                    let replacement = MountedBackdrop::new(
                        surface,
                        entry.get().runtime,
                        display_scale,
                        core::mem::take(&mut entry.get_mut().members),
                    );
                    for &other in &replacement.members {
                        let mount = self
                            .keyed
                            .get(&other)
                            .expect("hydrolysis mounts: backdrop for an uncreated mount");
                        tx[&mount.content].backdrop(replacement.group.sample());
                    }
                    *entry.get_mut() = replacement;
                }
                entry.into_mut()
            }
            // First visible member of a frame: the group's chain runs on
            // the key's own runtime — the level and the resolved colour
            // scheme, both plain values. Members under one key share both
            // terms, and an appearance flip re-keys them into a new group.
            Entry::Vacant(entry) => entry.insert(MountedBackdrop::new(
                surface,
                crate::renderer::material::MaterialRuntime::new(key.level, key.scheme),
                display_scale,
                FxHashSet::default(),
            )),
        };
        {
            let mount = self
                .keyed
                .get(&member)
                .expect("hydrolysis mounts: backdrop for an uncreated mount");
            tx[&mount.content].backdrop(group.group.sample());
        }
        group.members.insert(member);
        self.backdrop_members.insert(member, key);
    }

    /// Takes `member` out of its backdrop group's membership. The member's
    /// content layer is untouched — the join or clear that follows sets it.
    fn leave_backdrop(&mut self, member: RenderKey) {
        if let Some(key) = self.backdrop_members.remove(&member) {
            // Membership always names a live group: the order sync drops
            // a member's table entry before it releases the emptied group.
            let group = self
                .backdrop_groups
                .get_mut(&key)
                .expect("hydrolysis mounts: a member's backdrop key must name a live group");
            group.members.remove(&member);
        }
    }

    /// Releases `member`'s membership in its backdrop group, if it holds
    /// one, and clears its content layer's membership, so the engine
    /// neither captures nor filters for it until [`Self::set_backdrop`]
    /// builds a new one.
    pub(crate) fn clear_backdrop(
        &mut self,
        tx: &mut cherenkov::Transaction<'_, cherenkov_gpu::Gpu>,
        member: RenderKey,
    ) {
        if !self.backdrop_members.contains_key(&member) {
            return;
        }
        self.leave_backdrop(member);
        let mount = self
            .keyed
            .get(&member)
            .expect("hydrolysis mounts: backdrop for an uncreated mount");
        tx[&mount.content].clear_backdrop();
    }

    /// The display scale `member`'s backdrop group was built for, `None`
    /// while the member holds no membership.
    #[cfg(test)]
    pub(crate) fn backdrop_display_scale(&self, member: RenderKey) -> Option<f64> {
        let key = self.backdrop_members.get(&member)?;
        self.backdrop_groups
            .get(key)
            .map(|group| f64::from_bits(group.display_scale))
    }

    /// The id of the backdrop group `member` samples — `None` while the
    /// member holds no membership. Two members answering the same id share
    /// one capture. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_group_id(&self, member: RenderKey) -> Option<cherenkov::BackdropId> {
        let key = self.backdrop_members.get(&member)?;
        self.backdrop_groups.get(key).map(|group| group.group.id())
    }

    /// How many live backdrop groups the table holds — a test-facing
    /// answer.
    #[cfg(test)]
    pub(crate) fn backdrop_group_count(&self) -> usize {
        self.backdrop_groups.len()
    }

    /// The colour scheme `member`'s backdrop group is keyed by — `None`
    /// while the member holds no membership. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_scheme(&self, member: RenderKey) -> Option<waterui::theme::ColorScheme> {
        self.backdrop_members.get(&member).map(|key| key.scheme)
    }

    /// The colour-stage parameters `member`'s backdrop group runs —
    /// `None` while the member holds no membership. The answer is the
    /// group's own runtime: the key's level and colour scheme resolved
    /// once, so two groups under different schemes run different tones. A
    /// test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_tone(&self, member: RenderKey) -> Option<[f32; 7]> {
        let key = self.backdrop_members.get(&member)?;
        self.backdrop_groups
            .get(key)
            .map(|group| group.runtime.tone_params())
    }

    /// The chain `member`'s backdrop group runs, rebuilt from the runtime
    /// the group holds at the group's own display scale — `None` while the
    /// member holds no membership. A test-facing answer.
    #[cfg(test)]
    pub(crate) fn backdrop_chain(
        &self,
        member: RenderKey,
    ) -> Option<crate::renderer::material::MaterialChain> {
        let key = self.backdrop_members.get(&member)?;
        self.backdrop_groups
            .get(key)
            .map(|group| group.runtime.chain(f64::from_bits(group.display_scale)))
    }

    /// The segment layer `key`'s group orders group child `index` under,
    /// creating the group body and segment layers on first use.
    ///
    /// The layer is the group's positional mount at `index`; its parent
    /// link is committed by [`Self::sync_group_order`].
    pub(crate) fn group_layer(
        &mut self,
        surface: &cherenkov::Surface<cherenkov_gpu::Gpu>,
        key: RenderKey,
        index: usize,
    ) -> &cherenkov::Layer {
        let mount = self
            .keyed
            .get_mut(&key)
            .expect("hydrolysis mounts: group layer for an uncreated mount");
        let group = mount.group.get_or_insert_with(GroupBody::default);
        while group.segments.len() <= index {
            group.segments.push(surface.layer());
            self.frame_created += 1;
        }
        &group.segments[index]
    }

    /// The layer `key`'s group orders for `slot`: a group segment, or a
    /// keyed mount's ordered layer — the same resolution [`Self::ordered`]
    /// applies at the root, with segments drawn from the group body. An
    /// overlay slot is a programmer error inside a group.
    pub(crate) fn ordered_in_group(&self, group: RenderKey, slot: MountSlot) -> &cherenkov::Layer {
        match slot {
            MountSlot::Segment(index) => {
                &self
                    .keyed
                    .get(&group)
                    .expect("hydrolysis mounts: group order for an uncreated mount")
                    .group
                    .as_ref()
                    .expect("hydrolysis mounts: group order sync before first group layer")
                    .segments[index]
            }
            MountSlot::Keyed(key) => self.ordered(MountSlot::Keyed(key)),
            MountSlot::Overlay => {
                panic!("hydrolysis mounts: overlay slot inside a filtered group")
            }
        }
    }

    /// Stores the registrations the recording installed on `slot`'s content
    /// layer names — `parent` is the filtered group's key, `None` at the
    /// surface root. Replaces whatever the slot held before: the previous
    /// set releases once the replacement content is installed, which is the
    /// ordering the caller already guarantees by installing content first.
    pub(crate) fn set_held(
        &mut self,
        parent: Option<RenderKey>,
        slot: MountSlot,
        held: HeldResources,
    ) {
        match (parent, slot) {
            (Some(parent), MountSlot::Segment(index)) => {
                let group = self
                    .keyed
                    .get_mut(&parent)
                    .and_then(|mount| mount.group.as_mut())
                    .expect("hydrolysis mounts: held registrations for an uncreated group");
                while group.held.len() <= index {
                    group.held.push(None);
                }
                group.held[index] = Some(held);
            }
            (Some(_), MountSlot::Overlay) => {
                panic!("hydrolysis mounts: overlay slot inside a filtered group")
            }
            (None, MountSlot::Segment(index)) => {
                while self.segment_held.len() <= index {
                    self.segment_held.push(None);
                }
                self.segment_held[index] = Some(held);
            }
            (None, MountSlot::Overlay) => {
                self.overlay_held = Some(held);
            }
            (_, MountSlot::Keyed(key)) => {
                self.keyed
                    .get_mut(&key)
                    .expect("hydrolysis mounts: held registrations for an uncreated mount")
                    .held = Some(held);
            }
        }
    }

    /// Commits this frame's child order under `key`'s content layer — the
    /// children a filtered mount draws under its filter.
    ///
    /// `order` lists group slots in bottom-to-top order; a keyed slot's
    /// ordered layer is pushed under the content layer just as a root slot
    /// is pushed under the root. Excess group segments truncate; the group
    /// body itself stays allocated for the mount's life.
    pub(crate) fn sync_group_order(
        &mut self,
        tx: &mut cherenkov::Transaction<'_, cherenkov_gpu::Gpu>,
        key: RenderKey,
        order: &[MountSlot],
    ) {
        let order_ids: Vec<cherenkov::LayerId> = order
            .iter()
            .map(|slot| self.ordered_in_group(key, *slot).id())
            .collect();
        {
            let mount = self
                .keyed
                .get_mut(&key)
                .expect("hydrolysis mounts: group order for an uncreated mount");
            // A group whose children are all keyed mounts owns no segment
            // layers — the body materializes on the first order sync.
            let group = mount.group.get_or_insert_with(GroupBody::default);

            let segment_count = order
                .iter()
                .filter_map(|slot| match slot {
                    MountSlot::Segment(index) => Some(*index + 1),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            if group.segments.len() > segment_count {
                self.frame_removed += (group.segments.len() - segment_count) as u64;
            }
            group.segments.truncate(segment_count);
            group.held.truncate(segment_count);

            if order_ids == group.order_ids {
                return;
            }
            group.order_ids = order_ids;
        }

        let parent = &self
            .keyed
            .get(&key)
            .expect("hydrolysis mounts: group parent vanished during sync")
            .content;
        for slot in order {
            let child = self.ordered_in_group(key, *slot);
            tx[parent].push(child);
        }
    }

    /// Commits this frame's child order and prunes mounts that did not
    /// appear in it.
    ///
    /// Order edits run only on change: the comparison is over resolved
    /// layer ids, so a scope-count change that swaps a keyed mount's
    /// ordered child re-emits the ordering even when the slot list is
    /// identical. Push is move semantics, so re-pushing the list in order
    /// both parents new mounts and fixes stacking in one pass.
    ///
    /// `live_keys` is the set of keyed mounts the frame presented; mounts
    /// absent from it are dropped — detached at the next commit — instead
    /// of lingering under a surface they no longer belong to.
    pub(crate) fn sync_order(
        &mut self,
        surface: &cherenkov::Surface<cherenkov_gpu::Gpu>,
        tx: &mut cherenkov::Transaction<'_, cherenkov_gpu::Gpu>,
        order: &[MountSlot],
        live_keys: &FxHashSet<RenderKey>,
    ) {
        let keyed_before = self.keyed.len();
        self.keyed.retain(|key, _| live_keys.contains(key));
        self.frame_removed += (keyed_before - self.keyed.len()) as u64;

        // Backdrop groups release at this commit: the members whose mounts
        // left the frame drop out of their groups' membership, and a group
        // with no visible member is dropped with them — the same ordering
        // argument `set_backdrop` documents applies, since the engine
        // applies a frame's commits before it renders.
        self.backdrop_members
            .retain(|member, _| live_keys.contains(member));
        self.backdrop_groups.retain(|_, group| {
            group.members.retain(|member| live_keys.contains(member));
            !group.members.is_empty()
        });

        let segment_count = order
            .iter()
            .filter_map(|slot| match slot {
                MountSlot::Segment(index) => Some(*index + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        if self.segments.len() > segment_count {
            self.frame_removed += (self.segments.len() - segment_count) as u64;
        }
        self.segments.truncate(segment_count);
        self.segment_held.truncate(segment_count);
        if !order.contains(&MountSlot::Overlay) {
            self.frame_removed += u64::from(self.overlay.is_some());
            self.overlay = None;
            self.overlay_held = None;
        }

        let order_ids: Vec<cherenkov::LayerId> =
            order.iter().map(|slot| self.ordered(*slot).id()).collect();
        if order_ids == self.order_ids {
            return;
        }
        self.order_ids = order_ids;
        let root = surface.root();
        for slot in order {
            let child = self.ordered(*slot);
            tx[root].push(child);
        }
    }
}
