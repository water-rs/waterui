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
//! Wrapper layers are never destroyed while their mount lives: dropping a
//! [`cherenkov::Layer`] removes its whole subtree at the next commit, so a
//! shrinking ancestry detaches its excess wrappers and parks them for
//! reuse instead. A shrunken chain's handles stay alive under the mount.
//!
//! One persistent overlay layer sits above every other child for the
//! frame's transient scene (popups, menus and capture/transition content).

use super::identity::RenderKey;
use rustc_hash::{FxHashMap, FxHashSet};
use waterui_graphics::HeldResources;

/// One slot in the surface root's desired child order for a presented frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MountSlot {
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
pub(crate) struct AncestryScope {
    /// The scope's clip shape in root coordinates, when it clips.
    pub(crate) clip: Option<cherenkov::ShapeData>,
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
    /// Detached wrappers kept alive for reuse. Destroying a layer removes
    /// its subtree at the next commit, so a shrinking ancestry parks its
    /// excess instead.
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

impl KeyedMount {
    /// The layer the parent orders under — the outermost wrapper when the
    /// ancestry is non-empty.
    fn ordered(&self) -> &cherenkov::Layer {
        self.wrappers.first().unwrap_or(&self.content)
    }
}

/// The persistent engine layers a window presents through.
///
/// Handles are `Layer`s: dropping one removes it and its descendants at the
/// next commit, so pruning an absent keyed mount is a map removal and
/// nothing else.
pub(crate) struct Mounts {
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
            overlay: None,
            overlay_held: None,
            order_ids: Vec::new(),
            frame_created: 0,
            frame_removed: 0,
        }
    }

    /// The layer create/remove counts since the last call, consumed by the
    /// frame's work counters.
    pub(crate) fn take_frame_stats(&mut self) -> (u64, u64) {
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
                // it: destroying the handles would take the content layer's
                // subtree with them at the next commit.
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
