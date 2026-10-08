//! System-compositor planes (#90).
//!
//! On a target that exposes a system-compositor parent, the engine promotes
//! eligible layers to their own system layers: the platform compositor, not
//! the engine, composites their content, so a full-screen video that plays
//! on a hardware overlay costs no per-frame engine composition. Everything
//! else is still composited inside the engine, onto one texture per *part*:
//! a promoted layer splits the surface into the part painted below it and
//! the part painted above it, so content drawn after the layer (controls,
//! overlays, its own children) stays above it.
//!
//! The decision is deterministic per frame and invisible to callers:
//! [`plan`] reads the sampled tree and the platform's [`Compositor`] limits
//! and returns the promoted layers in paint order, with the named cause for
//! every candidate it kept in the engine. Promotion is a realization choice
//! made before rendering, never a fallback after a failure: a platform that
//! rejects a plane the plan chose reports an error naming the cause.
//!
//! Content the engine cannot composite — a hosted system layer — is a
//! *mandatory* candidate ([`Source::mandatory`]): it is judged by the
//! mandatory-plane rule, receives the budget first, and a mandatory
//! candidate that fails is not kept in the engine but listed in
//! [`Plan::unplaced`], which the renderer turns into a render error.
//!
//! A platform implements [`SystemPlanes`] and realizes the ordered stack of
//! engine parts and promoted planes a [`Composition`] describes.

use kurbo::{Affine, Rect, Vec2};
use rustc_hash::{FxHashMap, FxHashSet};

use cherenkov::{BlendMode, Display, LayerId, RenderError, ShapeData, SurfaceError, SurfaceTree};

use crate::interop::ExternalFrame;
use crate::render::lower::axis_aligned;
use crate::render::present::Presenter;

#[cfg(target_vendor = "apple")]
mod animation;

pub mod static_layer;

/// A plane buffer's extent and texel-to-content transform. External frames
/// use identity; recorded layers have a local raster origin and density.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Raster extent.
    pub size: (u32, u32),
    /// Buffer texels to layer content coordinates.
    pub raster: Affine,
    /// What the plane shows, which orders the budget: mandatory content
    /// first, then external frames, then recorded captures.
    pub source: Source,
}

/// What a plane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A system layer the host supplies (`cherenkov::HostedLayers`): the
    /// engine has no pixels of it, so it is shown on a plane or not at all.
    Hosted,
    /// An external frame the system layer shows itself.
    Frame,
    /// An immutable capture of a recorded layer.
    Recorded,
}

impl Source {
    /// The budget order: mandatory content, then frames, then captures.
    const BUDGET_ORDER: [Self; 3] = [Self::Hosted, Self::Frame, Self::Recorded];

    /// Whether the content can only be shown on a plane: it is judged by
    /// the mandatory-plane rule, is never pending a realization, and a
    /// failed verdict is an error ([`Plan::unplaced`]) rather than
    /// composition in the engine.
    #[must_use]
    pub const fn mandatory(self) -> bool {
        matches!(self, Self::Hosted)
    }
}

impl From<(u32, u32)> for Candidate {
    fn from(size: (u32, u32)) -> Self {
        Self {
            size,
            raster: Affine::IDENTITY,
            source: Source::Frame,
        }
    }
}

/// What a platform's system compositor can express, which bounds promotion.
pub trait Compositor {
    /// The most layers promoted on one surface. Every promoted layer adds
    /// an engine part above it, a full-surface texture, so the budget bounds
    /// memory as well as the hardware overlays the system can scan out.
    const BUDGET: usize;

    /// Whether a system layer carries `transform`, a layer's local matrix,
    /// exactly.
    fn expresses_transform(transform: Affine) -> bool;

    /// Whether a system layer clips its sublayers to `clip`, in the layer's
    /// own space, exactly.
    fn expresses_clip(clip: &ShapeData) -> bool;

    /// Whether a hosted system layer — a layer the host draws, which has no
    /// buffer of the engine's — carries `transform`, a local matrix on its
    /// path, exactly.
    fn hosts_transform(transform: Affine) -> bool {
        Self::expresses_transform(transform)
    }

    /// Whether a hosted system layer carries an opacity below one.
    const HOSTS_OPACITY: bool;

    /// Whether a system layer shows `frame` itself, with the colour the
    /// frame declares: its planes are a buffer the system compositor can
    /// scan out. Only such frames are candidates.
    fn shows(frame: &ExternalFrame) -> bool;
}

/// Why a candidate layer stays composited in the engine.
///
/// Each cause names the rule it failed. Opportunistic promotion keeps such
/// a layer in the engine; content that can only be shown on a plane turns
/// the same cause into a render error ([`Plan::unplaced`]).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Ineligible {
    /// An ancestor composites its subtree through an offscreen (opacity
    /// below one, a filter, or a blend), so the layer never reaches the
    /// surface level a plane sits at.
    #[error("ancestor layer {0:?} isolates its subtree into an offscreen")]
    Isolated(LayerId),
    /// The layer carries a filter, which must process its pixels.
    #[error("it carries a filter")]
    Filter,
    /// The layer composites with a non-default blend mode, or a child blends
    /// onto its content.
    #[error("it uses a non-default blend mode")]
    Blend,
    /// A layer painted above it blends with a non-default mode onto the
    /// surface, which would need the layer's pixels.
    #[error("layer {0:?} painted above it blends with a non-default mode")]
    BlendAbove(LayerId),
    /// A layer painted above it, intersecting its bounds, is not known to
    /// be opaque: the system compositor blends the part above a plane in
    /// its own space, which does not reproduce the engine's linear blend,
    /// so the pixels would differ wherever the above layer has coverage.
    #[error("layer {0:?} painted above it is not known to be opaque")]
    TranslucentAbove(LayerId),
    /// The layer samples a backdrop.
    #[error("it samples a backdrop")]
    Backdrop,
    /// A layer painted above it samples a backdrop, whose capture would need
    /// the layer's pixels.
    #[error("layer {0:?} painted above it samples a backdrop")]
    BackdropAbove(LayerId),
    /// Its opacity is below one and it has child layers: the opacity applies
    /// to the group, which a plane and the part above it cannot share.
    #[error("its opacity applies to a group of child layers")]
    GroupOpacity,
    /// Its opacity is below one, and the system compositor cannot fade a
    /// hosted layer.
    #[error("its opacity is below one, which the system compositor cannot apply to a hosted layer")]
    Opacity,
    /// A transform on the path to the layer is not expressible by the
    /// system layer.
    #[error("layer {0:?}'s transform is not expressible by the system compositor")]
    Transform(LayerId),
    /// A clip on the path to the layer is not expressible by the system
    /// layer.
    #[error("layer {0:?}'s clip is not expressible by the system compositor")]
    Clip(LayerId),
    /// A clip on the path to the layer nests inside another clip that is not
    /// a device-aligned rectangle; the engine composites that pair through a
    /// clip offscreen, which the layer would then sit in.
    #[error("layer {0:?}'s clip nests inside another non-rectangular clip")]
    NestedClip(LayerId),
    /// The surface's plane budget is spent.
    #[error("the plane budget of {0} per surface is spent")]
    Budget(usize),
}

/// One level of the path from the root to a promoted layer: the sampled
/// properties of that tree layer, with the tree's semantics. The level's own
/// space is its parent's content space times `transform`; `clip` applies in
/// that space; content and children sit in that space translated by
/// `-scroll`.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    /// The tree layer this level mirrors.
    pub layer: LayerId,
    /// The local transform.
    pub transform: Affine,
    /// The clip, in the level's own space.
    pub clip: Option<ShapeData>,
    /// The snapped scroll offset.
    pub scroll: Vec2,
}

impl Level {
    /// `transform * translate(-scroll)`: the space of the level's content
    /// and children.
    #[must_use]
    pub fn content_transform(&self) -> Affine {
        self.transform * Affine::translate(-self.scroll)
    }
}

/// Where a promoted layer sits.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The promoted layer.
    pub layer: LayerId,
    /// The native buffer realization committed for this content.
    pub source: Source,
    /// The content rectangle `(0, 0, w, h)` in the layer's content space.
    pub size: (u32, u32),
    /// Buffer texels to the layer's content space.
    pub raster: Affine,
    /// The layer's opacity; every ancestor is opaque by eligibility.
    pub opacity: f32,
    /// The root first, the promoted layer last.
    pub path: Vec<Level>,
}

impl Placement {
    /// Content space to device space: the product of every level's content
    /// transform.
    #[must_use]
    #[cfg_attr(
        not(any(test, target_os = "android")),
        expect(
            dead_code,
            reason = "a flattened placement, for realizations without nested layers"
        )
    )]
    pub fn content_to_device(&self) -> Affine {
        self.path.iter().fold(Affine::IDENTITY, |acc, level| {
            acc * level.content_transform()
        }) * self.raster
    }
}

/// A surface's promotion decision for one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// The promoted layers, in paint order.
    pub planes: Vec<Placement>,
    /// The candidates kept in the engine, in paint order, with their cause.
    pub rejected: Vec<(LayerId, Ineligible)>,
    /// The mandatory candidates that cannot be placed on a plane, in paint
    /// order, with their cause. Nothing shows them: a plan with any is a
    /// render error, never presented.
    pub unplaced: Vec<(LayerId, Ineligible)>,
    /// Whether any layer is painted after the last promoted one, so an
    /// engine part exists above it.
    pub trailing: bool,
}

impl Plan {
    /// The promoted layers a new engine part opens after: every plane but
    /// the last, which opens one only when `trailing` content follows it.
    /// Lowering emits a `Part` pass for each of these and no more, and
    /// `parts()` counts this same boundary set.
    pub fn opens_part(&self) -> impl Iterator<Item = LayerId> + '_ {
        let last_needs_none = usize::from(!self.trailing);
        self.planes
            .iter()
            .take(self.planes.len().saturating_sub(last_needs_none))
            .map(|p| p.layer)
    }

    /// The number of engine parts: part 0 plus one opened after each
    /// plane [`opens_part`](Self::opens_part) reports.
    #[must_use]
    pub fn parts(&self) -> usize {
        self.opens_part().count() + 1
    }
}

/// Whether a layer composites its subtree through an offscreen: the rule the
/// lowering applies (`Lowering::layer`). A projective layer's subtree renders
/// into its local image — the same offscreen isolation — and the root renders
/// into the surface target and never isolates for blended children.
fn isolates(tree: &SurfaceTree, id: LayerId) -> bool {
    let node = tree.layer(id);
    node.filter.is_some()
        || node.opacity < 1.0
        || node.blend != BlendMode::Normal
        || tree.projective_pose(id).is_some()
        || (id != tree.root() && node.blends_within())
}

/// One layer in paint order.
struct Visit {
    id: LayerId,
    /// The index in `order` of each ancestor, root first.
    ancestors: Vec<usize>,
}

/// A visit's device-space footprint: the content-space transform
/// (ancestor transforms and scrolls applied), the intersection of every
/// clip on the path mapped to device space — `None` when no level clips,
/// meaning the layer can paint anywhere on the surface — and whether the
/// layer composites at the surface at all (inside an isolating ancestor
/// it lands only in that ancestor's offscreen).
struct VisitDevice {
    space: Affine,
    bounds: Option<Rect>,
}

/// Reusable workspace for a plan walk: the paint order (each visit's
/// ancestor chain keeps the buffer it had last call — `pool` holds the
/// ones `order` gave back), the traversal stack, and the backdrop/blend
/// suffix lookups. A renderer owns one so a frame's plan check allocates
/// nothing steady-state (#90).
#[derive(Default)]
pub struct PlanScratch {
    order: Vec<Visit>,
    pool: Vec<Vec<usize>>,
    stack: Vec<(LayerId, usize)>,
    backdrop_above: Vec<Option<LayerId>>,
    blend_above: Vec<Option<LayerId>>,
    device: Vec<VisitDevice>,
    decisions: Vec<(usize, Result<(), Ineligible>)>,
}

/// Every layer in paint order: a layer's content, then its children —
/// into `order`, whose per-visit `ancestors` buffers are recycled through
/// `pool` from the last call.
fn paint_order<'a>(
    tree: &SurfaceTree,
    order: &'a mut Vec<Visit>,
    pool: &mut Vec<Vec<usize>>,
    stack: &mut Vec<(LayerId, usize)>,
) -> &'a [Visit] {
    for visit in order.drain(..) {
        pool.push(visit.ancestors);
    }
    stack.clear();
    stack.push((tree.root(), usize::MAX));
    while let Some((id, parent)) = stack.pop() {
        let index = order.len();
        let mut ancestors = pool.pop().unwrap_or_default();
        ancestors.clear();
        if parent != usize::MAX {
            ancestors.extend_from_slice(&order[parent].ancestors);
            ancestors.push(parent);
        }
        order.push(Visit { id, ancestors });
        for &child in tree.layer(id).children.iter().rev() {
            stack.push((child, index));
        }
    }
    order
}

/// The first layer in paint order the tree draws that `hit` selects — on a
/// surface without planes, the hosted layer that fails its render.
pub fn first_in_paint_order(tree: &SurfaceTree, hit: impl Fn(LayerId) -> bool) -> Option<LayerId> {
    let mut stack = vec![tree.root()];
    while let Some(id) = stack.pop() {
        if hit(id) {
            return Some(id);
        }
        stack.extend(tree.layer(id).children.iter().rev());
    }
    None
}

/// Every visit's content-space transform and device-space clip bounds —
/// the footprint a "layer above" check intersects a candidate's rect
/// against.
fn devices(tree: &SurfaceTree, order: &[Visit], device: &mut Vec<VisitDevice>) {
    device.clear();
    for visit in order {
        let mut space = Affine::IDENTITY;
        let mut bounds = None;
        let mut projective = false;
        for id in visit
            .ancestors
            .iter()
            .map(|&a| order[a].id)
            .chain([visit.id])
        {
            let level = tree.layer(id);
            projective |= tree.projective_pose(id).is_some();
            let own = space * level.transform;
            if let Some(clip) = &level.clip {
                let clipped = own.transform_rect_bbox(clip.bounds());
                bounds = Some(bounds.map_or(clipped, |outer: Rect| outer.intersect(clipped)));
            }
            space = own * Affine::translate(-level.scroll_offset);
        }
        // A pose replaces the affine transform. Without projecting its
        // full clipped footprint, no finite affine bound is a safe proof
        // of non-overlap, including for descendants of the posed layer.
        device.push(VisitDevice {
            space,
            bounds: if projective { None } else { bounds },
        });
    }
}

/// Whether the layer's painted output is not provably opaque: its own
/// opacity below one, recorded content that may paint translucency,
/// installed content whose alpha is the producer's, or a filter.
fn translucent(tree: &SurfaceTree, id: LayerId) -> bool {
    let node = tree.layer(id);
    node.opacity < 1.0 || node.content_translucent() || node.filter.is_some()
}

/// The first layer at or after each index that samples a backdrop or
/// blends onto the surface, scanning from the top of the paint order —
/// index `order.len()` is the empty suffix.
fn suffixes(
    tree: &SurfaceTree,
    order: &[Visit],
    backdrop_above: &mut Vec<Option<LayerId>>,
    blend_above: &mut Vec<Option<LayerId>>,
) {
    backdrop_above.clear();
    backdrop_above.resize(order.len() + 1, None);
    blend_above.clear();
    blend_above.resize(order.len() + 1, None);
    for (i, visit) in order.iter().enumerate().rev() {
        let node = tree.layer(visit.id);
        backdrop_above[i] = if node.backdrop.is_some() {
            Some(visit.id)
        } else {
            backdrop_above[i + 1]
        };
        let at_surface = visit
            .ancestors
            .iter()
            .all(|&a| !isolates(tree, order[a].id));
        blend_above[i] = if at_surface && node.blend != BlendMode::Normal {
            Some(visit.id)
        } else {
            blend_above[i + 1]
        };
    }
}

/// Each offered candidate's verdict in paint order as its `order`
/// index — `Err(Budget)` once `C::BUDGET` promotions are taken. The
/// offered set is `candidates` intersected with `ready`, plus every
/// mandatory candidate: a candidate whose realization is still pending is
/// absent from the verdicts entirely, not rejected, and mandatory content
/// is never pending.
fn verdicts<'a, C: Compositor>(
    tree: &'a SurfaceTree,
    order: &'a [Visit],
    candidates: &'a FxHashMap<LayerId, Candidate>,
    ready: &'a FxHashSet<LayerId>,
    above: (&'a [Option<LayerId>], &'a [Option<LayerId>]),
    device: &'a [VisitDevice],
    decisions: &'a mut Vec<(usize, Result<(), Ineligible>)>,
) -> impl Iterator<Item = (usize, Result<(), Ineligible>)> + 'a {
    let (backdrop_above, blend_above) = above;
    decisions.clear();
    decisions.extend(order.iter().enumerate().filter_map(move |(i, visit)| {
        let &size = candidates.get(&visit.id)?;
        if !size.source.mandatory() && !ready.contains(&visit.id) {
            return None;
        }
        let verdict = judge::<C>(
            tree,
            order,
            i,
            size,
            backdrop_above[i + 1],
            blend_above[i + 1],
            device,
        );
        Some((i, verdict))
    }));
    let mut promoted = 0;
    for source in Source::BUDGET_ORDER {
        for (i, verdict) in decisions.iter_mut() {
            if candidates[&order[*i].id].source == source && verdict.is_ok() {
                if promoted < C::BUDGET {
                    promoted += 1;
                } else {
                    *verdict = Err(Ineligible::Budget(C::BUDGET));
                }
            }
        }
    }
    decisions.drain(..)
}

/// Decides which of `candidates` (layer to content size) the platform
/// reports `ready` are promoted this frame on a surface realized by
/// compositor `C`.
///
/// Eligible mandatory candidates receive the budget first, then external
/// frames, then recorded captures; ties within each class use paint order.
/// Output remains in paint order.
#[must_use]
pub fn plan<C: Compositor>(
    tree: &SurfaceTree,
    candidates: &FxHashMap<LayerId, Candidate>,
    ready: &FxHashSet<LayerId>,
) -> Plan {
    let mut result = Plan::default();
    plan_with::<C>(
        tree,
        candidates,
        ready,
        &mut PlanScratch::default(),
        &mut result,
    );
    result
}

/// Rebuild a plan while retaining its workspace and placement-path buffers.
pub fn plan_with<C: Compositor>(
    tree: &SurfaceTree,
    candidates: &FxHashMap<LayerId, Candidate>,
    ready: &FxHashSet<LayerId>,
    scratch: &mut PlanScratch,
    plan: &mut Plan,
) {
    plan.rejected.clear();
    plan.unplaced.clear();
    if candidates.is_empty() {
        plan.planes.clear();
        plan.trailing = false;
        return;
    }
    let PlanScratch {
        order,
        pool,
        stack,
        backdrop_above,
        blend_above,
        device,
        decisions,
    } = scratch;
    let order = paint_order(tree, order, pool, stack);
    suffixes(tree, order, backdrop_above, blend_above);
    devices(tree, order, device);
    let mut promoted = 0;
    let mut last = None;
    for (i, verdict) in verdicts::<C>(
        tree,
        order,
        candidates,
        ready,
        (backdrop_above, blend_above),
        device,
        decisions,
    ) {
        match verdict {
            Ok(()) => {
                last = Some(i);
                let size = candidates[&order[i].id];
                if promoted == plan.planes.len() {
                    plan.planes.push(Placement {
                        layer: order[i].id,
                        source: size.source,
                        size: size.size,
                        raster: size.raster,
                        opacity: 1.0,
                        path: Vec::new(),
                    });
                }
                placement(tree, order, i, size, &mut plan.planes[promoted]);
                promoted += 1;
            }
            Err(cause) if candidates[&order[i].id].source.mandatory() => {
                plan.unplaced.push((order[i].id, cause));
            }
            Err(cause) => plan.rejected.push((order[i].id, cause)),
        }
    }
    plan.planes.truncate(promoted);
    plan.trailing = last.is_some_and(|i| i + 1 < order.len());
}

/// Whether `order[i]`'s placement for content `size` is `placed` — the
/// fields `placement` would put in it, compared without building them.
fn placement_eq(
    tree: &SurfaceTree,
    order: &[Visit],
    i: usize,
    size: Candidate,
    placed: &Placement,
) -> bool {
    let visit = &order[i];
    (placed.layer, placed.size, placed.raster, placed.source)
        == (visit.id, size.size, size.raster, size.source)
        && placed.opacity == tree.layer(visit.id).opacity
        && placed.path.len() == visit.ancestors.len() + 1
        && visit
            .ancestors
            .iter()
            .map(|&a| order[a].id)
            .chain([visit.id])
            .zip(&placed.path)
            .all(|(id, level)| {
                let node = tree.layer(id);
                level.layer == id
                    && level.transform == node.transform
                    && level.clip == node.clip
                    && level.scroll == node.scroll_offset
            })
}

/// Whether `committed` is the plan the same verdicts would produce over
/// `tree` and the `ready` subset of `candidates` — checked without
/// materialising it (#90).
fn same_plan<C: Compositor>(
    committed: &Plan,
    tree: &SurfaceTree,
    candidates: &FxHashMap<LayerId, Candidate>,
    ready: &FxHashSet<LayerId>,
    scratch: &mut PlanScratch,
) -> bool {
    if candidates.is_empty() {
        return *committed == Plan::default();
    }
    if !committed.unplaced.is_empty() {
        return false;
    }
    let PlanScratch {
        order,
        pool,
        stack,
        backdrop_above,
        blend_above,
        device,
        decisions,
    } = scratch;
    let order = paint_order(tree, order, pool, stack);
    suffixes(tree, order, backdrop_above, blend_above);
    devices(tree, order, device);
    let mut planes = committed.planes.iter();
    let mut rejected = committed.rejected.iter();
    let mut last = None;
    for (i, verdict) in verdicts::<C>(
        tree,
        order,
        candidates,
        ready,
        (backdrop_above, blend_above),
        device,
        decisions,
    ) {
        match verdict {
            Ok(()) => {
                let Some(placed) = planes.next() else {
                    return false;
                };
                if !placement_eq(tree, order, i, candidates[&order[i].id], placed) {
                    return false;
                }
                last = Some(i);
            }
            // A plan with unplaced content is never presented, so it is
            // never the committed one.
            Err(_) if candidates[&order[i].id].source.mandatory() => return false,
            Err(cause) => {
                if rejected.next() != Some(&(order[i].id, cause)) {
                    return false;
                }
            }
        }
    }
    planes.next().is_none()
        && rejected.next().is_none()
        && last.is_some_and(|i| i + 1 < order.len()) == committed.trailing
}

/// The eligibility rules for the candidate at `order[i]`, in a fixed order
/// so the named cause is deterministic: the mandatory-plane rule every
/// candidate must pass, then — for content the engine can composite
/// instead — the rules that keep promotion invisible.
fn judge<C: Compositor>(
    tree: &SurfaceTree,
    order: &[Visit],
    i: usize,
    size: Candidate,
    backdrop_above: Option<LayerId>,
    blend_above: Option<LayerId>,
    device: &[VisitDevice],
) -> Result<(), Ineligible> {
    mandatory_plane::<C>(tree, order, i, size.source, backdrop_above, blend_above)?;
    if size.source.mandatory() {
        return Ok(());
    }
    invisible::<C>(tree, order, i, size, device)
}

/// The mandatory-plane rule: whether the system compositor can show the
/// candidate at `order[i]` where the engine would, with nothing the engine
/// composites needing its pixels. Content that is shown only on a plane
/// fails with exactly these causes.
fn mandatory_plane<C: Compositor>(
    tree: &SurfaceTree,
    order: &[Visit],
    i: usize,
    source: Source,
    backdrop_above: Option<LayerId>,
    blend_above: Option<LayerId>,
) -> Result<(), Ineligible> {
    let visit = &order[i];
    let node = tree.layer(visit.id);
    if let Some(&a) = visit
        .ancestors
        .iter()
        .find(|&&a| isolates(tree, order[a].id))
    {
        return Err(Ineligible::Isolated(order[a].id));
    }
    if node.filter.is_some() {
        return Err(Ineligible::Filter);
    }
    if node.blend != BlendMode::Normal || node.blends_within() {
        return Err(Ineligible::Blend);
    }
    if node.backdrop.is_some() {
        return Err(Ineligible::Backdrop);
    }
    if let Some(layer) = backdrop_above {
        return Err(Ineligible::BackdropAbove(layer));
    }
    if let Some(layer) = blend_above {
        return Err(Ineligible::BlendAbove(layer));
    }
    if node.opacity < 1.0 && !node.children.is_empty() {
        return Err(Ineligible::GroupOpacity);
    }
    let hosted = source == Source::Hosted;
    if hosted && node.opacity < 1.0 && !C::HOSTS_OPACITY {
        return Err(Ineligible::Opacity);
    }
    for id in path(order, visit) {
        let level = tree.layer(id);
        // A projective level's placement is its pose, not `transform` —
        // a plane's affine levels cannot carry it. An isolating ancestor
        // was already named `Isolated`, so this can only be the candidate.
        let expressed = if hosted {
            C::hosts_transform(level.transform)
        } else {
            C::expresses_transform(level.transform)
        };
        if tree.projective_pose(id).is_some() || !expressed {
            return Err(Ineligible::Transform(id));
        }
        if level
            .clip
            .as_ref()
            .is_some_and(|clip| !C::expresses_clip(clip))
        {
            return Err(Ineligible::Clip(id));
        }
    }
    Ok(())
}

/// The rules that keep an opportunistic promotion invisible: the engine
/// would composite the candidate at `order[i]` at the surface level, and
/// nothing painted above it blends differently on the system compositor.
fn invisible<C: Compositor>(
    tree: &SurfaceTree,
    order: &[Visit],
    i: usize,
    size: Candidate,
    device: &[VisitDevice],
) -> Result<(), Ineligible> {
    // The engine merges nested clips in place only while at most one of
    // them is not a device-aligned rectangle (`Lowering::run_clipped`).
    let mut shaped_clip = false;
    let mut space = Affine::IDENTITY;
    for id in path(order, &order[i]) {
        let level = tree.layer(id);
        let own = space * level.transform;
        if let Some(clip) = &level.clip
            && !(matches!(clip, ShapeData::Rect(_)) && axis_aligned(own))
        {
            if shaped_clip {
                return Err(Ineligible::NestedClip(id));
            }
            shaped_clip = true;
        }
        space = own * Affine::translate(-level.scroll_offset);
    }
    // The plane shows the frame inside the path's clips only: content a
    // clip cuts away cannot overlap a layer above.
    let rect = (device[i].space * size.raster).transform_rect_bbox(Rect::new(
        0.0,
        0.0,
        f64::from(size.size.0),
        f64::from(size.size.1),
    ));
    let rect = device[i]
        .bounds
        .map_or(rect, |bounds| bounds.intersect(rect));
    for j in i + 1..order.len() {
        // Children of an isolated layer still contribute alpha to its
        // output. Inspect them too, rather than treating isolation as
        // proof that the group will composite opaquely over the video.
        if translucent(tree, order[j].id)
            && device[j].bounds.is_none_or(|bounds| bounds.overlaps(rect))
        {
            return Err(Ineligible::TranslucentAbove(order[j].id));
        }
    }
    Ok(())
}

/// The layers from the root to `visit`, root first.
fn path<'a>(order: &'a [Visit], visit: &'a Visit) -> impl Iterator<Item = LayerId> + 'a {
    visit
        .ancestors
        .iter()
        .map(|&a| order[a].id)
        .chain([visit.id])
}

fn placement(
    tree: &SurfaceTree,
    order: &[Visit],
    i: usize,
    size: Candidate,
    placed: &mut Placement,
) {
    let visit = &order[i];
    placed.path.clear();
    placed.path.extend(
        visit
            .ancestors
            .iter()
            .map(|&a| order[a].id)
            .chain([visit.id])
            .map(|id| {
                let node = tree.layer(id);
                Level {
                    layer: id,
                    transform: node.transform,
                    clip: node.clip.clone(),
                    scroll: node.scroll_offset,
                }
            }),
    );
    placed.layer = visit.id;
    placed.source = size.source;
    placed.size = size.size;
    placed.raster = size.raster;
    placed.opacity = tree.layer(visit.id).opacity;
}

/// Whether a frame whose only committed change is new external frames on
/// `layers` can present through the planes alone: every changed layer is
/// promoted by the surface's committed `plan`, and the verdicts the
/// frame's tree and the `ready` subset of the fresh `candidates` produce
/// are the committed ones — the plan saw the same readiness filter, so
/// a candidate still pending realization neither fails the check nor
/// reaches a plane, and a newly ready candidate changes the verdicts and
/// keeps the full path. A new frame's different size, or a new frame no
/// plane can show, changes the candidate set and fails the check like any
/// other change (#90). `scratch` reuses the walk's buffers across calls,
/// so a steady stream allocates nothing.
#[must_use]
pub fn frames_only<C: Compositor>(
    plan: &Plan,
    tree: &SurfaceTree,
    candidates: &FxHashMap<LayerId, Candidate>,
    ready: &FxHashSet<LayerId>,
    layers: &FxHashSet<LayerId>,
    scratch: &mut PlanScratch,
) -> bool {
    !layers.is_empty()
        && layers
            .iter()
            .all(|layer| plan.planes.iter().any(|plane| plane.layer == *layer))
        && same_plan::<C>(plan, tree, candidates, ready, scratch)
}

/// The content a plane shows.
#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "android")),
    expect(
        dead_code,
        reason = "read by the platform realizations of `SystemPlanes`"
    )
)]
pub enum PlaneContent<'a> {
    /// An immutable capture of a recorded layer in linear Display P3.
    Raster {
        /// Pixels for a new native capture; absent after publication.
        view: Option<&'a wgpu::TextureView>,
        /// The capture's content version.
        generation: u64,
    },
    /// A retained external frame, handed to the system compositor instead of
    /// being sampled by the engine.
    Frame {
        /// The installed frame.
        frame: &'a ExternalFrame,
        /// Bumped every time a new frame is installed on the layer.
        generation: u64,
    },
    /// A system layer the host supplies and draws: the realization places
    /// the object itself. This is the only place a hosted object reaches —
    /// it is no texture, so nothing in the engine can sample it.
    Hosted {
        /// The host's platform object.
        object: &'a Hosted,
        /// Its extent in the layer's content coordinates.
        extent: kurbo::Size,
    },
}

/// The platform object a hosted plane shows
/// (`cherenkov::HostedLayers::Object`).
#[cfg(target_vendor = "apple")]
pub type Hosted = crate::interop::apple::HostedLayer;
/// The platform object a hosted plane shows
/// (`cherenkov::HostedLayers::Object`).
#[cfg(target_os = "android")]
pub type Hosted = crate::interop::android::HostedSurface;
/// No hosted object exists where the platform has no plane realization.
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
pub type Hosted = NoHosted;

/// Stands in for a hosted object on platforms without plane realization:
/// no value exists, so no layer there hosts one.
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
#[derive(Debug)]
pub enum NoHosted {}

/// A layer's hosted content: the platform object and its extent.
#[derive(Debug)]
pub struct HostedBinding {
    /// The host's platform object.
    pub object: Hosted,
    /// Its extent in the layer's content coordinates.
    pub extent: kurbo::Size,
}

impl HostedBinding {
    /// The plane candidate the binding offers: its extent rounded out to
    /// whole units, placed at the content origin.
    #[must_use]
    pub const fn candidate(&self) -> Candidate {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a hosted extent is a finite, non-negative layout size"
        )]
        let size = (
            self.extent.width.ceil() as u32,
            self.extent.height.ceil() as u32,
        );
        Candidate {
            size,
            raster: Affine::IDENTITY,
            source: Source::Hosted,
        }
    }
}

/// One promoted plane of a [`Composition`].
#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "android")),
    expect(
        dead_code,
        reason = "read by the platform realizations of `SystemPlanes`"
    )
)]
pub struct Plane<'a> {
    /// Where it sits.
    pub placement: &'a Placement,
    /// What it shows.
    pub content: PlaneContent<'a>,
}

/// One engine part of a [`Composition`]: premultiplied linear Display P3 at
/// the surface size.
#[derive(Debug)]
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "android")),
    expect(
        dead_code,
        reason = "read by the platform realizations of `SystemPlanes`"
    )
)]
pub struct Part<'a> {
    /// The engine texture holding the part.
    pub view: &'a wgpu::TextureView,
}

/// A surface's stack for one frame, bottom first: `parts[0]`, `planes[0]`,
/// `parts[1]`, `planes[1]`, … and, when [`Plan::trailing`] holds, a last
/// part above the last plane. Without promoted planes there is exactly one
/// part, the whole surface.
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "android")),
    expect(
        dead_code,
        reason = "read by the platform realizations of `SystemPlanes`"
    )
)]
pub struct Composition<'a> {
    /// The engine's device.
    pub device: &'a wgpu::Device,
    /// The engine's queue.
    pub queue: &'a wgpu::Queue,
    /// Presents an engine part into a platform drawable, tone-mapping to the
    /// display's headroom.
    pub presenter: &'a mut Presenter,
    /// The surface size in device pixels.
    pub size: (u32, u32),
    /// The display the surface is on.
    pub display: Display,
    /// The engine parts, bottom first.
    pub parts: &'a [Part<'a>],
    /// The promoted planes, bottom first.
    pub planes: &'a [Plane<'a>],
}

/// Whether presentation finished or which event must resume it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Presented,
    /// Drawable availability requires another display frame.
    Retry,
    /// A queued operation will wake the surface when it completes.
    #[cfg(target_vendor = "apple")]
    Pending,
}

/// A platform's realization of a surface's planes under the host's
/// system-compositor parent.
///
/// The render thread calls [`SystemPlanes::compose`] once per rendered
/// frame with the whole stack; the realization makes the system tree match
/// it, atomically where the platform allows, and presents the parts.
pub trait SystemPlanes: Compositor {
    /// The frame's commit for the platform compositor's own thread,
    /// returned by [`SystemPlanes::compose`] instead of being run from
    /// the render thread: on Apple the `CATransaction` of layer geometry
    /// and per-part drawable presents, which the frame's awaiting caller
    /// applies on the main thread so a part's drawable is never acquired
    /// while its predecessor still waits to present. `Default` is the
    /// empty commit a frame that commits nothing off-thread produces.
    type Commit: cherenkov::RenderTransfer + Default + 'static;

    /// Native immutable capture allocations, excluding the engine's source.
    fn captured_bytes(&self) -> u64 {
        0
    }
    /// Hands supported tracks to the committed native layer tree. Called
    /// only after a successful presentation of `plan`.
    fn animate(&mut self, _tree: &SurfaceTree, _plan: &Plan) {}

    /// Layers whose complete property animation is compositor-owned.
    fn owned_animations(&self) -> &[LayerId] {
        &[]
    }

    /// Withdraws scheduling ownership when this frame could not present.
    fn withdraw_animations(&mut self) {}

    /// Realizes `composition`, distinguishing display-paced acquisition
    /// from asynchronous work that supplies its own completion wake.
    /// Returns the presentation outcome and the frame's
    /// [`SystemPlanes::Commit`].
    ///
    /// # Errors
    /// A [`RenderError`] naming the cause when the system rejects a plane
    /// or a part cannot be presented.
    fn compose(
        &mut self,
        composition: Composition<'_>,
    ) -> Result<(Presentation, Self::Commit), RenderError>;

    /// Presents only the promoted planes' new frames: `frames` carries
    /// every promoted layer whose frame changed this frame, inside the
    /// transaction or equivalent atomic update the platform composes, and
    /// every part's shown buffer stays in place — the engine did no work
    /// for this frame, so there is nothing to blit and no buffer to
    /// acquire. Called only for a frame [`frames_only`] admitted, so the
    /// stack itself is the one `compose` last realized (#90).
    ///
    /// # Errors
    /// A [`RenderError`] naming the cause when the system rejects a plane.
    fn refresh<'a>(&mut self, frames: impl Iterator<Item = Plane<'a>>) -> Result<(), RenderError>;

    /// Queues whatever realization a candidate still needs — a display
    /// layer another thread must create — and notes when one first
    /// becomes able to show a frame, without materializing a set. Runs at
    /// render admission for every promotion-capable surface, dirty or
    /// not, so a deferred realization that completes between renders is
    /// seen on the next one.
    fn groom(&mut self, candidates: &FxHashMap<LayerId, Candidate>) {
        let _ = candidates;
    }

    /// Queues realization with the installed frames available for a
    /// platform readiness probe. The default keeps synchronous platforms'
    /// behavior unchanged.
    fn groom_with_frames(
        &mut self,
        candidates: &FxHashMap<LayerId, Candidate>,
        frames: &FxHashMap<LayerId, (ExternalFrame, u64)>,
    ) {
        let _ = frames;
        self.groom(candidates);
    }

    /// Whether the last [`SystemPlanes::groom`] found a newly ready
    /// candidate the plan has not been offered: the renderer re-lowers
    /// the surface so the plan can promote it. `false` for synchronous
    /// realizations, which offer every candidate at once.
    fn wants_plan(&self) -> bool {
        false
    }

    /// `groom` plus the subset of `candidates` that can show a frame now,
    /// filled into `ready`, which the caller keeps between calls so a
    /// prepare allocates nothing steady-state. A platform that realizes
    /// a plane asynchronously reports a candidate ready only once its
    /// realization has completed; until then the layer keeps compositing
    /// in-engine. The default reports every candidate, which suits
    /// synchronous realizations.
    fn prepare(
        &mut self,
        candidates: &FxHashMap<LayerId, Candidate>,
        frames: &FxHashMap<LayerId, (ExternalFrame, u64)>,
        ready: &mut FxHashSet<LayerId>,
    ) {
        self.groom_with_frames(candidates, frames);
        ready.clear();
        ready.extend(candidates.keys().copied());
    }

    /// The surface was resized to `size` device pixels.
    fn resize(&mut self, size: (u32, u32));

    /// A display move or a scale change re-runs every part's output
    /// negotiation — each part's [`WindowSurface::reselect`].
    ///
    /// # Errors
    /// A part's [`WindowSurface::reselect`] error: the surface no longer
    /// advertises what the host's request needs.
    fn reselect(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Result<(), SurfaceError>;
}

#[cfg(target_vendor = "apple")]
pub mod apple;

/// The realization on this platform: Core Animation layer planes.
#[cfg(target_vendor = "apple")]
pub type Platform = apple::LayerPlanes;
/// The realization on this platform: child surface controls on Android.
#[cfg(target_os = "android")]
pub type Platform = super::surface_control::planes::Planes;
/// The realization on this platform.
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
pub type Platform = NoPlanes;

/// Stands in for [`SystemPlanes`] on platforms without a realization yet:
/// no value exists, so a surface there never has planes.
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
#[derive(Debug)]
pub enum NoPlanes {}

#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
impl Compositor for NoPlanes {
    const BUDGET: usize = 0;
    const HOSTS_OPACITY: bool = false;
    fn expresses_transform(_: Affine) -> bool {
        false
    }
    fn expresses_clip(_: &ShapeData) -> bool {
        false
    }
    fn shows(_: &ExternalFrame) -> bool {
        false
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
impl SystemPlanes for NoPlanes {
    type Commit = ();

    fn compose(&mut self, _: Composition<'_>) -> Result<(Presentation, ()), RenderError> {
        unreachable!("no `NoPlanes` value exists")
    }
    fn refresh<'a>(&mut self, _: impl Iterator<Item = Plane<'a>>) -> Result<(), RenderError> {
        unreachable!("no `NoPlanes` value exists")
    }
    fn resize(&mut self, _: (u32, u32)) {
        unreachable!("no `NoPlanes` value exists")
    }
    fn reselect(&mut self, _: &wgpu::Adapter, _: &wgpu::Device) -> Result<(), SurfaceError> {
        unreachable!("no `NoPlanes` value exists")
    }
}

#[cfg(test)]
mod tests;
