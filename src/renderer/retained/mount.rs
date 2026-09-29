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
//! - **Keyed mounts** are identity-bearing: a GPU surface's mount lives
//!   under its [`RenderKey`], created on first appearance and dropped —
//!   detached at the next commit — the first frame it is absent. Clip and
//!   opacity scopes a surface is drawn under become a persistent chain of
//!   wrapper layers above its content mount, each carrying one clip and
//!   one alpha. This is the mount "reused across frames, never rebuilt
//!   per frame".
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
    /// The content layer: the frame's produced texture or drawing attaches
    /// here, innermost under the wrapper chain.
    content: cherenkov::Layer,
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
    /// Identity-bearing mounts, keyed by the visual node's [`RenderKey`].
    keyed: FxHashMap<RenderKey, KeyedMount>,
    /// The overlay layer, created on first transient scene and kept.
    overlay: Option<cherenkov::Layer>,
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
            keyed: FxHashMap::default(),
            overlay: None,
            order_ids: Vec::new(),
            frame_created: 0,
            frame_removed: 0,
        }
    }

    /// The layer create/remove counts since the last call, consumed by the
    /// frame's migration counters.
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
        if !order.contains(&MountSlot::Overlay) {
            self.frame_removed += u64::from(self.overlay.is_some());
            self.overlay = None;
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

    /// Statistics the migration counters report per frame.
    pub(crate) fn layer_count(&self) -> usize {
        self.segments.len()
            + self
                .keyed
                .values()
                .map(|mount| mount.wrappers.len() + mount.parked.len() + 1)
                .sum::<usize>()
            + usize::from(self.overlay.is_some())
    }
}
