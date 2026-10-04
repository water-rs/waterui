//! Lowering: a surface's layer tree and display lists become one list of
//! instanced-quad passes.

use cherenkov::lowering::Realization;
use std::ops::Range;

use rustc_hash::FxHashMap;

use super::MAX_PASS_INSTANCES;
use super::prepared::{ClipShape, Op, Outline, PaintData, ResolvedPaint, box_shape};
use cherenkov::kurbo::{self, Affine, BezPath, PathEl, Point, Rect, Vec2};
use cherenkov::{FillRule, GlyphRun, ShapeData, WorkingColor};

use cherenkov::RenderError;

use crate::names;
use cherenkov::{LayerId, ProducerId, SurfaceTree};

use crate::render::GpuImage;
use crate::render::filter::FilterKey;

use crate::render::glyph::{self, Atlas, FontData, MaskCell, PathEmit, PendingRaster, glyph_key};
use crate::render::instance::{
    FLAG_BLEND_SRC, FLAG_HAS_CLIP, FLAG_HAS_INNER, FLAG_HAS_MASK, FLAG_MASK_TEXTURE, FLAG_TEX_SRGB,
    Globals, Instance, KIND_FILL, KIND_GLYPH, KIND_REGION, KIND_SHADOW, KIND_SPAN,
    KIND_STROKE_DIST, KIND_STROKE_OFFSET, PAINT_IMAGE, PAINT_PROJECTIVE, PAINT_SOLID,
    PAINT_TEXTURE, Shape, Stop, affine, blend_code,
};
use crate::render::path;
use crate::render::projective::{LocalKey, Placement};

/// The target a pass draws into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The surface's render texture for engine part `n`: the whole surface
    /// is part 0, and every layer promoted to a system plane starts the
    /// next part above it (`render::planes`).
    Part(u32),
    /// Scratch texture at this isolation depth index.
    Scratch(usize),
    /// A backdrop group's capture texture for this region index.
    Backdrop { group: u64, region: u32 },
    /// A projective layer's local image, base level (#84).
    Projected(LocalKey),
    /// A recorded layer captured for the system compositor.
    Plane(LayerId),
}

/// The texture a draw range samples at bind group 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Scratch texture at this isolation depth index.
    Scratch(usize),
    /// A backdrop group's capture texture for this region index.
    Backdrop { group: u64, region: u32 },
    /// A projective layer's local image with its mips (#84).
    Projected(LocalKey),
}

/// The blend pipeline a draw range uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PipelineKind {
    /// Fixed-function source-over compositing.
    #[default]
    SrcOver,
    /// Write the fragment result unblended; blended composites do their
    /// compositing in the shader.
    Replace,
    /// A registered backdrop effect shader's pipeline (its raw id).
    Effect(u64),
    /// The projective composite (#84): source-over, or `replace` for a
    /// blended composite that reads the backdrop copy itself.
    Projective { replace: bool },
}

/// The specialised fragment pipeline a range draws with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaderVariant {
    /// Solid fills, spans, and glyphs: coverage + opacity + solid colour.
    #[default]
    Simple,
    /// The shadow kernel + solid colour.
    Shadow,
    /// Everything else: clips, masks, strokes, gradients, composites.
    Full,
}

/// A texture identity scoped to its resource owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ImageSource {
    Registered(u64),
    Bitmap(super::bitmap::BitmapKey),
    /// The current frame a producer's bindings share
    /// (`cherenkov::GpuContent`), by `ProducerId`: the range draws the
    /// external pipeline against the producer's slot bind, not
    /// `image_tex`.
    Content(ProducerId),
    Shader(std::sync::Arc<super::paint::Key>),
}

/// One draw call's instance range and bound source texture.
#[derive(Clone, Debug)]
pub struct DrawRange {
    /// The source texture bound as group 1, `None` for the dummy texture.
    pub source: Option<Source>,
    /// The image texture bound for `PAINT_IMAGE` instances.
    pub image: Option<ImageSource>,
    /// The mask texture bound at group-1 binding 3, by key.
    pub mask: Option<u64>,
    /// The pipeline variant this range draws with.
    pub pipeline: PipelineKind,
    /// The fragment-shader variant this range draws with.
    pub variant: ShaderVariant,
    /// Range into the frame's instance buffer.
    pub instances: Range<u32>,
}

/// One render pass.
#[derive(Clone, Debug)]
pub struct Pass {
    /// What it draws into.
    pub target: Target,
    /// Clear colour; `None` loads the previous contents.
    pub clear: Option<[f32; 4]>,
    /// Draw calls.
    pub ranges: Vec<DrawRange>,
    /// The space the target's premultiplied pixels are stored in: the
    /// pass writes and blends in it. `Linear` everywhere except an
    /// isolate scratch for a declared [`cherenkov::BlendSpace::SrgbEncoded`]
    /// group, or a capture of one.
    pub space: cherenkov::BlendSpace,
    /// Device-space `(x, y, w, h)` of the target this pass covers: the
    /// whole surface for [`Target::Part`], the tight union bbox of its
    /// content for [`Target::Scratch`].
    pub region: [u32; 4],
    /// When set, the target's region is copied into the backdrop texture
    /// before this pass begins — the blend composite reads it explicitly.
    pub backdrop_copy: Option<[u32; 4]>,
    /// When set, `region` is copied from `copy_from` into the group's
    /// capture texture before this pass draws — only on
    /// [`Target::Backdrop`] passes.
    pub capture: Option<Capture>,
}

/// A backdrop capture's copy source, on a [`Target::Backdrop`] pass.
#[derive(Clone, Copy, Debug)]
pub struct Capture {
    /// The group's raw id.
    pub group: u64,
    /// The capture's index in the group's region list.
    pub region: u32,
    /// The nearest semantic isolation's target: [`Target::Part`] or a
    /// [`Target::Scratch`] whose region is guaranteed the full surface.
    pub copy_from: Target,
}

/// A lowered frame.
#[derive(Default)]
pub struct Frame {
    /// Instance data.
    pub instances: Vec<Instance>,
    /// Gradient stops.
    pub stops: Vec<Stop>,
    /// Passes in submission order.
    pub passes: Vec<Pass>,
    /// Producer bindings drawn this frame, composited in-engine or
    /// promoted to a plane: `(producer, requested size)`, one entry per
    /// drawn binding — a producer can appear once per binding.
    pub content: Vec<(ProducerId, (u32, u32))>,
    /// Producers whose current frame composed on this frame, in paint
    /// order — rendered and submitted alike.
    pub external: Vec<ProducerId>,
    pub filters: Vec<(usize, FilterKey)>,
    pub shadows: Vec<(usize, super::shadow::Parameters)>,
    /// Local images whose mip chains build after the pass at the index:
    /// the last pass of their realization.
    pub mips: Vec<(usize, LocalKey)>,
    open: Option<OpenPass>,
}

#[derive(Clone)]
struct OpenPass {
    target: Target,
    clear: Option<[f32; 4]>,
    /// The target's storage space, resolved when the pass opened.
    space: cherenkov::BlendSpace,
    source: Option<Source>,
    image: Option<ImageSource>,
    /// The mask texture bound at group-1 binding 3, by key.
    mask: Option<u64>,
    pipeline: PipelineKind,
    variant: ShaderVariant,
    backdrop_copy: Option<[u32; 4]>,
    capture: Option<Capture>,
    ranges: Vec<DrawRange>,
    seg_start: u32,
}

/// A rollback point for [`Frame::restore`] (used by speculative
/// pass-through layers).
struct FrameSnapshot {
    instances: usize,
    stops: usize,
    passes: usize,
    filters: usize,
    shadows: usize,
    open: Option<OpenPass>,
}

impl Frame {
    /// Captures the frame's emission state.
    fn snapshot(&self) -> FrameSnapshot {
        FrameSnapshot {
            instances: self.instances.len(),
            stops: self.stops.len(),
            passes: self.passes.len(),
            filters: self.filters.len(),
            shadows: self.shadows.len(),
            open: self.open.clone(),
        }
    }

    /// Reverts every emission since `snap`.
    fn restore(&mut self, snap: FrameSnapshot) {
        self.instances.truncate(snap.instances);
        self.stops.truncate(snap.stops);
        self.passes.truncate(snap.passes);
        self.filters.truncate(snap.filters);
        self.shadows.truncate(snap.shadows);
        self.open = snap.open;
    }

    /// Removes pass `i` and shifts the pass indexes recorded in `shadows`
    /// and `filters` down past it.
    fn remove_pass(&mut self, i: usize) {
        fn shift<T>(indexed: &mut Vec<(usize, T)>, removed: usize) {
            indexed.retain_mut(|(pass, _)| {
                if *pass == removed {
                    return false;
                }
                if *pass > removed {
                    *pass -= 1;
                }
                true
            });
        }
        self.passes.remove(i);
        shift(&mut self.shadows, i);
        shift(&mut self.filters, i);
        shift(&mut self.mips, i);
    }
}

impl Frame {
    /// Reuses this frame's allocations for the next lowering.
    pub fn reset(&mut self) {
        self.instances.clear();
        self.stops.clear();
        self.passes.clear();
        self.content.clear();
        self.external.clear();
        self.filters.clear();
        self.shadows.clear();
        self.mips.clear();
        self.open = None;
    }
}

/// A clip shape with its device-to-clip-local transform, plus its device
/// rectangle when it is axis-aligned.
#[derive(Clone, Copy, Debug)]
struct DeviceClip {
    /// Device to clip-local.
    inv: Affine,
    /// The centred clip shape.
    shape: Shape,
    /// The clip's device-space rectangle, when it is one.
    aligned_rect: Option<Rect>,
    /// A rasterized path-coverage mask.
    mask: Option<ClipMask>,
}

/// A path-clip mask: stored in the atlas or on its own texture, or
/// produced by a pending raster the render thread will store — then
/// `uv.zw` of every instance emitted under it is patched by
/// `Lowering::mask_patches`.
#[derive(Clone, Copy, Debug)]
enum ClipMask {
    /// Stored in the atlas: the atlas origin is `cell.atlas`.
    Cell(MaskCell),
    /// Stored on a dedicated texture: the mask's content-hash key,
    /// bound at group-1 binding 3.
    Texture(MaskCell, u64),
    /// Pending its raster (index into `Lowering::pending`).
    Pending(MaskCell, u32),
}

impl ClipMask {
    /// The mask data: device rect, size, and the pending-or-stored atlas
    /// origin.
    const fn cell(self) -> MaskCell {
        match self {
            Self::Cell(m) | Self::Texture(m, _) | Self::Pending(m, _) => m,
        }
    }

    /// This mask shifted by `(dx, dy)` device pixels.
    fn translated(self, dx: f64, dy: f64) -> Self {
        match self {
            Self::Cell(m) => Self::Cell(m.translated(dx, dy)),
            Self::Texture(m, key) => Self::Texture(m.translated(dx, dy), key),
            Self::Pending(m, i) => Self::Pending(m.translated(dx, dy), i),
        }
    }
}

/// f64 to f32; instance data is f32 by design.
#[expect(clippy::cast_possible_truncation)]
const fn f32_f64(v: f64) -> f32 {
    v as f32
}

/// `count` stays a valid coverage-order index: `shader.wgsl` derives each
/// instance's depth as `bitcast<f32>(0x3e000000u + ii * 8u)`, injective only
/// below `MAX_PASS_INSTANCES` — fail fast rather than alias two depths.
fn check_instance_bound(count: usize) {
    assert!(
        count <= MAX_PASS_INSTANCES,
        "a coverage-order pass emits at most MAX_PASS_INSTANCES ({MAX_PASS_INSTANCES}) instances"
    );
}

#[expect(clippy::cast_possible_truncation, reason = "bounded above")]
fn instance_index(count: usize) -> u32 {
    check_instance_bound(count);
    count as u32
}

fn aa_margin(transform: Affine) -> f64 {
    let [c0, c1, c2, c3, _, _] = transform.as_coeffs();
    let lmin = c0.hypot(c1).min(c2.hypot(c3));
    if lmin <= 1e-9 { 0.0 } else { 2.0 / lmin }
}

/// The specialised fragment pipeline `inst` requires: the uber-shader
/// when it reads rare fields (clip, mask, inner, strokes, non-solid
/// paint), the shadow kernel otherwise for shadows, and the trivial
/// coverage path for solid fills, spans, and glyphs.
const fn variant_of(inst: &Instance) -> ShaderVariant {
    let flags = inst.meta[3] >> 24;
    if (flags & (FLAG_HAS_CLIP | FLAG_HAS_MASK | FLAG_HAS_INNER)) != 0
        || inst.meta[1] != PAINT_SOLID
        || matches!(inst.meta[0], KIND_STROKE_OFFSET | KIND_STROKE_DIST)
    {
        return ShaderVariant::Full;
    }
    if inst.meta[0] == KIND_SHADOW {
        return ShaderVariant::Shadow;
    }
    ShaderVariant::Simple
}

/// The part of a shadow its following opaque fill hides, in shadow-local
/// space: the fill's rounded box minus its corner squares, as the union of
/// `wide` (corner rows excluded) and `tall` (corner columns excluded).
/// Both are inset by an antialiasing margin so the fill's edge pixels stay.
#[derive(Clone, Copy, PartialEq)]
struct Cover {
    wide: Rect,
    tall: Rect,
}

/// The four border strips of `b` minus the covered box `c`: top and
/// bottom run the full width, left and right fit between them. Strips
/// may be empty; `c` need not lie inside `b`.
const fn border_strips(b: Rect, c: Rect) -> [Rect; 4] {
    [
        Rect::new(b.x0, b.y0, b.x1, c.y0),
        Rect::new(b.x0, c.y1, b.x1, b.y1),
        Rect::new(b.x0, c.y0, c.x0, c.y1),
        Rect::new(c.x1, c.y0, b.x1, c.y1),
    ]
}

/// The up-to-eight strips of `b` minus the cover `c`: full-width top and
/// bottom strips above/below `wide`, the four corner blocks beside `wide`
/// but above/below `tall`, and the two side strips beside `tall`. A cover
/// box that misses `b` degenerates to the four [`border_strips`] of the
/// other; empty strips are dropped.
fn cover_strips(b: Rect, c: Cover) -> impl Iterator<Item = Rect> {
    let nonempty = |r: Rect| r.width() > 0.0 && r.height() > 0.0;
    let w = c.wide.intersect(b);
    let t = c.tall.intersect(b);
    // The strip layout below needs `w` to span the rows `t` does not;
    // swap when `t` reaches higher or lower than `w`.
    let (w, t) = if t.y0 < w.y0 || t.y1 > w.y1 {
        (t, w)
    } else {
        (w, t)
    };
    let mut rects = [Rect::ZERO; 8];
    match (nonempty(w), nonempty(t)) {
        (true, true) => {
            rects = [
                Rect::new(b.x0, b.y0, b.x1, w.y0),
                Rect::new(b.x0, w.y1, b.x1, b.y1),
                Rect::new(b.x0, w.y0, w.x0, t.y0),
                Rect::new(w.x1, w.y0, b.x1, t.y0),
                Rect::new(b.x0, t.y1, w.x0, w.y1),
                Rect::new(w.x1, t.y1, b.x1, w.y1),
                Rect::new(b.x0, t.y0, t.x0, t.y1),
                Rect::new(t.x1, t.y0, b.x1, t.y1),
            ];
        }
        (true, false) => rects[..4].copy_from_slice(&border_strips(b, w)),
        (false, true) => rects[..4].copy_from_slice(&border_strips(b, t)),
        (false, false) => rects[0] = b,
    }
    rects.into_iter().filter(move |r| nonempty(*r))
}

/// A layer's retained content and device output.
pub struct ContentData {
    pub(crate) retained: cherenkov::lowering::Content<Op, Emission>,
    pub(crate) storage: EmissionStorage,
}

impl ContentData {
    pub fn new(list: cherenkov::Picture) -> Self {
        Self {
            retained: cherenkov::lowering::Content::new(list),
            storage: EmissionStorage::default(),
        }
    }

    pub fn replace(&mut self, list: cherenkov::Picture) -> cherenkov::Picture {
        let previous = self.retained.replace(list);
        self.storage.instances.clear();
        self.storage.stops.clear();
        self.storage.templates.clear();
        self.storage.covers.clear();
        // The emissions these ranges served are gone with the picture —
        // clear them too or orphaned entries outlive every compaction
        // check (#119).
        self.storage.refs.clear();
        previous
    }

    pub fn into_picture(self) -> cherenkov::Picture {
        self.retained.into_picture()
    }

    pub fn picture(list: cherenkov::Picture) -> Self {
        Self {
            retained: cherenkov::lowering::Content::picture(list),
            storage: EmissionStorage::default(),
        }
    }

    pub fn update(&mut self, updates: Vec<cherenkov::SlotUpdate>) {
        self.retained.update(updates);
    }

    pub fn invalidate(&mut self) {
        self.retained.invalidate();
        self.storage = EmissionStorage::default();
    }

    /// Whether the content samples `resource`.
    pub fn references(&self, resource: cherenkov::ResourceId) -> bool {
        self.retained.references(resource)
    }

    /// Discards the lowering of content that samples image `id`, whose
    /// dimensions changed behind the same id.
    pub fn invalidate_image(&mut self, id: cherenkov::ImageId) {
        if self.retained.invalidate_image(id) {
            self.storage = EmissionStorage::default();
        }
    }

    pub fn trim(&mut self) {
        self.retained.trim();
        self.storage = EmissionStorage::default();
    }
}

/// A layer's device data. Leaf ranges remain independent for dirty updates.
#[derive(Default)]
pub struct EmissionStorage {
    templates: Vec<InstanceTemplate>,
    covers: Vec<Cover>,
    pub(crate) instances: Vec<RetainedInstance>,
    stops: Vec<Stop>,
    /// `(shelf slot, band epoch)` atlas references every retained
    /// emission samples, addressed by `Emission::refs` ranges.
    pub(crate) refs: Vec<(u32, u64)>,
}

impl EmissionStorage {
    /// Reclaim obsolete ranges after patches without rebuilding valid leaves.
    fn compact(&mut self, emissions: &mut [Realization<Emission>]) {
        if self.templates.is_empty() {
            return;
        }
        let (instances, stops, templates, covers, refs) = emissions
            .iter()
            .filter_map(|e| e.data.as_ref())
            .fold((0, 0, 0, 0, 0), |(i, s, t, c, r), e| {
                (
                    i + e.instances.len(),
                    s + e.stops.len(),
                    t + 1,
                    c + usize::from(e.cover.is_some()),
                    r + e.refs_len(),
                )
            });
        if self.instances.len() <= instances * 2
            && self.stops.len() <= stops * 2
            && self.templates.len() <= templates * 2
            && self.covers.len() <= covers * 2
            && self.refs.len() <= refs * 2
        {
            return;
        }
        let mut storage = Self {
            templates: Vec::with_capacity(templates),
            covers: Vec::with_capacity(covers),
            instances: Vec::with_capacity(instances),
            stops: Vec::with_capacity(stops),
            refs: Vec::with_capacity(refs),
        };
        for e in emissions.iter_mut().filter_map(|e| e.data.as_mut()) {
            storage.templates.push(self.templates[e.template]);
            e.template = storage.templates.len() - 1;
            if let Some(cover) = &mut e.cover {
                storage.covers.push(self.covers[*cover]);
                *cover = storage.covers.len() - 1;
            }
            let first = storage.instances.len();
            storage
                .instances
                .extend_from_slice(&self.instances[e.instances.clone()]);
            e.instances = first..storage.instances.len();
            let first = storage.stops.len();
            storage
                .stops
                .extend_from_slice(&self.stops[e.stops.clone()]);
            e.stops = first..storage.stops.len();
            if e.refs & DEFERRED_REFS != 0 {
                // A deferred `refs` has no pairs yet — nothing moves
                // (#119).
            } else {
                let first = storage.refs.len();
                storage.refs.extend_from_slice(&self.refs[e.refs_range()]);
                e.refs = Emission::pack_refs(first, storage.refs.len() - first);
            }
        }
        *self = storage;
    }
}

/// The non-varying fields of an unclipped leaf. Clip fields, affine padding,
/// mask coordinates and per-quad fields are reconstructed when composed.
#[derive(Clone, Copy)]
struct InstanceTemplate {
    affine: [f32; 6],
    shape: Shape,
    inner: Shape,
    color: [f32; 4],
    grad: [f32; 4],
    grad2: [f32; 4],
    params: [f32; 2],
    paint: u32,
    packed: u32,
}

impl InstanceTemplate {
    fn new(inst: &Instance) -> Self {
        Self {
            affine: inst.affine[..6]
                .try_into()
                .expect("six affine coefficients"),
            shape: inst.shape,
            inner: inst.inner,
            color: inst.color,
            grad: inst.grad,
            grad2: inst.grad2,
            params: [inst.params[0], inst.params[1]],
            paint: inst.meta[1],
            packed: inst.meta[3],
        }
    }

    fn restore(&self) -> Instance {
        let mut inst = Instance::new(0);
        inst.affine[..6].copy_from_slice(&self.affine);
        inst.shape = self.shape;
        inst.inner = self.inner;
        inst.color = self.color;
        inst.grad = self.grad;
        inst.grad2 = self.grad2;
        inst.params[..2].copy_from_slice(&self.params);
        inst.meta[1] = self.paint;
        inst.meta[3] = self.packed;
        inst
    }
}

/// Fields that vary within one realized leaf. Shape, placement and paint are
/// shared by all its quads, including a box's interior/border split.
#[derive(Clone, Copy)]
pub struct RetainedInstance {
    bounds: [f32; 4],
    pub(crate) uv: [f32; 2],
    pub(crate) kind: u32,
    first_stop: u32,
}

impl RetainedInstance {
    fn restore(self, template: &InstanceTemplate, stop_base: u32) -> Instance {
        let mut inst = template.restore();
        inst.bounds = self.bounds;
        inst.uv[..2].copy_from_slice(&self.uv);
        inst.meta[0] = self.kind;
        inst.meta[2] = self.first_stop
            + if self.kind == KIND_REGION {
                0
            } else {
                stop_base
            };
        inst
    }
}

/// `refs` flag: the emission's `(slot, epoch)` pairs are not folded
/// yet — the commit derives them at apply from the stored UVs and the
/// rasterized origins instead of a per-cell slot list the leaf would
/// have to record. The flag is only meaningful for the lowering's own
/// commit, so a stale check on an emission that still carries it
/// always misses (#119).
pub const DEFERRED_REFS: u64 = 1 << 62;

/// Per-operation device output under its sampled placement.
pub struct Emission {
    /// `(instance, pending raster, cell)` patch range in the producing
    /// frame's `Lowering::emission_patches` — `inst_base << 32 | start
    /// << 11 | len`, verbatim frame indices resolved at apply and never
    /// carried past it, where `inst_base` is the first frame instance
    /// slot the leaf occupied so apply and a same-frame recompose can
    /// recover storage-local indices without copying the patch list.
    /// The range indexes `emission_patches`, not `cell_patches`: frame
    /// rollbacks truncate the latter but never the former, so a leaf
    /// produced inside a rolled-back speculation keeps a valid range
    /// (#119).
    pub(crate) pending_cells: u64,
    /// `(start << 32 | len)` range into `EmissionStorage::refs`
    /// holding the `(shelf slot, band epoch)` of every atlas band the
    /// retained instances sample. `DEFERRED_REFS` marks an emission
    /// whose pairs the commit folds at apply (#119).
    pub(crate) refs: u64,
    /// Atlas texture generation and eviction clock at last verification
    /// — one compare on the hit fast path, written at realize time so a
    /// not-yet-applied emission can hit while its commit is in flight
    /// (#119).
    pub(crate) live_stamp: u64,
    template: usize,
    // Only shadows carry an occlusion key; ordinary leaves keep a small index.
    cover: Option<usize>,
    transform: Affine,
    size: [f32; 2],
    pub(crate) instances: Range<usize>,
    stops: Range<usize>,
    image: Option<ImageSource>,
}

impl Emission {
    /// Whether every atlas band the retained UVs reference is still the
    /// band they were baked against. Between commits that evicted
    /// nothing the stored `live_stamp` short-circuits the walk; after
    /// an eviction each referenced band's epoch must still match.
    fn atlas_live(&mut self, atlas: &Atlas, refs: &[(u32, u64)]) -> bool {
        let stamp = atlas.live_stamp();
        if self.live_stamp == stamp {
            return true;
        }
        if refs
            .iter()
            .all(|&(slot, epoch)| atlas.shelf_epoch(slot) == epoch)
        {
            self.live_stamp = stamp;
            return true;
        }
        false
    }

    /// Cold arm of the leaf hit check: the stored stamp differs, so
    /// evictions may have reclaimed a referenced band. Kept out of
    /// line so the hit path never pays for resolving `refs`.
    #[cold]
    #[inline(never)]
    fn stale_live(&mut self, atlas: &Atlas, storage: &EmissionStorage) -> bool {
        if self.refs & DEFERRED_REFS != 0 {
            // Deferred pairs exist only until the producing frame's
            // commit — they can make no epoch claims outside it (#119).
            return false;
        }
        self.atlas_live(atlas, &storage.refs[self.refs_range()])
    }

    /// `start << 32 | len` packing for `refs`; `start` stays under
    /// `2^30`, leaving the top bits for `DEFERRED_REFS`.
    pub(crate) const fn pack_refs(start: usize, len: usize) -> u64 {
        debug_assert!(
            start < (1 << 30),
            "refs start must leave the flag bits free"
        );
        ((start as u64) << 32) | len as u64
    }

    /// The `refs` field decoded back to a usable range.
    pub(crate) const fn refs_range(&self) -> Range<usize> {
        let packed = self.refs & !DEFERRED_REFS;
        (packed >> 32) as usize..(packed >> 32) as usize + (packed & 0xFFFF_FFFF) as usize
    }

    /// Length of the `refs` range, for memory accounting.
    #[allow(dead_code)]
    pub(crate) const fn refs_len(&self) -> usize {
        (self.refs & 0xFFFF_FFFF) as usize
    }

    /// The emission's patch range in the producing frame's
    /// `Lowering::emission_patches`, unpacked from `pending_cells`
    /// (#119).
    pub(crate) const fn pending_cells(&self) -> Range<usize> {
        let start = ((self.pending_cells >> 11) & 0x1F_FFFF) as usize;
        start..start + (self.pending_cells & 0x7FF) as usize
    }

    /// Whether the emission carries no pending cell patches.
    pub(crate) const fn pending_cells_empty(&self) -> bool {
        self.pending_cells.trailing_zeros() >= 11
    }

    /// Frame instance index the pending-cell entries are relative to
    /// (#119).
    pub(crate) const fn cell_inst_base(&self) -> u32 {
        (self.pending_cells >> 32) as u32
    }

    /// Pack `pending_cells` from the producing leaf's cell range and
    /// its frame instance base (#119).
    pub(crate) const fn pack_pending_cells(inst_base: u32, start: usize, len: usize) -> u64 {
        debug_assert!(start < (1 << 21), "cell patch count fits 21 bits");
        debug_assert!(len <= 0x7FF, "cells per leaf fit 11 bits");
        ((inst_base as u64) << 32) | ((start as u64) << 11) | len as u64
    }

    /// Clear the pending-cell extent after the commit applied it (#119).
    pub(crate) const fn clear_pending_cells(&mut self) {
        self.pending_cells = 0;
    }
}

/// GPU resources the lowering needs to emit glyph instances.
pub struct GlyphContext<'a> {
    /// The atlas, read-only here: lookups never mutate, misses become
    /// [`PendingRaster`]s on the `Lowering`, applied serially on the
    /// render thread.
    pub atlas: &'a Atlas,
    /// `atlas.live_stamp()` captured once — the atlas is immutable for
    /// the whole lowering, so emissions compare against one value
    /// instead of repacking generation and clock per check (#119).
    pub live_stamp: u64,
    /// Registered fonts — a per-worker snapshot, so reads and the COLR
    /// cache stay lock-free.
    pub fonts: &'a FxHashMap<u64, FontData>,
    /// Registered images, for dimension lookup during lowering.
    pub images: &'a FxHashMap<u64, GpuImage>,
    /// Decoded bitmap glyph textures.
    pub bitmaps: &'a FxHashMap<super::bitmap::BitmapKey, super::GpuBitmap>,
    /// Producer bindings by layer (`cherenkov::GpuContent`): the quad
    /// each emits samples the producer's current frame.
    pub content: &'a FxHashMap<LayerId, super::gpu_content::Binding>,
}

/// One surface's lowering output: the raster counts plus every deferred
/// atlas insert and COLR cache update, in lowering order, for the render
/// thread to commit before encoding.
#[derive(Default)]
pub struct Lowered {
    /// Source commands resolved this frame.
    pub commands: u32,
    /// Layers with new device realizations.
    pub layers: u32,
    /// Glyphs rasterized during the lowering.
    pub glyphs: u32,
    /// Path rasters during the lowering (cache misses).
    pub paths: u32,
    /// Deferred rasters, in lowering order.
    pub pending: Vec<PendingRaster>,
    /// `uv.xy` patches: `(instance, pending index, cell index)`.
    pub cell_patches: Vec<(u32, u32, u32)>,
    /// The producing emission's view of the same triples, indexed by
    /// `Emission::pending_cells` — survives the frame rollbacks that
    /// truncate `cell_patches` (#119).
    pub emission_patches: Vec<(u32, u32, u32)>,
    /// `uv.zw` patches: `(instance, pending index)`.
    pub mask_patches: Vec<(u32, u32)>,
    /// Local images this frame renders (#84), innermost first.
    pub realize: Vec<super::projective::Realize>,
    /// Local images this frame composes, cached or realized.
    pub composed: Vec<LocalKey>,
}

/// What the renderer knows about one backdrop group this frame.
#[derive(Clone, Copy, Debug)]
pub struct BackdropGroupInfo {
    /// The registered filter chain's key, when the group is filtered.
    pub filter: Option<FilterKey>,
    /// The filter chain's footprint bound; `None` only when the registry
    /// lost the entry (an internal error a sampled group reports).
    pub footprint: Option<filtrate_core::Footprint>,
}

/// The per-region capture overhead in captured pixels: a separated pair
/// merges only while its bounding box wastes fewer pixels than this.
/// Measured on lavapipe (#117): frame GPU time fits
/// `c + a·regions + b·pixels` with `a` indistinguishable from zero
/// (at most ~123 px of work at σ = 8), so the threshold is that bound
/// rounded up to a power of two.
const OVERHEAD_PX: u64 = 128;

/// Groups with more members than this use one union region: the O(n²)
/// clustering pass is bounded, and the union is always a correct answer.
const MAX_CLUSTER_MEMBERS: usize = 64;

/// One cluster's bounding box and the member indices (into the caller's
/// rect list) it contains.
#[derive(Debug, PartialEq, Eq)]
pub struct Cluster {
    /// The cluster's axis-aligned bounding box (`x, y, w, h`).
    pub bbox: [u32; 4],
    /// Indices into the input rect list, in input order.
    pub members: Vec<u32>,
}

fn area(r: [u32; 4]) -> u64 {
    u64::from(r[2]) * u64::from(r[3])
}

fn bbox(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

/// Clusters aproned member rects into capture regions: agglomerative
/// merge on `cost(cluster) = area(bbox) + overhead`, always taking the
/// lowest-waste pair while `waste < overhead` — overlapping or touching
/// rects (waste ≤ 0) always merge. Deterministic: ties break on the
/// lowest first index. `rects` is in member paint order.
fn cluster(rects: &[[u32; 4]], overhead: u64) -> Vec<Cluster> {
    if rects.len() > MAX_CLUSTER_MEMBERS {
        let mut u = rects[0];
        for &r in &rects[1..] {
            u = bbox(u, r);
        }
        return vec![Cluster {
            bbox: u,
            #[expect(
                clippy::cast_possible_truncation,
                reason = "member count is bounded by the caller's layer count"
            )]
            members: (0..rects.len() as u32).collect(),
        }];
    }
    let mut clusters: Vec<Cluster> = rects
        .iter()
        .enumerate()
        .map(|(i, &r)| Cluster {
            bbox: r,
            #[expect(
                clippy::cast_possible_truncation,
                reason = "member count is bounded by the caller's layer count"
            )]
            members: vec![i as u32],
        })
        .collect();
    loop {
        let mut best: Option<(u64, usize, usize)> = None;
        for i in 0..clusters.len() {
            for j in (i + 1)..clusters.len() {
                let u = bbox(clusters[i].bbox, clusters[j].bbox);
                // Signed waste would need care at u64 bounds; saturating
                // keeps `waste < overhead` correct (waste < 0 merges).
                let waste = area(u).saturating_sub(area(clusters[i].bbox) + area(clusters[j].bbox));
                // Overlapping or touching rects merge unconditionally;
                // a bbox area comparison cannot see an L-shaped overlap.
                let [a, b] = [clusters[i].bbox, clusters[j].bbox];
                let touch = a[0] <= b[0] + b[2]
                    && b[0] <= a[0] + a[2]
                    && a[1] <= b[1] + b[3]
                    && b[1] <= a[1] + a[3];
                if touch {
                    if best.is_none() {
                        best = Some((0, i, j));
                    }
                } else if waste < overhead && best.is_none_or(|(w, _, _)| waste < w) {
                    best = Some((waste, i, j));
                }
            }
        }
        let Some((_, i, j)) = best else {
            break;
        };
        let mut merged = clusters.remove(j);
        let a = &mut clusters[i];
        a.bbox = bbox(a.bbox, merged.bbox);
        a.members.append(&mut merged.members);
    }
    clusters
}

/// The plan for one backdrop group: the capture point (its first member
/// in paint order), the clustered capture regions and every member's
/// device bounds for its sampling composite.
struct BackdropPlan {
    /// The first member layer in paint order — its entry emits the capture.
    first: LayerId,
    /// The union of members' clip bounds before the footprint apron.
    union: Rect,
    /// Each member's aproned rect (`A_i`) in paint order — the
    /// clustering input.
    aproned: Vec<(LayerId, Rect)>,
    /// The capture regions in device pixels, one per cluster; the
    /// single-region case is exactly the union rect of the old plan.
    regions: Vec<[u32; 4]>,
    /// Each member layer's device-space clip bounds and region index.
    members: FxHashMap<LayerId, (Rect, u32)>,
}

/// The lowering walk state for one surface frame.
pub struct Lowering<'a> {
    frame: &'a mut Frame,
    width: f32,
    height: f32,
    transform: Affine,
    /// Whether an enclosing layer's transform or scroll track is running;
    /// content then snaps its translation to the ¼-pixel grid.
    animating: bool,
    // The margin depends only on the transform's linear coefficients. Keep
    // their exact bits so signed zero and non-finite inputs retain semantics.
    margin: Option<([u64; 4], f64)>,
    clip: Option<DeviceClip>,
    depth: usize,
    glyphs: u32,
    paths: u32,
    /// `(instance, pending, cell)` triples whose `uv.xy` are set when the
    /// render thread stores the pending path's cells — frame-scoped:
    /// entries name frame instance slots and die with them on rollbacks.
    pub(crate) cell_patches: Vec<(u32, u32, u32)>,
    /// The same triples keyed to the emission that produced them — the
    /// list `Emission::pending_cells` ranges index. Frame rollbacks
    /// truncate `cell_patches` but leave this intact, so a pending
    /// emission produced inside a rolled-back scope can still resolve
    /// its cell origins at apply (#119).
    pub(crate) emission_patches: Vec<(u32, u32, u32)>,
    /// `(instance, pending)` pairs whose `uv.zw` are set when the render
    /// thread stores the pending clip mask.
    pub(crate) mask_patches: Vec<(u32, u32)>,

    /// The open range's bound mask texture key; `None` for the dummy view.
    /// Kept in sync with `clip` by `set_clip`.
    mask_key: Option<u64>,
    /// The current clip's pending mask raster index, if any.
    mask_pending: Option<u32>,
    /// Atlas writes and cache updates to commit, in lowering order.
    pub(crate) pending: Vec<PendingRaster>,
    /// Path identities admitted during this lowering, before atlas commit.
    /// A second use must replay the admitted layout as well as its texels.
    pending_paths: FxHashMap<u64, u32>,
    pub commands_lowered: u32,
    pub layers_composed: u32,
    /// Backdrop groups planned before lowering.
    backdrops: FxHashMap<u64, BackdropPlan>,
    /// The `FilterKey` of each filtered group's capture chain.
    backdrop_filters: FxHashMap<u64, FilterKey>,
    /// Set when a capture ran inside the current isolation: the enclosing
    /// scratch must then cover the full surface so its texel origin is
    /// `(0, 0)` for the capture's composite.
    capture_isolation: bool,
    /// Clip-only scratch depths opened since the last semantic isolation,
    /// outer first.
    clip_scratches: Vec<usize>,
    /// The nearest semantic isolation's target the capture copies from.
    semantic_target: Target,
    /// The storage space of the enclosing level, innermost last; the
    /// surface renders in `Linear` (the implicit base).
    space_stack: Vec<cherenkov::BlendSpace>,
    /// The engine part surface-level passes draw into.
    part: u32,
    /// The layers promoted to system planes this frame: their content is
    /// not drawn.
    promoted: Vec<LayerId>,
    /// The subset of `promoted` a new engine part opens after — the
    /// plan's `opens_part`, so the walk emits exactly `parts()` passes.
    opens: Vec<LayerId>,
    /// The storage space of each scratch target by depth index: a
    /// semantic isolate's declared space, a clip-only level's parent
    /// space, or the opening level's for shadow and capture scopes.
    scratch_space: Vec<cherenkov::BlendSpace>,
    /// Each backdrop capture texture's storage space (the semantic
    /// target it copies), by group id.
    capture_space: FxHashMap<u64, cherenkov::BlendSpace>,
    /// The projective layer whose local image this walk renders, with the
    /// transform replacing its placement (layer space to texels); `None`
    /// when the walk renders the surface from the tree root.
    local: Option<(LayerId, Affine)>,
    /// The target the walk's root draws into.
    root_target: Target,
    /// Projective layers composed into this walk's raster, by layer.
    projected: FxHashMap<LayerId, Placement>,
    /// Projective composites emitted so far: speculative opacity
    /// pass-through never folds across one.
    projective_draws: u32,
    /// The surface size: a local walk lowers onto its image's size, the
    /// surface walk onto this.
    surface: (u32, u32),
}

impl<'a> Lowering<'a> {
    /// Starts a lowering into `frame` for a `width` × `height` surface.
    #[expect(clippy::cast_precision_loss, reason = "surface sizes fit f32")]
    pub fn new(frame: &'a mut Frame, size: (u32, u32)) -> Self {
        Self {
            frame,
            width: size.0 as f32,
            height: size.1 as f32,
            transform: Affine::IDENTITY,
            animating: false,
            margin: None,
            clip: None,
            mask_key: None,
            mask_pending: None,
            depth: 0,
            glyphs: 0,
            paths: 0,
            cell_patches: Vec::new(),
            emission_patches: Vec::new(),
            mask_patches: Vec::new(),
            pending: Vec::new(),
            pending_paths: FxHashMap::default(),
            commands_lowered: 0,
            layers_composed: 0,
            backdrops: FxHashMap::default(),
            backdrop_filters: FxHashMap::default(),
            capture_isolation: false,
            clip_scratches: Vec::new(),
            semantic_target: Target::Part(0),
            part: 0,
            promoted: Vec::new(),
            opens: Vec::new(),
            space_stack: Vec::new(),
            scratch_space: Vec::new(),
            capture_space: FxHashMap::default(),
            local: None,
            root_target: Target::Part(0),
            projected: FxHashMap::default(),
            projective_draws: 0,
            surface: size,
        }
    }

    /// Reuse only the scalar margin, never a command's device realization.
    fn margin(&mut self, transform: Affine) -> f64 {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        let linear = [a, b, c, d].map(f64::to_bits);
        if let Some((cached, value)) = self.margin
            && cached == linear
        {
            return value;
        }
        let value = aa_margin(transform);
        self.margin = Some((linear, value));
        value
    }

    /// Glyphs rasterized during this lowering.
    pub const fn glyphs_rasterized(&self) -> u32 {
        self.glyphs
    }

    /// Paths rasterized during this lowering (cache misses).
    pub const fn paths_rasterized(&self) -> u32 {
        self.paths
    }

    /// Lowers a surface's sampled [`SurfaceTree`] and its clear colour
    /// into the frame. `caches` holds each layer's render-side content.
    /// Plans every backdrop group: paint-order walk collecting each
    /// member's device-space clip bounds, then the capture regions — each
    /// member's aproned rect `A_i` (bounds ∪ reach, inflated by the filter
    /// footprint's apron) integer-rounded and clipped to the surface,
    /// clustered by the `cluster` cost model into one or more regions.
    /// A group that ends up with one region produces exactly the union
    /// rect this planning always made.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "region coordinates are finite, non-negative and below the surface size"
    )]
    fn plan_backdrops(
        &mut self,
        tree: &SurfaceTree,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
    ) -> Result<(), RenderError> {
        self.plan_layer(self.start(tree), tree, groups, Affine::IDENTITY)?;
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        // A member's aproned rect in integer pixels, `None` when it is
        // empty or clipped fully off the surface.
        let aproned = |r: Rect, a: f64| -> Option<[u32; 4]> {
            let x0 = (r.x0 - a).floor().max(0.0);
            let y0 = (r.y0 - a).floor().max(0.0);
            let x1 = (r.x1 + a).ceil().min(w);
            let y1 = (r.y1 + a).ceil().min(h);
            (x1 > x0 && y1 > y0).then_some([
                x0 as u32,
                y0 as u32,
                (x1 - x0) as u32,
                (y1 - y0) as u32,
            ])
        };
        for (gid, plan) in &mut self.backdrops {
            let footprint = groups
                .get(gid)
                .and_then(|info| info.footprint)
                .ok_or_else(|| {
                    RenderError::Render(format!("backdrop group {gid} has no footprint bound"))
                })?;
            if footprint.extent.partial_cmp(&0.5) != Some(std::cmp::Ordering::Less) {
                return Err(RenderError::Unsupported(names::BACKDROP_FOOTPRINT));
            }
            let (uw, uh) = (plan.union.width(), plan.union.height());
            if uw.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
                || uh.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            {
                plan.regions = Vec::new();
                continue;
            }
            // Relative-extent filters make the apron depend on the region
            // size, so per-cluster regions are not guaranteed identical:
            // they stay a single union region (a rule, not an error).
            if footprint.extent > 0.0 {
                let a = (f64::from(footprint.extent)
                    .mul_add(uw.max(uh), f64::from(footprint.pixels))
                    / 2.0f64.mul_add(-f64::from(footprint.extent), 1.0))
                .ceil();
                plan.regions = aproned(plan.union, a).into_iter().collect();
                for (i, _) in &plan.aproned {
                    plan.members.entry(*i).and_modify(|e| e.1 = 0);
                }
                continue;
            }
            let a = (f64::from(footprint.extent).mul_add(uw.max(uh), f64::from(footprint.pixels))
                / 2.0f64.mul_add(-f64::from(footprint.extent), 1.0))
            .ceil();
            // Integer aproned rects per member, in paint order. Members
            // fully outside the surface take no region index.
            let rects: Vec<(LayerId, Option<[u32; 4]>)> = plan
                .aproned
                .iter()
                .map(|(id, r)| (*id, aproned(*r, a)))
                .collect();
            let have: Vec<usize> = (0..rects.len()).filter(|&i| rects[i].1.is_some()).collect();
            let clusters = cluster(
                &have
                    .iter()
                    .map(|&i| rects[i].1.unwrap())
                    .collect::<Vec<_>>(),
                OVERHEAD_PX,
            );
            plan.regions = clusters.iter().map(|c| c.bbox).collect();
            for (r, c) in clusters.iter().enumerate() {
                for &m in &c.members {
                    let id = rects[have[m as usize]].0;
                    plan.members.entry(id).and_modify(|e| e.1 = r as u32);
                }
            }
        }
        Ok(())
    }

    /// One layer of the planning walk, mirroring `layer`'s transform
    /// math: the clip sits in `parent * node.transform` space, children in
    /// `parent * node.content_transform()` space. A projective layer's
    /// members plan in its own local walk.
    fn plan_layer(
        &mut self,
        id: LayerId,
        tree: &SurfaceTree,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        parent: Affine,
    ) -> Result<(), RenderError> {
        if self.projects(id, tree) {
            return Ok(());
        }
        let node = tree.layer(id);
        let (transform, children) = self.placement(id, node, parent);
        if let Some(sample) = &node.backdrop {
            let g = sample.group().raw();
            if !groups.contains_key(&g) {
                return Err(RenderError::Render(format!(
                    "layer {id:?} samples unknown backdrop group {:?}",
                    sample.group()
                )));
            }
            let clip = node
                .clip
                .as_ref()
                .ok_or(RenderError::Unsupported(names::BACKDROP_UNCLIPPED))?;
            let member = clip_device_bounds(transform, clip)?;
            let reach = sample.effect().map(effect_reach).transpose()?;
            // The capture region covers every member's reach; the member's
            // own bounds stay uninflated for its composite and size input.
            let footprint = member.inflate(
                f64::from(reach.unwrap_or(0.0)),
                f64::from(reach.unwrap_or(0.0)),
            );
            let plan = self.backdrops.entry(g).or_insert_with(|| BackdropPlan {
                first: id,
                union: footprint,
                aproned: Vec::new(),
                regions: Vec::new(),
                members: FxHashMap::default(),
            });
            plan.union = plan.union.union(footprint);
            plan.aproned.push((id, footprint));
            plan.members.insert(id, (member, 0));
        }
        for child in &node.children {
            self.plan_layer(*child, tree, groups, children)?;
        }
        Ok(())
    }

    /// Prepares every layer's retained content once for the frame's walks.
    ///
    /// # Errors
    /// A [`RenderError`] for content the lowering cannot prepare.
    pub fn prepare(
        &mut self,
        caches: &mut FxHashMap<LayerId, ContentData>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        for content in caches.values_mut() {
            self.commands_lowered += content.retained.prepare(&mut super::prepared::Lowerer {
                fonts: glyphs.fonts,
                images: glyphs.images,
                pending: &mut self.pending,
            })?;
        }
        Ok(())
    }

    /// Lowers a surface's sampled tree and its clear colour into the
    /// frame, placing the projective layers composed into the surface
    /// from `projected`. Each layer `plan` promotes (paint order) is left
    /// to its system plane: its content is not drawn, and a promoted layer
    /// the plan opens a part after ends the current engine part.
    ///
    /// # Errors
    /// A [`RenderError`] for content or state the lowering cannot render.
    #[expect(clippy::too_many_arguments, reason = "one surface walk's inputs")]
    pub fn run(
        &mut self,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        clear: WorkingColor,
        glyphs: &GlyphContext<'_>,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        projected: FxHashMap<LayerId, Placement>,
        plan: &super::planes::Plan,
    ) -> Result<(), RenderError> {
        let [r, g, b, a] = clear.components;
        self.raster(self.surface);
        self.walk(
            tree,
            caches,
            glyphs,
            groups,
            (None, Target::Part(0), Some([r * a, g * a, b * a, a])),
            projected,
            plan,
        )
    }

    /// Lowers projective layer `layer`'s local image into `target`: its
    /// subtree under `local_to_texel` (layer space to texels) on a raster
    /// of `size` texels cleared to transparent, with its content, clip and
    /// filter but not its opacity or blend, which apply when the image
    /// composes. Nested projective layers compose from `projected`.
    ///
    /// # Errors
    /// A [`RenderError`] for content or state the lowering cannot render.
    #[expect(clippy::too_many_arguments, reason = "one local walk's inputs")]
    pub fn run_local(
        &mut self,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        glyphs: &GlyphContext<'_>,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        (layer, local_to_texel, target): (LayerId, Affine, LocalKey),
        size: (u32, u32),
        projected: FxHashMap<LayerId, Placement>,
    ) -> Result<(), RenderError> {
        self.raster(size);
        self.walk(
            tree,
            caches,
            glyphs,
            groups,
            (
                Some((layer, local_to_texel)),
                Target::Projected(target),
                Some([0.0; 4]),
            ),
            projected,
            // Promotion splits surface-level passes; a local image has none.
            &super::planes::Plan::default(),
        )
    }

    /// Captures a leaf's recorded pixels before its outer properties apply.
    pub fn run_plane(
        &mut self,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        glyphs: &GlyphContext<'_>,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        layer: LayerId,
        domain: super::planes::static_layer::Domain,
    ) -> Result<(), RenderError> {
        self.raster(domain.size);
        self.walk(
            tree,
            caches,
            glyphs,
            groups,
            (
                Some((layer, domain.raster().inverse())),
                Target::Plane(layer),
                Some([0.0; 4]),
            ),
            FxHashMap::default(),
            &super::planes::Plan::default(),
        )
    }

    /// Sets the raster the next walk lowers onto.
    #[expect(clippy::cast_precision_loss, reason = "raster sizes fit f32")]
    const fn raster(&mut self, size: (u32, u32)) {
        self.width = size.0 as f32;
        self.height = size.1 as f32;
    }

    /// One walk from the start layer into `root_target`.
    #[expect(clippy::too_many_arguments, reason = "one walk's inputs")]
    fn walk(
        &mut self,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        glyphs: &GlyphContext<'_>,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        (local, root_target, clear): (Option<(LayerId, Affine)>, Target, Option<[f32; 4]>),
        projected: FxHashMap<LayerId, Placement>,
        plan: &super::planes::Plan,
    ) -> Result<(), RenderError> {
        self.local = local;
        self.root_target = root_target;
        self.semantic_target = root_target;
        self.projected = projected;
        self.promoted = plan.planes.iter().map(|p| p.layer).collect();
        self.opens = plan.opens_part().collect();
        self.part = 0;
        self.transform = Affine::IDENTITY;
        self.animating = false;
        self.set_clip(None);
        self.backdrops.clear();
        self.backdrop_filters = groups
            .iter()
            .filter_map(|(g, info)| info.filter.map(|key| (*g, key)))
            .collect();
        self.plan_backdrops(tree, groups)?;
        self.begin_pass(root_target, clear);
        self.layer(self.start(tree), tree, caches, glyphs)?;
        self.finish_pass();
        Ok(())
    }

    /// The frame lowered so far.
    pub const fn frame(&self) -> &Frame {
        self.frame
    }

    /// Builds `key`'s mip chain after pass `pass`, its realization's last.
    pub fn push_mips(&mut self, pass: usize, key: LocalKey) {
        self.frame.mips.push((pass, key));
    }

    /// The layer the walk starts at.
    fn start(&self, tree: &SurfaceTree) -> LayerId {
        self.local.map_or_else(|| tree.root(), |(id, _)| id)
    }

    /// Whether `id` composes by projection into this walk's raster:
    /// projective, and not the layer whose local image the walk renders.
    fn projects(&self, id: LayerId, tree: &SurfaceTree) -> bool {
        self.local.is_none_or(|(root, _)| root != id) && tree.projective_pose(id).is_some()
    }

    /// `(transform, content transform)` of `node` under `parent`: the
    /// local root renders under its texel transform, never its pose.
    fn placement(
        &self,
        id: LayerId,
        node: &cherenkov::LayerNode,
        parent: Affine,
    ) -> (Affine, Affine) {
        match self.local {
            Some((root, local)) if root == id => (
                local,
                if matches!(self.root_target, Target::Plane(_)) {
                    local
                } else {
                    local * Affine::translate(-node.scroll_offset)
                },
            ),
            _ => (parent * node.transform, parent * node.content_transform()),
        }
    }

    /// Ends the open draw range at the current source boundary.
    fn end_segment(&mut self) {
        self.end_segment_at(self.frame.instances.len());
    }

    #[expect(
        clippy::inline_always,
        reason = "keep current-length calls as cheap as the original unsplit hot path"
    )]
    #[inline(always)]
    fn end_segment_at(&mut self, end: usize) {
        let Some(open) = &mut self.frame.open else {
            return;
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "instance counts fit u32 in practice"
        )]
        let end = end as u32;
        if end > open.seg_start {
            open.ranges.push(DrawRange {
                source: open.source,
                image: open.image.clone(),
                mask: open.mask,
                pipeline: open.pipeline,
                variant: open.variant,
                instances: open.seg_start..end,
            });
            open.seg_start = end;
        }
    }

    /// The storage space of `target`: scratch and capture textures keep
    /// the space recorded when their level or group opened.
    fn target_space(&self, target: Target) -> cherenkov::BlendSpace {
        match target {
            // A local image stores its layer's linear isolation; a part is
            // the surface's own working space.
            Target::Part(_) | Target::Projected(_) | Target::Plane(_) => {
                cherenkov::BlendSpace::Linear
            }
            Target::Scratch(k) => self
                .scratch_space
                .get(k)
                .copied()
                .unwrap_or(cherenkov::BlendSpace::Linear),
            Target::Backdrop { group, .. } => self
                .capture_space
                .get(&group)
                .copied()
                .unwrap_or(cherenkov::BlendSpace::Linear),
        }
    }

    /// The target of the level currently being drawn into: the walk's
    /// current engine part or its projective root, or the innermost
    /// isolation's scratch.
    const fn current_target(&self) -> Target {
        if self.depth == 0 {
            match self.root_target {
                Target::Part(_) => Target::Part(self.part),
                target => target,
            }
        } else {
            Target::Scratch(self.depth - 1)
        }
    }

    /// Ends the current engine part at a promoted layer's paint position and
    /// starts the next one, transparent, for everything painted above it.
    ///
    /// # Panics
    /// When lowering is inside an isolation or a projective local image:
    /// eligibility keeps every promoted layer at the surface level, so this
    /// is an engine defect.
    fn next_part(&mut self) {
        assert!(
            self.depth == 0 && matches!(self.semantic_target, Target::Part(_)),
            "a promoted layer must composite at the surface level"
        );
        self.part += 1;
        self.semantic_target = Target::Part(self.part);
        self.begin_pass(Target::Part(self.part), Some([0.0; 4]));
    }

    /// The storage space of the level currently being drawn into.
    fn current_space(&self) -> cherenkov::BlendSpace {
        self.space_stack
            .last()
            .copied()
            .unwrap_or(cherenkov::BlendSpace::Linear)
    }

    fn begin_pass(&mut self, target: Target, clear: Option<[f32; 4]>) {
        self.finish_pass();
        let space = self.target_space(target);
        self.frame.open = Some(OpenPass {
            target,
            clear,
            space,
            source: None,
            image: None,
            mask: None,
            pipeline: PipelineKind::SrcOver,
            variant: ShaderVariant::Simple,
            backdrop_copy: None,
            capture: None,
            ranges: Vec::new(),
            seg_start: instance_index(self.frame.instances.len()),
        });
        // A masked clip can span passes; the new pass binds its texture.
        self.set_mask(self.mask_key);
    }

    fn finish_pass(&mut self) {
        self.finish_pass_at(self.frame.instances.len());
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "surface size is a small positive float"
    )]
    #[expect(
        clippy::inline_always,
        reason = "keep current-length calls as cheap as the original unsplit hot path"
    )]
    #[inline(always)]
    fn finish_pass_at(&mut self, end: usize) {
        self.end_segment_at(end);
        if let Some(open) = self.frame.open.take() {
            let region = match open.target {
                Target::Part(_) | Target::Projected(_) | Target::Plane(_) => {
                    [0, 0, self.width as u32, self.height as u32]
                }
                // Scratch regions are tightened in `isolate` once the
                // pass's instance bboxes are known.
                Target::Scratch(_) => [0, 0, 0, 0],
                // A capture pass covers its planned region.
                Target::Backdrop {
                    group: g,
                    region: r,
                } => self
                    .backdrops
                    .get(&g)
                    .and_then(|plan| plan.regions.get(r as usize))
                    .copied()
                    .unwrap_or([0, 0, 0, 0]),
            };
            self.frame.passes.push(Pass {
                target: open.target,
                clear: open.clear,
                ranges: open.ranges,
                region,
                space: open.space,
                backdrop_copy: open.backdrop_copy,
                capture: open.capture,
            });
        }
    }

    /// Starts a new draw range when `source` changes.
    fn set_source(&mut self, source: Option<Source>) {
        if self.frame.open.as_ref().is_some_and(|o| o.source != source) {
            self.end_segment();
            if let Some(open) = &mut self.frame.open {
                open.source = source;
            }
        }
    }

    /// Starts a new draw range when the bound image texture changes.
    #[expect(
        clippy::inline_always,
        reason = "keep ordinary image identity changes as cheap as the original Copy path"
    )]
    #[inline(always)]
    fn set_image(&mut self, image: Option<ImageSource>) {
        if self.frame.open.as_ref().is_some_and(|o| o.image != image) {
            self.end_segment();
            if let Some(open) = &mut self.frame.open {
                open.image = image;
            }
        }
    }

    /// Starts a new draw range when the pipeline variant changes.
    fn set_pipeline(&mut self, pipeline: PipelineKind) {
        if self
            .frame
            .open
            .as_ref()
            .is_some_and(|o| o.pipeline != pipeline)
        {
            self.end_segment();
            if let Some(open) = &mut self.frame.open {
                open.pipeline = pipeline;
            }
        }
    }

    /// Sets the current clip and the open range's bound mask texture.
    /// `mask_key`/`mask_pending` derive from the clip, so the per-instance
    /// path only reads scalars.
    fn set_clip(&mut self, clip: Option<DeviceClip>) {
        self.clip = clip;
        let mask = clip.and_then(|c| c.mask);
        self.mask_key = match mask {
            Some(ClipMask::Texture(_, key)) => Some(key),
            _ => None,
        };
        self.mask_pending = match mask {
            Some(ClipMask::Pending(_, p)) => Some(p),
            _ => None,
        };
        self.set_mask(self.mask_key);
    }

    /// Starts a new draw range when the bound mask texture changes.
    #[expect(
        clippy::inline_always,
        reason = "a compare on the hot push_instance path"
    )]
    #[inline(always)]
    fn set_mask(&mut self, mask: Option<u64>) {
        if self.frame.open.as_ref().is_some_and(|o| o.mask != mask) {
            self.end_segment();
            if let Some(open) = &mut self.frame.open {
                open.mask = mask;
            }
        }
    }

    /// Starts a new draw range when the shader variant changes.
    fn set_variant(&mut self, variant: ShaderVariant) {
        if self
            .frame
            .open
            .as_ref()
            .is_some_and(|o| o.variant != variant)
        {
            self.end_segment();
            if let Some(open) = &mut self.frame.open {
                open.variant = variant;
            }
        }
    }

    /// Emits `inst` into the current draw range, segmenting on its
    /// specialised fragment variant. Under a pending clip mask the
    /// instance's `uv.zw` is patched after the mask is stored.
    fn push_instance(&mut self, inst: &Instance) {
        self.set_variant(variant_of(inst));
        let index = instance_index(self.frame.instances.len());
        self.frame.instances.push(*inst);
        if let Some(pending) = self.mask_pending {
            self.mask_patches.push((index, pending));
        }
    }

    /// Renders `body` into an isolated scratch texture, composited back at
    /// `opacity` under the saved outer clip.
    ///
    /// When the isolation exists only for `opacity < 1`, first runs `body`
    /// non-isolated: if it stays in the current pass and its instances'
    /// device bboxes are pairwise disjoint, the instances cannot overlap
    /// and folding `opacity` into each is identical to the isolated
    /// composite. An unclipped overlapping batch can become the scratch pass
    /// directly; nested or clipped batches are rolled back and isolated again.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "surface size is a small positive float; an isolate carries the
        clip, style and pixel-space state of one scope"
    )]
    // Keep isolation's speculative buffers off the ordinary drawing walk's stack.
    #[inline(never)]
    fn isolate(
        &mut self,
        inner_clip: Option<DeviceClip>,
        filter: Option<cherenkov::FilterId>,
        opacity: f32,
        blend: cherenkov::BlendMode,
        space: cherenkov::BlendSpace,
        mut body: impl FnMut(&mut Self, &GlyphContext<'_>) -> Result<(), RenderError>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        if filter.is_none()
            && opacity < 1.0
            && blend == cherenkov::BlendMode::Normal
            && space == self.current_space()
            && self.try_passthrough(opacity, inner_clip.is_none(), &mut body, glyphs)?
        {
            return Ok(());
        }
        self.depth += 1;
        let scratch = self.depth - 1;
        let outer_clip = self.clip;
        self.set_clip(inner_clip);
        // Nested isolations split this scratch's open pass into segments;
        // every segment at this depth needs the region.
        let passes_start = self.frame.passes.len();
        // Backdrop captures inside the body sample the nearest *semantic*
        // isolation's target; clip-only scratches between it and the
        // member compose over the capture. Opacity, blend, space and
        // filter isolations change what the member sees; clip-only ones
        // do not.
        let semantic = filter.is_some()
            || opacity < 1.0
            || blend != cherenkov::BlendMode::Normal
            || space != cherenkov::BlendSpace::Linear;
        // Members composite in the declared space; a clip-only level
        // shares the space it merges back into.
        let storage = if semantic {
            space
        } else {
            self.current_space()
        };
        if self.scratch_space.len() <= scratch {
            self.scratch_space
                .resize(scratch + 1, cherenkov::BlendSpace::Linear);
        }
        self.scratch_space[scratch] = storage;
        self.begin_pass(Target::Scratch(scratch), Some([0.0; 4]));
        let inst_start = self.frame.instances.len();
        let saved_capture = self.capture_isolation;
        let saved_target = self.semantic_target;
        let saved_scratches = std::mem::take(&mut self.clip_scratches);
        self.capture_isolation = false;
        if semantic {
            self.semantic_target = Target::Scratch(scratch);
        } else {
            self.clip_scratches.clone_from(&saved_scratches);
            self.clip_scratches.push(scratch);
        }
        self.space_stack.push(storage);
        body(self, glyphs)?;
        self.space_stack.pop();
        self.finish_pass();
        self.depth -= 1;
        self.set_clip(outer_clip);
        let inner_capture = self.capture_isolation;
        self.capture_isolation = saved_capture || inner_capture;
        self.semantic_target = saved_target;
        self.clip_scratches = saved_scratches;
        let outer_target = self.current_target();
        let region = if let Some(filter) = filter {
            self.frame
                .filters
                .push((self.frame.passes.len() - 1, FilterKey::Layer(filter.raw())));
            [0, 0, self.width as u32, self.height as u32]
        } else if inner_capture {
            // A capture inside reads this scratch at texel origin (0, 0):
            // its region must cover the whole surface.
            [0, 0, self.width as u32, self.height as u32]
        } else if is_destructive(blend) {
            clip_region(
                inner_clip.or(outer_clip),
                self.width as u32,
                self.height as u32,
            )
        } else {
            tight_region(
                &self.frame.instances[inst_start..],
                self.width as u32,
                self.height as u32,
            )
        };
        if region[2] == 0 || region[3] == 0 {
            // A capture inside forces the full-surface region, so this
            // depth is never empty when one ran.
            debug_assert!(!inner_capture);
            // Nothing visible in the scratch: drop this depth's segment
            // passes and the composite entirely.
            for i in (passes_start..self.frame.passes.len()).rev() {
                if self.frame.passes[i].target == Target::Scratch(scratch) {
                    self.frame.remove_pass(i);
                }
            }
            self.begin_pass(outer_target, None);
            return Ok(());
        }
        for pass in &mut self.frame.passes[passes_start..] {
            if pass.target == Target::Scratch(scratch) {
                pass.region = region;
            }
        }
        self.begin_pass(outer_target, None);
        let dst_space = self.current_space();
        if blend != cherenkov::BlendMode::Normal || storage != dst_space {
            // A blended or cross-space composite samples the backdrop
            // explicitly: the target's current contents are copied aside
            // before this pass.
            if let Some(open) = &mut self.frame.open {
                open.backdrop_copy = Some(region);
            }
        }
        // The composite instance: a quad over the scratch's region
        // sampling it with `grad.xy` as the texel origin. A destructive
        // composite carries the effective clip itself: its coverage decides
        // where the operator applies across the whole region.
        if is_destructive(blend) {
            self.set_clip(inner_clip.or(outer_clip));
        }
        #[expect(clippy::cast_precision_loss, reason = "region fits the surface")]
        let origin = [region[0] as f32, region[1] as f32];
        self.emit_composite(
            Source::Scratch(scratch),
            origin,
            opacity,
            region,
            blend,
            storage,
        );
        if is_destructive(blend) {
            self.set_clip(outer_clip);
        }
        Ok(())
    }

    /// Capture a silhouette in padded device coordinates, convolve it, then
    /// apply the outer clip. Padding retains off-surface contributors.
    #[inline(never)]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "finite nonnegative capture extents checked before conversion"
    )]
    fn silhouette_scope(
        &mut self,
        parameters: super::shadow::Parameters,
        mut body: impl FnMut(&mut Self, &GlyphContext<'_>) -> Result<(), RenderError>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        if !parameters.spread.is_finite() {
            return Err(RenderError::Render("non-finite shadow parameters".into()));
        }
        let sigma = parameters.sigma;
        let transform = self.transform * parameters.transform;
        if !transform.is_finite() || !transform.inverse().is_finite() {
            return Err(RenderError::Render("invalid silhouette transform".into()));
        }
        let (px, py) =
            cherenkov::lowering::shadow::capture_padding(transform, sigma, parameters.spread);
        let width = 2.0f64.mul_add(px, f64::from(self.width));
        let height = 2.0f64.mul_add(py, f64::from(self.height));
        if !width.is_finite()
            || !height.is_finite()
            || width > f64::from(u32::MAX)
            || height > f64::from(u32::MAX)
        {
            return Err(RenderError::Render(
                "shadow capture exceeds addressable extent".into(),
            ));
        }
        let saved = (self.transform, self.width, self.height, self.clip);
        self.finish_pass();
        self.depth += 1;
        let scratch = self.depth - 1;
        self.transform = Affine::translate((px, py)) * self.transform;
        self.width = width as f32;
        self.height = height as f32;
        self.clip = None;
        // The silhouette stores the enclosing level's space — it is
        // member content, not an isolation.
        if self.scratch_space.len() <= scratch {
            self.scratch_space
                .resize(scratch + 1, cherenkov::BlendSpace::Linear);
        }
        self.scratch_space[scratch] = self.current_space();
        self.begin_pass(Target::Scratch(scratch), Some([0.0; 4]));
        body(self, glyphs)?;
        self.finish_pass();
        let pass = self.frame.passes.len() - 1;
        self.frame.passes[pass].region = [0, 0, width as u32, height as u32];
        self.frame.shadows.push((
            pass,
            super::shadow::Parameters {
                transform,
                ..parameters
            },
        ));
        (self.transform, self.width, self.height, self.clip) = saved;
        self.depth -= 1;
        self.begin_pass(self.current_target(), None);
        let mut instance = self.base(KIND_SPAN, affine(Affine::IDENTITY));
        instance.bounds = [0.0, 0.0, self.width, self.height];
        instance.meta[1] = PAINT_TEXTURE;
        instance.grad[0] = -(px as f32);
        instance.grad[1] = -(py as f32);
        if self.scratch_space[scratch] == cherenkov::BlendSpace::SrgbEncoded {
            instance.meta[3] |= FLAG_TEX_SRGB << 24;
        }
        self.set_source(Some(Source::Scratch(scratch)));
        self.push_instance(&instance);
        self.set_source(None);
        Ok(())
    }

    /// Speculative pass-through for [`Lowering::isolate`]. Returns `Ok(true)`
    /// when `body` stayed in one pass (folding opacity for disjoint bounds,
    /// or promoting an unclipped overlapping batch into a scratch pass).
    /// Otherwise rolls back and returns `Ok(false)` for nested/clipped isolation.
    fn try_passthrough(
        &mut self,
        opacity: f32,
        inner_unclipped: bool,
        body: &mut impl FnMut(&mut Self, &GlyphContext<'_>) -> Result<(), RenderError>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<bool, RenderError> {
        let snap = self.frame.snapshot();
        let patches = (self.cell_patches.len(), self.mask_patches.len());
        let depth = self.depth;
        let clip = self.clip;
        let transform = self.transform;
        let capture = self.capture_isolation;
        let draws = self.projective_draws;
        let result = body(self, glyphs);
        // A projective composite never takes the pass-through: the layer
        // above it isolates for real.
        let folds = result.is_ok() && self.projective_draws == draws;
        let new = &self.frame.instances[snap.instances..];
        if folds && self.frame.passes.len() == snap.passes && bboxes_disjoint(new) {
            for inst in &mut self.frame.instances[snap.instances..] {
                inst.params[1] *= opacity;
            }
            return Ok(true);
        }
        if folds
            && inner_unclipped
            && clip.is_none()
            && self.frame.passes.len() == snap.passes
            && self.frame.open.is_some()
        {
            self.promote_isolation(snap, opacity);
            return Ok(true);
        }
        self.frame.restore(snap);
        self.cell_patches.truncate(patches.0);
        self.mask_patches.truncate(patches.1);
        self.depth = depth;
        self.set_clip(clip);
        self.transform = transform;
        self.capture_isolation = capture;
        self.projective_draws = draws;
        result?;
        Ok(false)
    }

    /// Reuse an overlapping speculative batch as the isolated pass. The body
    /// stayed in one pass and used the same (empty) inner and outer clip, so
    /// its instances, stops and pending atlas patches already are final.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "surface dimensions and instance indices fit u32"
    )]
    fn promote_isolation(&mut self, snap: FrameSnapshot, opacity: f32) {
        self.end_segment();
        let mut open = self
            .frame
            .open
            .take()
            .expect("speculation has an open pass");
        let first = snap.instances as u32;
        open.ranges.retain_mut(|range| {
            range.instances.start = range.instances.start.max(first);
            range.instances.start < range.instances.end
        });
        let outer_target = open.target;
        self.frame.open = snap.open;
        self.finish_pass_at(snap.instances);
        let region = tight_region(
            &self.frame.instances[snap.instances..],
            self.width as u32,
            self.height as u32,
        );
        if region[2] != 0 && region[3] != 0 {
            // The promoted pass stores the parent's space: the batch's
            // members drew as if into the enclosing level.
            if self.scratch_space.len() <= self.depth {
                self.scratch_space
                    .resize(self.depth + 1, cherenkov::BlendSpace::Linear);
            }
            self.scratch_space[self.depth] = open.space;
            self.frame.passes.push(Pass {
                target: Target::Scratch(self.depth),
                clear: Some([0.0; 4]),
                ranges: open.ranges,
                region,
                space: open.space,
                backdrop_copy: None,
                capture: None,
            });
        }
        self.begin_pass(outer_target, None);
        if region[2] != 0 && region[3] != 0 {
            #[expect(clippy::cast_precision_loss, reason = "region fits the surface")]
            let origin = [region[0] as f32, region[1] as f32];
            self.emit_composite(
                Source::Scratch(self.depth),
                origin,
                opacity,
                region,
                cherenkov::BlendMode::Normal,
                open.space,
            );
        }
    }

    /// Emits the group's capture passes at the first member's paint-order
    /// position — one per region: `region` is copied from the semantic
    /// target, then every clip-only scratch opened since it composes over
    /// that copy.
    fn emit_capture(&mut self, gid: u64) {
        let regions = self.backdrops[&gid].regions.clone();
        let copy_from = self.semantic_target;
        let current = self.current_target();
        // The capture texture stores the semantic target's space: the
        // copies and the clip-only composites over it all stay in it.
        self.capture_space.insert(gid, self.target_space(copy_from));
        for (r, region) in regions.iter().enumerate() {
            #[expect(clippy::cast_possible_truncation, reason = "regions fit u32")]
            let r = r as u32;
            self.begin_pass(
                Target::Backdrop {
                    group: gid,
                    region: r,
                },
                None,
            );
            if let Some(open) = &mut self.frame.open {
                open.capture = Some(Capture {
                    group: gid,
                    region: r,
                    copy_from,
                });
            }
            let scratches = std::mem::take(&mut self.clip_scratches);
            for &k in &scratches {
                // Clip-only scratches cover the full surface (see
                // `isolate`), so their texel origin is (0, 0).
                self.emit_composite(
                    Source::Scratch(k),
                    [0.0, 0.0],
                    1.0,
                    *region,
                    cherenkov::BlendMode::Normal,
                    self.target_space(copy_from),
                );
            }
            self.clip_scratches = scratches;
            self.finish_pass();
            let pass = self.frame.passes.len() - 1;
            if let Some(key) = self.backdrop_filters.get(&gid) {
                self.frame.filters.push((pass, *key));
            }
        }
        self.begin_pass(current, None);
        self.capture_isolation = true;
    }

    /// Emits a member's composite of the shared capture as the
    /// bottom-most draw inside its clip, covering `member ∩ region`.
    /// Members without an effect keep the plain `PAINT_TEXTURE` sample;
    /// an effect turns the instance into `PAINT_BACKDROP` with its kind
    /// and parameter stops packed in `meta[3]`'s low bits.
    #[expect(clippy::cast_precision_loss, reason = "region fits the surface")]
    fn emit_backdrop_sample(
        &mut self,
        gid: u64,
        member: LayerId,
        effect: Option<&cherenkov::BackdropEffect>,
    ) -> Result<(), RenderError> {
        let Some(plan) = self.backdrops.get(&gid) else {
            return Ok(());
        };
        let Some(&(member_bounds, r)) = plan.members.get(&member) else {
            return Ok(());
        };
        let Some(&region) = plan.regions.get(r as usize) else {
            return Ok(());
        };
        if region[2] == 0 || region[3] == 0 {
            return Ok(());
        }
        let (rx, ry, rw, rh) = (
            region[0] as f32,
            region[1] as f32,
            region[2] as f32,
            region[3] as f32,
        );
        let bounds = member_bounds.intersect(Rect::new(
            f64::from(region[0]),
            f64::from(region[1]),
            f64::from(region[0] + region[2]),
            f64::from(region[1] + region[3]),
        ));
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return Ok(());
        }
        let mut inst = self.base(KIND_SPAN, affine(Affine::IDENTITY));
        inst.bounds = [
            f32_f64(bounds.x0),
            f32_f64(bounds.y0),
            f32_f64(bounds.x1),
            f32_f64(bounds.y1),
        ];
        inst.meta[1] = PAINT_TEXTURE;
        inst.params[1] = 1.0;
        // `grad.xy` carries the capture region's texel origin.
        inst.grad[0] = rx;
        inst.grad[1] = ry;
        let mut pipeline = PipelineKind::SrcOver;
        if let Some(effect) = effect {
            // Refraction and shader effects evaluate the member clip's
            // SDF; a path/mask clip has no analytic shape to read.
            if !matches!(effect, cherenkov::BackdropEffect::Color(_))
                && self.clip.is_none_or(|c| c.mask.is_some())
            {
                return Err(RenderError::Unsupported(names::BACKDROP_EFFECT_SDF_PATH));
            }
            #[expect(clippy::cast_possible_truncation, reason = "stop counts fit u32")]
            let first = self.frame.stops.len() as u32;
            let (kind, count) = push_effect_stops(&mut self.frame.stops, effect);
            inst.meta[1] = super::instance::PAINT_BACKDROP;
            inst.meta[2] = first;
            inst.meta[3] |= kind | (count << 8);
            // `grad2.zw` is the member's device size for effect shaders:
            // the unclipped bounds, not the visible intersection.
            inst.grad2 = [
                rw,
                rh,
                f32_f64(member_bounds.width()),
                f32_f64(member_bounds.height()),
            ];
            if let cherenkov::BackdropEffect::Shader(s) = effect {
                pipeline = PipelineKind::Effect(s.shader.raw());
            }
        }
        if self.capture_space.get(&gid).copied() == Some(cherenkov::BlendSpace::SrgbEncoded) {
            inst.meta[3] |= FLAG_TEX_SRGB << 24;
        }
        self.set_source(Some(Source::Backdrop {
            group: gid,
            region: r,
        }));
        self.set_pipeline(pipeline);
        self.push_instance(&inst);
        self.set_pipeline(PipelineKind::SrcOver);
        self.set_source(None);
        Ok(())
    }

    /// Emits the composite quad sampling `source` at texel origin
    /// `origin` onto the current target, covering `region`
    /// (`x, y, w, h` device pixels). `src_space` is the space the source
    /// texture stores; the composite blends in it, so a source stored
    /// unlike the current target forces the explicit-composite pipeline
    /// even under `Normal`.
    fn emit_composite(
        &mut self,
        source: Source,
        origin: [f32; 2],
        opacity: f32,
        region: [u32; 4],
        blend: cherenkov::BlendMode,
        src_space: cherenkov::BlendSpace,
    ) {
        #[expect(clippy::cast_precision_loss, reason = "region fits the surface")]
        let (rx, ry, rw, rh) = (
            region[0] as f32,
            region[1] as f32,
            region[2] as f32,
            region[3] as f32,
        );
        // `KIND_SPAN` coverage is exactly 1: the region edge is the
        // content's edge, not a shape boundary the SDF would antialias
        // into a half-covered rim — wrong under a non-Normal blend.
        // Bounds are device-space for spans and the texture paint reads
        // `pixel`, so the affine is irrelevant.
        let mut inst = self.base(KIND_SPAN, affine(Affine::IDENTITY));
        inst.bounds = [rx, ry, rx + rw, ry + rh];
        inst.meta[1] = PAINT_TEXTURE;
        inst.params[1] = opacity;
        // `grad.xy` carries the sampled texture's texel origin.
        inst.grad[0] = origin[0];
        inst.grad[1] = origin[1];
        if src_space == cherenkov::BlendSpace::SrgbEncoded {
            inst.meta[3] |= FLAG_TEX_SRGB << 24;
        }
        // The composite blends in the source level's space; a mismatch
        // with the pass's storage needs the explicit-composite pipeline
        // (the destination converts in and the result back).
        let dst_space = self
            .frame
            .open
            .as_ref()
            .map_or_else(|| self.current_space(), |open| open.space);
        let cross = src_space != dst_space;
        if cross {
            inst.meta[3] |= FLAG_BLEND_SRC << 24;
        }
        let code = blend_code(blend);
        if code != 0 || cross {
            inst.meta[3] |= code << 16;
            self.set_pipeline(PipelineKind::Replace);
        }
        self.set_source(Some(source));
        self.push_instance(&inst);
        self.set_source(None);
        if code != 0 || cross {
            self.set_pipeline(PipelineKind::SrcOver);
        }
    }

    /// An instance of `kind` under the current transform and clip.
    const fn base(&self, kind: u32, local_to_device: [f32; 8]) -> Instance {
        let mut inst = Instance::new(kind);
        inst.affine = local_to_device;
        Self::apply_clip(&mut inst, self.clip);
        inst
    }

    /// Clip state belongs to composition; coverage and glyph atlas UVs remain retained.
    const fn apply_clip(inst: &mut Instance, clip: Option<DeviceClip>) {
        if let Some(clip) = clip {
            inst.clip_inv = affine(clip.inv);
            inst.clip = clip.shape;
            inst.meta[3] |= FLAG_HAS_CLIP << 24;
            if let Some(mask) = clip.mask {
                let cell = mask.cell();
                // A masked clip's shape is always a sharp rect, so
                // `aspect`/`exponent` — never read by its SDF — carry the
                // mask cell size for the shader's out-of-cell guard.
                inst.params[2] = cell.device[0];
                inst.params[3] = cell.device[1];
                inst.clip.aspect = cell.size[0];
                inst.clip.exponent = cell.size[1];
                inst.meta[3] |= FLAG_HAS_MASK << 24;
                match mask {
                    ClipMask::Cell(cell) => {
                        inst.uv[2] = cell.atlas[0];
                        inst.uv[3] = cell.atlas[1];
                    }
                    ClipMask::Texture(..) => {
                        inst.meta[3] |= FLAG_MASK_TEXTURE << 24;
                    }
                    ClipMask::Pending(..) => {}
                }
            }
        }
    }

    /// Applies `clip` around `body`, merging axis-aligned rects and
    /// isolating for nested non-rect clips.
    fn with_clip(
        &mut self,
        shape: Option<&ShapeData>,
        mut body: impl FnMut(&mut Self, &GlyphContext<'_>) -> Result<(), RenderError>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let Some(shape_data) = shape else {
            return body(self, glyphs);
        };
        if let ShapeData::Path { elements, rule } = shape_data {
            return self.with_path_clip(elements, *rule, body, glyphs);
        }
        let Some(boxed) = box_shape(shape_data)? else {
            return Ok(());
        };
        let inv = (self.transform * boxed.extra).inverse();
        let aligned_rect = match shape_data {
            ShapeData::Rect(r) if axis_aligned(self.transform) => {
                Some(device_rect(self.transform, *r))
            }
            _ => None,
        };
        let clip = DeviceClip {
            inv,
            shape: boxed.shape,
            aligned_rect,
            mask: None,
        };
        self.run_clipped(clip, body, glyphs)
    }

    /// Runs `body` under `clip`, merging it with the current clip when the
    /// combination stays analytic (or singly masked), isolating otherwise.
    fn run_clipped(
        &mut self,
        clip: DeviceClip,
        mut body: impl FnMut(&mut Self, &GlyphContext<'_>) -> Result<(), RenderError>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        /// The merged axis-aligned rect clip for `cr ∩ dr`.
        fn merged_rect(cr: Rect, dr: Rect, mask: Option<ClipMask>) -> DeviceClip {
            let merged = cr.intersect(dr);
            let size = (merged.width().max(0.0), merged.height().max(0.0));
            let center = merged.center();
            let half = [
                f32_f64(size.0 / 2.0).max(0.0),
                f32_f64(size.1 / 2.0).max(0.0),
            ];
            DeviceClip {
                inv: Affine::translate(Vec2::new(-center.x, -center.y)),
                shape: Shape::rect(half),
                aligned_rect: Some(if size.0 <= 0.0 || size.1 <= 0.0 {
                    Rect::new(center.x, center.y, center.x, center.y)
                } else {
                    merged
                }),
                mask,
            }
        }
        match self.clip {
            None => {
                self.set_clip(Some(clip));
                body(self, glyphs)?;
                self.set_clip(None);
                Ok(())
            }
            // The current clip is masked: only an aligned rect merges
            // (keeping the mask); anything else isolates.
            Some(cur) if cur.mask.is_some() => {
                if clip.mask.is_none()
                    && let (Some(cr), Some(dr)) = (cur.aligned_rect, clip.aligned_rect)
                {
                    self.set_clip(Some(merged_rect(cr, dr, cur.mask)));
                    body(self, glyphs)?;
                    self.set_clip(Some(cur));
                    return Ok(());
                }
                self.isolate(
                    Some(clip),
                    None,
                    1.0,
                    cherenkov::BlendMode::Normal,
                    cherenkov::BlendSpace::Linear,
                    body,
                    glyphs,
                )
            }
            Some(cur) => match (clip.mask, cur.aligned_rect, clip.aligned_rect) {
                // A new masked clip merges with an aligned rect clip (or
                // attaches to the current clip's analytic shape).
                (Some(mask), Some(cr), Some(dr)) => {
                    self.set_clip(Some(merged_rect(cr, dr, Some(mask))));
                    body(self, glyphs)?;
                    self.set_clip(Some(cur));
                    Ok(())
                }
                (Some(mask), _, _) => {
                    self.set_clip(Some(DeviceClip {
                        inv: cur.inv,
                        shape: cur.shape,
                        aligned_rect: cur.aligned_rect,
                        mask: Some(mask),
                    }));
                    body(self, glyphs)?;
                    self.set_clip(Some(cur));
                    Ok(())
                }
                (None, Some(cr), Some(dr)) => {
                    self.set_clip(Some(merged_rect(cr, dr, None)));
                    body(self, glyphs)?;
                    self.set_clip(Some(cur));
                    Ok(())
                }
                _ => self.isolate(
                    Some(clip),
                    None,
                    1.0,
                    cherenkov::BlendMode::Normal,
                    cherenkov::BlendSpace::Linear,
                    body,
                    glyphs,
                ),
            },
        }
    }

    /// A layer: push its transform, then clip, then isolate for opacity
    /// and blend, then content followed by children. The clip applies in
    /// `transform` space; content and children draw in
    /// `content_transform` space, which is where `scroll_offset` bites.
    fn layer(
        &mut self,
        id: LayerId,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let node = tree.layer(id);
        if self.projects(id, tree) {
            return self.projected_layer(id, node, glyphs);
        }
        let local_root = self.local.is_some_and(|(root, _)| root == id);
        let clip = if local_root && matches!(self.root_target, Target::Plane(_)) {
            None
        } else {
            node.clip.as_ref()
        };
        // A local root's opacity and blend apply when its image composes.
        let (opacity, blend) = if local_root {
            (1.0, cherenkov::BlendMode::Normal)
        } else {
            (node.opacity, node.blend)
        };
        let backdrop = node.backdrop.clone();
        let saved = self.transform;
        let saved_animating = self.animating;
        if !local_root {
            self.animating |= node.animating();
        }
        let (transform, mut content_space) = self.placement(id, node, saved);
        self.transform = transform;
        if self.animating {
            self.transform = cherenkov::snap_animating(self.transform);
            content_space = cherenkov::snap_animating(content_space);
        }
        let isolates = node.filter.is_some()
            || opacity < 1.0
            || blend != cherenkov::BlendMode::Normal
            // The root already renders into the surface target, and a
            // local root into its image.
            || (id != self.start(tree) && node.blends_within());
        if let Some(sample) = &backdrop {
            let gid = sample.group().raw();
            let plan = &self.backdrops[&gid];
            if plan.first == id && !plan.regions.is_empty() {
                self.emit_capture(gid);
            }
        }
        let result = if isolates && node.filter.is_none() && !is_destructive(blend) {
            // The member's sample draws to the current target under the
            // member clip, before and outside the layer's own isolation,
            // unaffected by the layer's opacity or blend.
            if let Some(sample) = &backdrop {
                self.with_clip(
                    clip,
                    |s, _glyphs| {
                        s.transform = content_space;
                        s.emit_backdrop_sample(sample.group().raw(), id, sample.effect())
                    },
                    glyphs,
                )?;
            }
            // The layer clip applies inside the scratch only; the composite
            // runs under the clip in force outside the layer.
            self.isolate(
                None,
                None,
                opacity,
                blend,
                // Layers declare no space; their isolation composites
                // in the enclosing level's linear storage.
                cherenkov::BlendSpace::Linear,
                |s, glyphs| {
                    s.with_clip(
                        clip,
                        |s, glyphs| {
                            s.transform = content_space;
                            s.layer_items(id, node, tree, caches, glyphs)
                        },
                        glyphs,
                    )
                },
                glyphs,
            )
        } else {
            self.with_clip(
                clip,
                |s, glyphs| {
                    s.transform = content_space;
                    if let Some(sample) = &backdrop {
                        s.emit_backdrop_sample(sample.group().raw(), id, sample.effect())?;
                    }
                    if isolates {
                        let inner = s.clip;
                        s.isolate(
                            inner,
                            node.filter,
                            opacity,
                            blend,
                            cherenkov::BlendSpace::Linear,
                            |s, glyphs| s.layer_items(id, node, tree, caches, glyphs),
                            glyphs,
                        )
                    } else {
                        s.layer_items(id, node, tree, caches, glyphs)
                    }
                },
                glyphs,
            )
        };
        self.transform = saved;
        self.animating = saved_animating;
        result
    }

    /// A projective layer composes its completed local image with one
    /// quad: the image already carries the layer's content, clip and
    /// filter, so the sample draws under the ancestor clip only, and the
    /// layer's opacity and blend apply once. A destructive blend keeps its
    /// operator domain: the ancestor clip intersected with the projected
    /// layer clip, never the image's alpha.
    fn projected_layer(
        &mut self,
        id: LayerId,
        node: &cherenkov::LayerNode,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let Some(placement) = self.projected.get(&id).copied() else {
            // No area this frame: behind the viewer, edge-on or off the
            // raster.
            return Ok(());
        };
        self.projective_draws += 1;
        if !is_destructive(node.blend) {
            self.emit_projected(&placement, node.opacity, node.blend);
            return Ok(());
        }
        let clip = node
            .clip
            .as_ref()
            .expect("a planned projective layer has a clip");
        let viewport =
            Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height)).inflate(1.0, 1.0);
        let Some((outline, rule)) = cherenkov::lowering::projective::project_clip(
            &placement.to_parent,
            clip,
            path::FLATTEN / placement.density,
            viewport,
        ) else {
            // A clip without area bounds the operator to nothing.
            return Ok(());
        };
        let saved = std::mem::replace(&mut self.transform, Affine::IDENTITY);
        let result = self.with_path_clip(
            outline.elements(),
            rule,
            |s, _| {
                s.emit_projected(&placement, node.opacity, node.blend);
                Ok(())
            },
            glyphs,
        );
        self.transform = saved;
        result
    }

    /// The projective composite quad over `placement`'s region under the
    /// current clip. A blended composite first opens a pass whose backdrop
    /// copy of the region it reads, and writes its result verbatim.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "regions fit the f32 raster space and stop counts fit u32"
    )]
    fn emit_projected(&mut self, placement: &Placement, opacity: f32, blend: cherenkov::BlendMode) {
        let [x0, y0, x1, y1] = placement.bounds;
        let code = blend_code(blend);
        if code != 0 {
            self.begin_pass(self.current_target(), None);
            if let Some(open) = &mut self.frame.open {
                open.backdrop_copy = Some([x0, y0, x1 - x0, y1 - y0]);
            }
        }
        let mut inst = self.base(KIND_SPAN, affine(Affine::IDENTITY));
        inst.bounds = [x0 as f32, y0 as f32, x1 as f32, y1 as f32];
        inst.meta[1] = PAINT_PROJECTIVE;
        inst.params[1] = opacity;
        // The homography's evaluation origin and the backdrop's texel
        // origin.
        inst.grad[0] = x0 as f32;
        inst.grad[1] = y0 as f32;
        inst.meta[2] = self.frame.stops.len() as u32;
        self.frame.stops.extend(placement.record());
        inst.meta[3] |= code << 16;
        self.set_pipeline(PipelineKind::Projective { replace: code != 0 });
        self.set_source(Some(Source::Projected(placement.key)));
        self.push_instance(&inst);
        self.set_source(None);
        self.set_pipeline(PipelineKind::SrcOver);
    }

    /// Content first, then children — the engine's layer ordering.
    fn layer_items(
        &mut self,
        id: LayerId,
        node: &cherenkov::LayerNode,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let binding = glyphs.content.get(&id);
        if let Some(binding) = binding {
            // The frame's drawn bindings, whether the layer composites
            // in-engine or promotes to a plane: the redraw and
            // wake-gate checks read them from the lowered frame.
            self.frame.content.push((binding.producer(), binding.size));
        }
        if self.promoted.contains(&id) {
            if self.opens.contains(&id) {
                self.next_part();
            }
            for &child in &node.children {
                self.layer(child, tree, caches, glyphs)?;
            }
            return Ok(());
        }
        if let Some(content) = caches.get_mut(&id) {
            let (ops, emissions, source) = content.retained.prepared_source();
            content.storage.compact(emissions);
            let changed = self.ops(
                source,
                ops,
                emissions,
                &mut content.storage,
                0..ops.len(),
                glyphs,
            )?;
            self.layers_composed += u32::from(changed);
        }
        if let Some(binding) = binding {
            // Every binding of the producer samples its current frame:
            // the drawn quad is the binding's own size, the frame is the
            // rendered ring buffer or the submitted planes. The quad is
            // emitted whether or not a frame has landed yet — the frame
            // lands after the lower — and the draw is skipped while the
            // producer has no current frame.
            if binding.size.0 != 0 && binding.size.1 != 0 {
                self.frame.external.push(binding.producer());
                let bounds = Rect::new(
                    0.0,
                    0.0,
                    f64::from(binding.size.0),
                    f64::from(binding.size.1),
                );
                if let Some(boxed) = box_shape(&ShapeData::Rect(bounds))? {
                    let transform = self.transform * boxed.extra;
                    let mut inst = self.base(KIND_FILL, affine(transform));
                    let margin = self.margin(self.transform);
                    let b = boxed.bounds.inflate(margin, margin);
                    inst.bounds = [f32_f64(b.x0), f32_f64(b.y0), f32_f64(b.x1), f32_f64(b.y1)];
                    inst.shape = boxed.shape;
                    // `cell.zw` carries the quad's own size: the fragment
                    // scales the frame's pixel coordinate by
                    // `params.dims / quad` when the binding's size is
                    // not the frame's (#264).
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "a binding size is a pixel extent within the device limit"
                    )]
                    let quad = [binding.size.0 as f32, binding.size.1 as f32, 0.0, 0.0];
                    inst.uv = quad;
                    self.set_image(Some(ImageSource::Content(binding.producer())));
                    self.push_shaped(inst, transform, boxed.bounds, margin);
                    // The frame draw is its own range: following siblings
                    // must not join it, so the bound image returns to none.
                    self.set_image(None);
                }
            }
        }
        for child in &node.children {
            self.layer(*child, tree, caches, glyphs)?;
        }
        Ok(())
    }

    /// Compose retained ops under the sampled layer state. Only invalid leaf
    /// realizations produce new instances or coverage; scopes assemble passes.
    fn ops(
        &mut self,
        source: &cherenkov::DisplayList,
        ops: &[Op],
        emissions: &mut [Realization<Emission>],
        storage: &mut EmissionStorage,
        range: Range<usize>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<bool, RenderError> {
        let mut changed = false;
        let mut i = range.start;
        while i < range.end {
            match &ops[i] {
                Op::BeginClip { local, shape, end } => {
                    let saved = self.transform;
                    self.transform = saved * *local;
                    let body = |s: &mut Self, g: &GlyphContext<'_>| {
                        s.transform = saved;
                        changed |=
                            s.ops(source, ops, emissions, storage, i + 1..*end as usize, g)?;
                        Ok(())
                    };
                    match shape {
                        ClipShape::Empty => {}
                        ClipShape::Boxed { extra, shape, rect } => {
                            let clip = DeviceClip {
                                inv: (self.transform * *extra).inverse(),
                                shape: *shape,
                                aligned_rect: rect
                                    .filter(|_| axis_aligned(self.transform))
                                    .map(|r| device_rect(self.transform, r)),
                                mask: None,
                            };
                            self.run_clipped(clip, body, glyphs)?;
                        }
                        ClipShape::Path { elements, rule, .. } => {
                            self.with_path_clip(elements, *rule, body, glyphs)?;
                        }
                    }
                    self.transform = saved;
                    i = *end as usize;
                }
                Op::BeginIsolate {
                    opacity,
                    blend,
                    space,
                    filter,
                    end,
                } => {
                    self.isolate(
                        None,
                        *filter,
                        *opacity,
                        *blend,
                        *space,
                        |s, g| {
                            changed |=
                                s.ops(source, ops, emissions, storage, i + 1..*end as usize, g)?;
                            Ok(())
                        },
                        glyphs,
                    )?;
                    i = *end as usize;
                }
                Op::BeginShadow { parameters, end } => {
                    self.silhouette_scope(
                        *parameters,
                        |s, g| {
                            changed |=
                                s.ops(source, ops, emissions, storage, i + 1..*end as usize, g)?;
                            Ok(())
                        },
                        glyphs,
                    )?;
                    i = *end as usize;
                }
                Op::End => unreachable!("paired scopes consume their ends"),
                op => {
                    changed |= self.leaf(
                        op,
                        ops.get(i + 1),
                        &mut emissions[i],
                        storage,
                        glyphs,
                        source,
                    )?;
                }
            }
            i += 1;
        }
        Ok(changed)
    }

    /// Retain each leaf's instances and gradient stops independently. A dirty
    /// command drops just its entries; atlas resets invalidate device addresses.
    ///
    /// A miss realizes the leaf straight into this frame, in the form the
    /// retained copy is defined in — unclipped, from an unbound image — and
    /// copies the emitted range into the cache, so each instance is built
    /// once. Unclipped, the realized range already is the composed output;
    /// under a clip the range is rolled back and the retained copy is
    /// composed under it like a hit, because draw ranges segment on the
    /// clipped instances' shader variant.
    fn leaf(
        &mut self,
        op: &Op,
        next: Option<&Op>,
        cache: &mut Realization<Emission>,
        storage: &mut EmissionStorage,
        glyphs: &GlyphContext<'_>,
        source: &cherenkov::DisplayList,
    ) -> Result<bool, RenderError> {
        let cover = self.shadow_cover(op, next);
        let hit = cache.valid
            && cache.data.as_mut().is_some_and(|e| {
                e.cover.map(|index| storage.covers[index]) == cover
                    && e.transform == self.transform
                    && e.size.map(f32::to_bits) == [self.width, self.height].map(f32::to_bits)
                    && (e.live_stamp == glyphs.live_stamp || e.stale_live(glyphs.atlas, storage))
            });
        cache.valid = true;
        if hit {
            self.compose(
                cache.data.as_ref().expect("a hit has data"),
                storage,
                self.clip,
            );
            return Ok(false);
        }
        if self.clip.is_some() {
            self.realize_clipped_leaf(op, cover, cache, storage, glyphs, source)?;
        } else {
            self.realize_leaf(op, cover, cache, storage, glyphs, source)?;
        }
        Ok(true)
    }

    /// Clipped misses need a rollback because clipping can change draw variants.
    /// Keep their large clip/snapshot values off the ordinary miss path.
    fn realize_clipped_leaf(
        &mut self,
        op: &Op,
        cover: Option<Cover>,
        cache: &mut Realization<Emission>,
        storage: &mut EmissionStorage,
        glyphs: &GlyphContext<'_>,
        source: &cherenkov::DisplayList,
    ) -> Result<(), RenderError> {
        self.set_image(None);
        let snapshot = self.frame.snapshot();
        let first_patch = self.cell_patches.len();
        let clip = self.clip;
        self.set_clip(None);
        let result = self.realize_leaf(op, cover, cache, storage, glyphs, source);
        self.set_clip(clip);
        result?;
        self.frame.restore(snapshot);
        self.cell_patches.truncate(first_patch);
        self.compose(
            cache.data.as_ref().expect("realized leaf has data"),
            storage,
            clip,
        );
        Ok(())
    }

    fn realize_leaf(
        &mut self,
        op: &Op,
        cover: Option<Cover>,
        cache: &mut Realization<Emission>,
        storage: &mut EmissionStorage,
        glyphs: &GlyphContext<'_>,
        source: &cherenkov::DisplayList,
    ) -> Result<(), RenderError> {
        debug_assert!(self.clip.is_none());
        self.set_image(None);
        let first_instance = self.frame.instances.len();
        let first_stop = self.frame.stops.len();
        let first_epatch = self.emission_patches.len();
        // `realize` composes path and glyph ops' local transforms into
        // `self.transform`; the leaf's placement is restored with the clip.
        let transform = self.transform;
        let realized = self.realize(op, cover, glyphs, source);
        self.transform = transform;
        realized?;
        let stop_base = u32::try_from(first_stop).expect("stop count fits u32");
        let instance_base = u32::try_from(first_instance).expect("instance count fits u32");
        let retained_instance = storage.instances.len();
        let retained_stop = storage.stops.len();
        let template = storage.templates.len();
        storage.templates.push(InstanceTemplate::new(
            self.frame
                .instances
                .get(first_instance)
                .unwrap_or(&Instance::new(0)),
        ));
        storage
            .instances
            .extend(self.frame.instances[first_instance..].iter().map(|inst| {
                let retained = RetainedInstance {
                    bounds: inst.bounds,
                    uv: [inst.uv[0], inst.uv[1]],
                    kind: inst.meta[0],
                    first_stop: if inst.meta[0] == KIND_REGION {
                        inst.meta[2]
                    } else {
                        inst.meta[2].saturating_sub(stop_base)
                    },
                };
                // Keep this invariant checked as new leaf emitters are added. Stop
                // indices without gradients are unused but preserve their input too.
                let restored = retained.restore(
                    &storage.templates[template],
                    inst.meta[2] - retained.first_stop,
                );
                debug_assert_eq!(bytemuck::bytes_of(&restored), bytemuck::bytes_of(inst));
                retained
            }));
        storage
            .stops
            .extend_from_slice(&self.frame.stops[first_stop..]);
        cache.data = Some(Emission {
            pending_cells: if first_epatch == self.emission_patches.len() {
                0
            } else {
                Emission::pack_pending_cells(
                    instance_base,
                    first_epatch,
                    self.emission_patches.len() - first_epatch,
                )
            },
            // `refs` pairs are derived at apply — deferred, so the
            // leaf check's stamp compare needs nothing extra for a
            // pending emission to hit inside its own frame, and a
            // leftover can never claim liveness in a later one (#119).
            refs: DEFERRED_REFS,
            live_stamp: glyphs.live_stamp,
            template,
            cover: cover.map(|cover| {
                storage.covers.push(cover);
                storage.covers.len() - 1
            }),
            transform,
            size: [self.width, self.height],
            instances: retained_instance..storage.instances.len(),
            stops: retained_stop..storage.stops.len(),
            image: self
                .frame
                .open
                .as_ref()
                .expect("a leaf lowers into an open pass")
                .image
                .clone(),
        });
        Ok(())
    }

    /// Emits a retained leaf's instances and stops into this frame under
    /// `clip`, rebasing its stop and pending-cell indices.
    fn compose(
        &mut self,
        emission: &Emission,
        storage: &EmissionStorage,
        clip: Option<DeviceClip>,
    ) {
        let offset = u32::try_from(self.frame.stops.len()).expect("stop count fits u32");
        self.frame
            .stops
            .extend_from_slice(&storage.stops[emission.stops.clone()]);
        self.set_image(emission.image.clone());
        let instance_base = instance_index(self.frame.instances.len());
        let retained = &storage.instances[emission.instances.clone()];
        if let Some(first) = retained.first() {
            // A realized leaf shares paint and clip fields. Its varying
            // kinds (fill/span/cell) all use the same shader specialization.
            let mut template = first.restore(&storage.templates[emission.template], offset);
            Self::apply_clip(&mut template, clip);
            self.set_variant(variant_of(&template));
            let first = self.frame.instances.len();
            check_instance_bound(first + retained.len());
            self.frame
                .instances
                .resize(first + retained.len(), template);
            for (inst, source) in self.frame.instances[first..].iter_mut().zip(retained) {
                inst.bounds = source.bounds;
                inst.uv[..2].copy_from_slice(&source.uv);
                inst.meta[0] = source.kind;
                inst.meta[2] = source.first_stop
                    + if source.kind == KIND_REGION {
                        0
                    } else {
                        offset
                    };
                debug_assert_eq!(variant_of(inst), variant_of(&template));
            }
            if let Some(pending) = self.mask_pending {
                self.mask_patches.extend(
                    (first..self.frame.instances.len())
                        .map(|i| (u32::try_from(i).expect("instance count fits u32"), pending)),
                );
            }
        }
        // The producing leaf's frame patches may be gone — rollbacks
        // truncate `cell_patches` — so recomposes re-emit them from
        // `emission_patches`, rebased onto the new instance slots.
        if !emission.pending_cells_empty() {
            for i in emission.pending_cells() {
                let (inst, pending, cell) = self.emission_patches[i];
                let inst = inst
                    .wrapping_sub(emission.cell_inst_base())
                    .wrapping_add(instance_base);
                self.cell_patches.push((inst, pending, cell));
            }
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "let leaf emitters eliminate unused paint fields instead of copying the full payload"
    )]
    #[inline(always)]
    fn resolved_paint(&mut self, paint: &ResolvedPaint) -> PaintData {
        let offset = u32::try_from(self.frame.stops.len()).expect("stop count fits u32");
        let data = match paint {
            ResolvedPaint::Shader(_) => unreachable!("shader paint needs draw bounds"),
            ResolvedPaint::Solid(color) => PaintData {
                kind: PAINT_SOLID,
                color: *color,
                first_stop: offset,
                ..PaintData::default()
            },
            ResolvedPaint::Resources(resources) => {
                let (data, stops) = resources.as_ref();
                let mut data = *data;
                data.first_stop += offset;
                self.frame.stops.extend_from_slice(stops);
                data
            }
        };
        self.set_image(data.image.map(ImageSource::Registered));
        data
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded texture extents"
    )]
    #[cold]
    #[inline(never)]
    fn shader_paint(
        &mut self,
        paint: &super::prepared::ResolvedShader,
        bounds: Rect,
        transform: Affine,
    ) -> Result<PaintData, RenderError> {
        let device = transform.transform_rect_bbox(bounds);
        if ![
            device.width(),
            device.height(),
            bounds.width(),
            bounds.height(),
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            return Err(RenderError::Render(
                "shader paint requires finite bounds".into(),
            ));
        }
        let size = (
            device.width().ceil().max(1.0) as u32,
            device.height().ceil().max(1.0) as u32,
        );
        let key = std::sync::Arc::new(super::paint::Key {
            shader: paint.source.shader.raw(),
            uniforms: paint.source.uniforms.iter().map(|v| v.to_bits()).collect(),
            size,
        });
        // Collapsed axes sample their center. Coverage still comes from the
        // ordinary shape/path lowering, including zero-area geometry.
        let x = if bounds.width() == 0.0 {
            0.0
        } else {
            f64::from(size.0) / bounds.width()
        };
        let y = if bounds.height() == 0.0 {
            0.0
        } else {
            f64::from(size.1) / bounds.height()
        };
        let offset_x = if bounds.width() == 0.0 {
            0.5
        } else {
            -bounds.x0 * x
        };
        let offset_y = if bounds.height() == 0.0 {
            0.5
        } else {
            -bounds.y0 * y
        };
        let [xx, yx, xy, yy, tx, ty] =
            (Affine::new([x, 0.0, 0.0, y, offset_x, offset_y]) * paint.sampling).as_coeffs();
        let data = PaintData {
            kind: super::instance::PAINT_IMAGE,
            grad: [f32_f64(xx), f32_f64(yx), f32_f64(xy), f32_f64(yy)],
            grad2: [
                f32_f64(tx),
                f32_f64(ty),
                f32_f64(f64::from(size.0)),
                f32_f64(f64::from(size.1)),
            ],
            packed: super::instance::EXTEND_PAD | (super::instance::EXTEND_PAD << 4) | (1 << 8),
            ..PaintData::default()
        };
        self.set_image(Some(ImageSource::Shader(key)));
        Ok(data)
    }

    #[inline]
    fn shaped_paint(
        &mut self,
        paint: &ResolvedPaint,
        bounds: Rect,
        local: Affine,
        extra_margin: f64,
    ) -> Result<PaintData, RenderError> {
        if let ResolvedPaint::Shader(shader) = paint {
            self.shader_paint(
                shader,
                bounds.inflate(extra_margin, extra_margin),
                self.transform * local,
            )
        } else {
            Ok(self.resolved_paint(paint))
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "preserve dev inlining of common leaf realization as capabilities grow"
    )]
    #[inline(always)]
    #[expect(
        clippy::too_many_lines,
        reason = "the existing realization match handles all retained operation variants"
    )]
    fn realize(
        &mut self,
        op: &Op,
        cover: Option<Cover>,
        glyphs: &GlyphContext<'_>,
        source: &cherenkov::DisplayList,
    ) -> Result<(), RenderError> {
        match op {
            Op::Shaped {
                kind,
                local,
                ambient,
                shape,
                inner,
                bounds,
                extra_margin,
                paint,
                param_x,
                flags,
            } => {
                let margin = extra_margin + self.margin(self.transform * *ambient);
                let b = bounds.inflate(margin, margin);
                if b.width() <= 0.0 || b.height() <= 0.0 {
                    return Ok(());
                }
                let mut inst = self.base(*kind, affine(self.transform * *local));
                inst.bounds = [f32_f64(b.x0), f32_f64(b.y0), f32_f64(b.x1), f32_f64(b.y1)];
                inst.shape = *shape;
                if let Some(inner) = inner {
                    inst.inner = *inner;
                }
                inst.params[0] = *param_x;
                let paint = self.shaped_paint(paint, *bounds, *local, *extra_margin)?;
                inst.color = paint.color;
                inst.grad = paint.grad;
                inst.grad2 = paint.grad2;
                inst.meta[1] = paint.kind;
                inst.meta[2] = paint.first_stop;
                inst.meta[3] |= (paint.packed & 0x00ff_ffff) | (flags << 24);
                self.push_shaped(inst, self.transform * *local, *bounds, margin);
            }
            Op::Shadow {
                local,
                ambient,
                shape,
                bounds,
                sigma_eff,
                color,
            } => {
                let margin = sigma_eff.mul_add(3.0, 1.0) + self.margin(self.transform * *ambient);
                let b = bounds.inflate(margin, margin);
                let mut inst = self.base(KIND_SHADOW, affine(self.transform * *local));
                inst.bounds = [f32_f64(b.x0), f32_f64(b.y0), f32_f64(b.x1), f32_f64(b.y1)];
                inst.shape = *shape;
                inst.params[0] = f32_f64(*sigma_eff);
                inst.color = *color;
                inst.meta[1] = PAINT_SOLID;
                self.push_shadow_quads(&inst, b, cover);
            }
            Op::Path {
                local,
                rule,
                outline,
                paint,
            } => {
                self.transform *= *local;
                match outline {
                    Outline::Fill { elements, content } => self.path(
                        *content,
                        *rule,
                        || BezPath::from_vec(elements.to_vec()),
                        paint,
                        glyphs,
                    )?,
                    Outline::Stroke { shape, stroke } => {
                        self.stroke_path(shape, stroke, *rule, paint, glyphs)?;
                    }
                    Outline::Source { command, content } => match &source.commands()[*command] {
                        cherenkov::Command::Fill {
                            shape: ShapeData::Path { elements, .. },
                            ..
                        } => self.path(
                            content.expect("prepared fill has a hash"),
                            *rule,
                            || BezPath::from_vec(elements.to_vec()),
                            paint,
                            glyphs,
                        )?,
                        cherenkov::Command::Stroke { shape, stroke, .. } => {
                            self.stroke_path(shape, stroke, *rule, paint, glyphs)?;
                        }
                        _ => unreachable!("prepared outline keeps its source kind"),
                    },
                }
            }
            Op::Glyphs { local, run, paint } => {
                self.transform *= *local;
                self.glyph_run(run.get(source), paint, glyphs)?;
            }
            Op::BitmapGlyph {
                local,
                font,
                glyph,
                origin,
                size,
            } => self.bitmap_glyph(*local, *font, *glyph, *origin, *size, glyphs)?,
            _ => unreachable!("scope is composed, never realized as a leaf"),
        }
        Ok(())
    }

    fn stroke_path(
        &mut self,
        shape: &ShapeData,
        stroke: &cherenkov::kurbo::Stroke,
        rule: FillRule,
        paint: &ResolvedPaint,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let tol = path::FLATTEN / path::sigma_max(self.transform).max(1e-12);
        self.path(
            path::hash_stroke(shape, stroke, tol),
            rule,
            || {
                let path = path::shape_path(shape, tol);
                kurbo::stroke(path, stroke, &kurbo::StrokeOpts::default(), tol)
            },
            paint,
            glyphs,
        )
    }

    #[cold]
    #[inline(never)]
    #[expect(
        clippy::too_many_lines,
        reason = "bitmap glyph decoding and image-instance emission form one cold path"
    )]
    fn bitmap_glyph(
        &mut self,
        local: Affine,
        font_id: u64,
        glyph_id: u32,
        origin: [f32; 2],
        size: f32,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let font = glyphs
            .fonts
            .get(&font_id)
            .ok_or_else(|| RenderError::Font(format!("unregistered font {font_id}")))?;
        let bitmap_font = font
            .bitmap
            .as_ref()
            .ok_or_else(|| RenderError::Font("bitmap glyph has no bitmap font".into()))?;
        let transform = self.transform * local;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "bitmap strike metadata uses f32 ppem"
        )]
        let device_ppem = (f64::from(size) * path::sigma_max(transform)) as f32;
        let strike = bitmap_font.select(device_ppem);
        let key = super::bitmap::BitmapKey {
            font: font_id,
            strike,
            glyph: glyph_id,
        };
        let (em, width, height) = if let Some(bitmap) = glyphs.bitmaps.get(&key) {
            (bitmap.em, bitmap.image.width, bitmap.image.height)
        } else if let Some((em, width, height)) = self.pending.iter().find_map(|raster| {
            let PendingRaster::Bitmap {
                key: pending_key,
                em,
                width,
                height,
                ..
            } = raster
            else {
                return None;
            };
            (*pending_key == key).then_some((*em, *width, *height))
        }) {
            (em, width, height)
        } else {
            let Some(decoded) =
                super::bitmap::decode(&font.data, font.index, bitmap_font, strike, glyph_id)?
            else {
                return Ok(());
            };
            let em = decoded.em;
            let (width, height) = (decoded.width, decoded.height);
            let texels = super::image_texels(
                &decoded.rgba,
                cherenkov::ImageColorSpace::Srgb,
                decoded.premultiplied,
            );
            self.pending.push(PendingRaster::Bitmap {
                key,
                em,
                width,
                height,
                texels,
            });
            self.glyphs += 1;
            (em, width, height)
        };
        let rect = Rect::new(
            f64::from(size).mul_add(em.x0, f64::from(origin[0])),
            f64::from(size).mul_add(em.y0, f64::from(origin[1])),
            f64::from(size).mul_add(em.x1, f64::from(origin[0])),
            f64::from(size).mul_add(em.y1, f64::from(origin[1])),
        );
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Ok(());
        }
        let Some(boxed) = box_shape(&ShapeData::Rect(rect))? else {
            return Ok(());
        };
        let to_device = transform * boxed.extra;
        let margin = self.margin(transform);
        let bounds = boxed.bounds.inflate(margin, margin);
        let mut inst = self.base(KIND_FILL, affine(to_device));
        inst.bounds = [
            f32_f64(bounds.x0),
            f32_f64(bounds.y0),
            f32_f64(bounds.x1),
            f32_f64(bounds.y1),
        ];
        inst.shape = boxed.shape;
        inst.meta[1] = PAINT_IMAGE;
        let bitmap_transform = Affine::translate((rect.x0, rect.y0))
            * Affine::scale_non_uniform(
                rect.width() / f64::from(width),
                rect.height() / f64::from(height),
            );
        let [x_scale, y_skew, x_skew, y_scale, x_translate, y_translate] = (boxed.extra.inverse()
            * bitmap_transform)
            .inverse()
            .as_coeffs();
        inst.grad = [
            f32_f64(x_scale),
            f32_f64(y_skew),
            f32_f64(x_skew),
            f32_f64(y_scale),
        ];
        inst.grad2 = [
            f32_f64(x_translate),
            f32_f64(y_translate),
            f32_f64(f64::from(width)),
            f32_f64(f64::from(height)),
        ];
        inst.meta[3] |= super::instance::EXTEND_PAD | (super::instance::EXTEND_PAD << 4) | (1 << 8);
        self.set_image(Some(ImageSource::Bitmap(key)));
        self.push_shaped(inst, to_device, boxed.bounds, margin);
        Ok(())
    }

    /// A following opaque box hides a rectangle inside each pair of its corner rows.
    #[expect(
        clippy::float_cmp,
        reason = "occlusion requires exact opacity and matching axes"
    )]
    fn shadow_cover(&mut self, op: &Op, next: Option<&Op>) -> Option<Cover> {
        let Op::Shadow { local, .. } = op else {
            return None;
        };
        let Some(Op::Shaped {
            kind: KIND_FILL,
            local: fill,
            ambient,
            shape,
            bounds,
            paint,
            flags: 0,
            ..
        }) = next
        else {
            return None;
        };
        if !matches!(paint, ResolvedPaint::Solid(color) if color[3] == 1.0) {
            return None;
        }
        let relative = local.inverse() * *fill;
        let [scale_x, skew_y, skew_x, scale_y, offset_x, offset_y] = relative.as_coeffs();
        if [scale_x, skew_y, skew_x, scale_y] != [1.0, 0.0, 0.0, 1.0] {
            return None;
        }
        let max_r = f64::from(shape.radii.iter().copied().fold(0.0, f32::max));
        let m = self.margin(self.transform * *ambient) + 1.0;
        Some(Cover {
            wide: bounds.inset((-m, -(max_r + m))) + Vec2::new(offset_x, offset_y),
            tall: bounds.inset((-(max_r + m), -m)) + Vec2::new(offset_x, offset_y),
        })
    }

    /// Split a large aligned fill into its full-coverage interior and antialiased border.
    fn push_shaped(&mut self, mut inst: Instance, to_device: Affine, bounds: Rect, margin: f64) {
        let [scale_x, skew_y, skew_x, scale_y, offset_x, offset_y] = to_device.as_coeffs();
        if inst.meta[0] != KIND_FILL
            || skew_y != 0.0
            || skew_x != 0.0
            || scale_x == 0.0
            || scale_y == 0.0
        {
            self.push_instance(&inst);
            return;
        }
        let radius = f64::from(inst.shape.radii.iter().copied().fold(0.0, f32::max));
        let inner = bounds.inset(-(radius + margin + 1.0));
        let device = device_rect(to_device, inner);
        let span = Rect::new(
            device.x0.ceil(),
            device.y0.ceil(),
            device.x1.floor(),
            device.y1.floor(),
        );
        if inner.width() <= 0.0
            || inner.height() <= 0.0
            || device.area() < 4096.0
            || span.width() <= 0.0
            || span.height() <= 0.0
        {
            self.push_instance(&inst);
            return;
        }
        inst.meta[0] = KIND_SPAN;
        inst.bounds = [
            f32_f64(span.x0),
            f32_f64(span.y0),
            f32_f64(span.x1),
            f32_f64(span.y1),
        ];
        self.push_instance(&inst);
        let (x0, x1) = if scale_x >= 0.0 {
            (
                (span.x0 - offset_x) / scale_x,
                (span.x1 - offset_x) / scale_x,
            )
        } else {
            (
                (span.x1 - offset_x) / scale_x,
                (span.x0 - offset_x) / scale_x,
            )
        };
        let (y0, y1) = if scale_y >= 0.0 {
            (
                (span.y0 - offset_y) / scale_y,
                (span.y1 - offset_y) / scale_y,
            )
        } else {
            (
                (span.y1 - offset_y) / scale_y,
                (span.y0 - offset_y) / scale_y,
            )
        };
        inst.meta[0] = KIND_FILL;
        for strip in border_strips(bounds.inflate(margin, margin), Rect::new(x0, y0, x1, y1))
            .into_iter()
            .filter(|r| r.width() > 0.0 && r.height() > 0.0)
        {
            inst.bounds = [
                f32_f64(strip.x0),
                f32_f64(strip.y0),
                f32_f64(strip.x1),
                f32_f64(strip.y1),
            ];
            self.push_instance(&inst);
        }
    }

    /// Pushes `inst` either as one quad or, when the shadow's local
    /// `covered` region hides its interior, as the up-to-eight strips of
    /// `b \ covered`. The interior behind an opaque card is opaque
    /// shadow: coverage there is already saturated, so skipping it
    /// changes no pixels — the strips' bounds only bound rasterization.
    /// An opacity below 1 disables the split: a translucent group would
    /// composite each strip separately.
    #[expect(clippy::float_cmp, reason = "the split is exact only at full opacity")]
    fn push_shadow_quads(&mut self, inst: &Instance, b: Rect, covered: Option<Cover>) {
        let mut inst = *inst;
        let c = covered.filter(|_| inst.params[1] == 1.0);
        match c {
            None => {
                inst.bounds = [f32_f64(b.x0), f32_f64(b.y0), f32_f64(b.x1), f32_f64(b.y1)];
                self.push_instance(&inst);
            }
            Some(c) => {
                for strip in cover_strips(b, c) {
                    inst.bounds = [
                        f32_f64(strip.x0),
                        f32_f64(strip.y0),
                        f32_f64(strip.x1),
                        f32_f64(strip.y1),
                    ];
                    self.push_instance(&inst);
                }
            }
        }
    }

    /// Bounds-dependent shader preparation must not expand ordinary path replay.
    #[cold]
    #[inline(never)]
    fn shader_outline(
        &mut self,
        make: impl FnOnce() -> BezPath,
        paint: &super::prepared::ResolvedShader,
    ) -> Result<(BezPath, PaintData), RenderError> {
        let path = make();
        let data = self.shader_paint(paint, kurbo::Shape::bounding_box(&path), self.transform)?;
        Ok((path, data))
    }

    /// A path or stroked outline: rasterize once per
    /// (content, matrix, subpixel, surface) and replay spans plus atlas
    /// cells. `content` hashes the draw's semantics; `make` builds the
    /// local outline only on a cache miss, so replays cost no `BezPath` or
    /// stroke work. Coverage clipped by the surface is stored under the
    /// offset-specific key: it is only valid at that integer offset.
    fn path(
        &mut self,
        content: u64,
        rule: FillRule,
        make: impl FnOnce() -> BezPath,
        paint: &ResolvedPaint,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "surface sizes fit u32"
        )]
        let surface = (self.width as u32, self.height as u32);
        let pl = path::placement(content, self.transform, surface);
        let mut make = Some(make);
        let (shader_path, shader_data) = if let ResolvedPaint::Shader(shader) = paint {
            let (path, data) = self.shader_outline(make.take().expect("path factory"), shader)?;
            (Some(path), Some(data))
        } else {
            (None, None)
        };
        let hit = glyphs
            .atlas
            .path(pl.key)
            .or_else(|| glyphs.atlas.path(pl.key_exact()));
        if let Some(emit) = hit {
            self.replay(emit, None, pl.offset, paint, shader_data.as_ref());
            return Ok(());
        }
        if let Some(&pending) = self
            .pending_paths
            .get(&pl.key)
            .or_else(|| self.pending_paths.get(&pl.key_exact()))
        {
            let PendingRaster::Path { emit, .. } = &self.pending[pending as usize] else {
                unreachable!("pending path identity addresses a path");
            };
            let emit = emit.clone();
            self.replay(&emit, Some(pending), pl.offset, paint, shader_data.as_ref());
            return Ok(());
        }
        let (stored, pending) = 'stored: {
            let device =
                pl.raster * shader_path.unwrap_or_else(|| make.take().expect("path factory")());
            let (segments, bbox) = path::flatten_segments(&device, path::FLATTEN);
            let Some(coverage) = path::rasterize(
                &segments,
                bbox,
                (f64::from(self.width), f64::from(self.height)),
                rule,
            ) else {
                // Missing the surface at this offset says nothing about
                // other offsets — and an empty emission carries no bands,
                // so it could never be pinned or replayed later either.
                // Store nothing: an exact-key insert would never hit again
                // and could not be evicted for want of a slot (#119).
                break 'stored (PathEmit::default(), None);
            };
            self.paths += 1;
            let (emit, cells) = path::emit(&coverage)?;
            // Emission rects are relative to the placement offset; the
            // stored record keeps that frame.
            let stored = emit.translated(-pl.offset.x, -pl.offset.y);
            let key = if coverage.clipped {
                pl.key_exact()
            } else {
                pl.key
            };
            let pending = u32::try_from(self.pending.len()).expect("pending count fits u32");
            self.pending.push(PendingRaster::Path {
                key,
                emit: stored.clone(),
                cells,
            });
            self.pending_paths.insert(key, pending);
            (stored, Some(pending))
        };
        self.replay(&stored, pending, pl.offset, paint, shader_data.as_ref());
        Ok(())
    }

    /// Replays a cached path emission: `KIND_SPAN` runs and `KIND_GLYPH`
    /// cells at `offset` from their stored rects, painted like glyphs.
    fn replay(
        &mut self,
        emit: &PathEmit,
        pending: Option<u32>,
        offset: Vec2,
        paint: &ResolvedPaint,
        shader_data: Option<&PaintData>,
    ) {
        let paint = shader_data
            .copied()
            .unwrap_or_else(|| self.resolved_paint(paint));
        let mut template = self.base(KIND_SPAN, affine(self.transform));
        template.color = paint.color;
        template.grad = paint.grad;
        template.grad2 = paint.grad2;
        template.meta[1] = paint.kind;
        template.meta[2] = paint.first_stop;
        template.meta[3] |= paint.packed & 0x00ff_ffff;
        self.replay_quads(
            &template,
            emit.spans.iter().map(|rect| (*rect, [0.0; 2], 0)),
            offset,
        );
        template.meta[0] = KIND_GLYPH;
        let first = self.frame.instances.len();
        self.replay_quads(
            &template,
            emit.cells.iter().map(|cell| {
                (
                    cell.rect,
                    [f32::from(cell.x), f32::from(cell.y)],
                    cell.interior,
                )
            }),
            offset,
        );
        if let Some(pending) = pending {
            for i in 0..emit.cells.len() {
                let patch = (
                    u32::try_from(first + i).expect("instance index fits u32"),
                    pending,
                    u32::try_from(i).expect("cell index fits u32"),
                );
                self.emission_patches.push(patch);
                self.cell_patches.push(patch);
            }
        }
    }

    /// All quads in a path group share paint, clip and shader variant. Reserve
    /// and segment once, then write their instances directly into the frame.
    fn replay_quads(
        &mut self,
        template: &Instance,
        quads: impl ExactSizeIterator<Item = ([f32; 4], [f32; 2], u32)>,
        offset: Vec2,
    ) {
        if quads.len() == 0 {
            return;
        }
        self.set_variant(variant_of(template));
        let first = self.frame.instances.len();
        check_instance_bound(first + quads.len());
        self.frame.instances.resize(first + quads.len(), *template);
        for (inst, (rect, uv, interior)) in self.frame.instances[first..].iter_mut().zip(quads) {
            inst.bounds = [
                f32_f64(f64::from(rect[0]) + offset.x),
                f32_f64(f64::from(rect[1]) + offset.y),
                f32_f64(f64::from(rect[2]) + offset.x),
                f32_f64(f64::from(rect[3]) + offset.y),
            ];
            inst.uv[..2].copy_from_slice(&uv);
            if interior != 0 && inst.meta[1] == PAINT_SOLID {
                inst.meta[0] = KIND_REGION;
                inst.meta[2] = interior;
            }
        }
        if let Some(pending) = self.mask_pending {
            self.mask_patches.extend(
                (first..self.frame.instances.len())
                    .map(|i| (u32::try_from(i).expect("instance index fits u32"), pending)),
            );
        }
    }

    /// A clip with a `Path` shape: the coverage rasterized into an atlas
    /// cell — or, when it exceeds `Atlas::MASK_TEXTURE_TEXELS` or the
    /// atlas cap, a dedicated texture — multiplies every instance drawn
    /// under it. The mask is cached like a path draw: replays cost no
    /// rasterization or storage.
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    #[expect(clippy::cast_precision_loss)]
    #[expect(
        clippy::too_many_lines,
        reason = "atlas and texture storage share the lookup tail"
    )]
    fn with_path_clip(
        &mut self,
        elements: &[PathEl],
        rule: FillRule,
        body: impl FnMut(&mut Self, &GlyphContext<'_>) -> Result<(), RenderError>,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let surface = (self.width as u32, self.height as u32);
        let pl = path::placement(path::hash_elements(elements, 2), self.transform, surface);
        let stored = 'stored: {
            if let Some(mask) = glyphs
                .atlas
                .mask(pl.key)
                .or_else(|| glyphs.atlas.mask(pl.key_exact()))
            {
                // The mask is re-read through `apply_clip` each frame;
                // its `uv[2..3]` atlas coordinates let the commit's pins
                // recover the shelf — no slot recorded here (#119).
                break 'stored ClipMask::Cell(*mask);
            }
            if let Some(mask) = glyphs
                .atlas
                .mask_texture(pl.key)
                .or_else(|| glyphs.atlas.mask_texture(pl.key_exact()))
            {
                let key = if glyphs.atlas.mask_texture(pl.key).is_some() {
                    pl.key
                } else {
                    pl.key_exact()
                };
                break 'stored ClipMask::Texture(*mask, key);
            }
            let device = pl.raster * BezPath::from_vec(elements.to_vec());
            let (segments, bbox) = path::flatten_segments(&device, path::FLATTEN);
            let Some(coverage) = path::rasterize(
                &segments,
                bbox,
                (f64::from(self.width), f64::from(self.height)),
                rule,
            ) else {
                // Clipping to nothing at this offset draws nothing.
                return Ok(());
            };
            self.paths += 1;
            let (Ok(w), Ok(h)) = (u32::try_from(coverage.w), u32::try_from(coverage.h)) else {
                return Err(RenderError::Unsupported(names::PATH_CLIP_TOO_LARGE));
            };
            let texels: Vec<u8> = coverage
                .data
                .iter()
                .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            let key = if coverage.clipped {
                pl.key_exact()
            } else {
                pl.key
            };
            if let Some(mask) = glyphs.atlas.mask(key) {
                break 'stored ClipMask::Cell(*mask);
            }
            if let Some(mask) = glyphs.atlas.mask_texture(key) {
                break 'stored ClipMask::Texture(*mask, key);
            }
            let mask = MaskCell {
                device: [
                    f32_f64(coverage.x - pl.offset.x),
                    f32_f64(coverage.y - pl.offset.y),
                ],
                // Filled by `Atlas::store_mask` on the render thread; a
                // texture mask's stays `[0, 0]`.
                atlas: [0.0, 0.0],
                size: [w as f32, h as f32],
                rect: [
                    f32_f64(coverage.x - pl.offset.x),
                    f32_f64(coverage.y - pl.offset.y),
                    f32_f64(coverage.x + f64::from(w) - pl.offset.x),
                    f32_f64(coverage.y + f64::from(h) - pl.offset.y),
                ],
                // Filled by `Atlas::place_mask` on the render thread.
                slot: 0,
            };
            if glyphs.atlas.mask_in_atlas(w, h) {
                let pending = u32::try_from(self.pending.len()).expect("pending count fits u32");
                self.pending.push(PendingRaster::Mask {
                    key,
                    mask,
                    w,
                    h,
                    texels,
                });
                ClipMask::Pending(mask, pending)
            } else {
                if !glyphs.atlas.mask_texture_fits(w, h) {
                    return Err(RenderError::Unsupported(names::PATH_CLIP_TOO_LARGE));
                }
                self.pending.push(PendingRaster::MaskTexture {
                    key,
                    mask,
                    w,
                    h,
                    texels,
                });
                ClipMask::Texture(mask, key)
            }
        };
        // Stored mask rects are relative to the placement offset.
        let stored = stored.translated(pl.offset.x, pl.offset.y);
        let mask = stored.cell();
        let rect = Rect::new(
            f64::from(mask.rect[0]),
            f64::from(mask.rect[1]),
            f64::from(mask.rect[2]),
            f64::from(mask.rect[3]),
        );
        let center = rect.center();
        let clip = DeviceClip {
            inv: Affine::translate(Vec2::new(-center.x, -center.y)),
            shape: Shape::rect([f32_f64(rect.width() / 2.0), f32_f64(rect.height() / 2.0)]),
            aligned_rect: Some(rect),
            mask: Some(stored),
        };
        self.run_clipped(clip, body, glyphs)
    }

    #[cold]
    #[inline(never)]
    fn shader_glyph_run(
        &mut self,
        run: &GlyphRun,
        paint: &super::prepared::ResolvedShader,
        glyphs: &GlyphContext<'_>,
        font: &FontData,
    ) -> Result<(), RenderError> {
        let key = glyph_key(run, 0, (0.0, 0.0), self.transform);
        let mut entries = Vec::new();
        let mut bounds = None::<Rect>;
        for glyph in run.glyphs.iter() {
            let origin = self.transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
            let x = origin.x.floor();
            let y = origin.y.floor();
            let fraction = (f32_f64(origin.x - x), f32_f64(origin.y - y));
            let key = key.at(glyph.id, fraction);
            let (entry, pending) = glyph::entry(
                glyphs.atlas,
                font,
                key,
                glyph.id,
                run.size,
                fraction,
                self.transform,
                &run.coords,
                &mut self.pending,
            )?;
            self.glyphs += u32::from(pending.is_some());
            if entry.w == 0 || entry.h == 0 {
                continue;
            }
            let rect = Rect::from_origin_size(
                (x + f64::from(entry.left), y + f64::from(entry.top)),
                (f64::from(entry.w), f64::from(entry.h)),
            );
            bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
            entries.push((entry, pending, rect));
        }
        let Some(bounds) = bounds else {
            return Ok(());
        };
        let data = self.shader_paint(
            paint,
            self.transform.inverse().transform_rect_bbox(bounds),
            self.transform,
        )?;
        for (entry, pending, rect) in entries {
            let mut inst = self.base(KIND_GLYPH, affine(self.transform));
            inst.grad = data.grad;
            inst.grad2 = data.grad2;
            inst.meta[1] = data.kind;
            inst.meta[3] |= data.packed;
            inst.bounds = [
                f32_f64(rect.x0),
                f32_f64(rect.y0),
                f32_f64(rect.x1),
                f32_f64(rect.y1),
            ];
            inst.uv = [f32::from(entry.x), f32::from(entry.y), 0.0, 0.0];
            self.push_instance(&inst);
            if let Some(pending) = pending {
                let patch = (instance_index(self.frame.instances.len() - 1), pending, 0);
                self.emission_patches.push(patch);
                self.cell_patches.push(patch);
            }
        }
        Ok(())
    }

    /// `Glyphs`: rasterize missing atlas entries and emit one quad per
    /// glyph.
    fn glyph_run(
        &mut self,
        run: &GlyphRun,
        paint: &ResolvedPaint,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        let font = glyphs
            .fonts
            .get(&run.font.raw())
            .ok_or_else(|| RenderError::Font(format!("unregistered font {:?}", run.font)))?;
        if let ResolvedPaint::Shader(shader) = paint {
            return self.shader_glyph_run(run, shader, glyphs, font);
        }
        let key = glyph_key(run, 0, (0.0, 0.0), self.transform);
        let mut template = None;
        for glyph in run.glyphs.iter() {
            let o = self.transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
            let ix = o.x.floor();
            let iy = o.y.floor();
            let fx = o.x - ix;
            let fy = o.y - iy;
            let key = key.at(glyph.id, (f32_f64(fx), f32_f64(fy)));
            let (entry, pending) = glyph::entry(
                glyphs.atlas,
                font,
                key,
                glyph.id,
                run.size,
                (f32_f64(fx), f32_f64(fy)),
                self.transform,
                &run.coords,
                &mut self.pending,
            )?;
            self.glyphs += u32::from(pending.is_some());
            if entry.w == 0 || entry.h == 0 {
                continue;
            }
            let mut inst = *template.get_or_insert_with(|| {
                let mut inst = self.base(KIND_GLYPH, affine(self.transform));
                let data = self.resolved_paint(paint);
                inst.color = data.color;
                inst.grad = data.grad;
                inst.grad2 = data.grad2;
                inst.meta[1] = data.kind;
                inst.meta[2] = data.first_stop;
                inst.meta[3] |= data.packed & 0x00ff_ffff;
                inst
            });
            let x0 = f32_f64(ix + f64::from(entry.left));
            let y0 = f32_f64(iy + f64::from(entry.top));
            inst.bounds = [x0, y0, x0 + f32::from(entry.w), y0 + f32::from(entry.h)];
            inst.uv = [f32::from(entry.x), f32::from(entry.y), 0.0, 0.0];
            self.push_instance(&inst);
            if let Some(pending) = pending {
                let patch = (instance_index(self.frame.instances.len() - 1), pending, 0);
                self.emission_patches.push(patch);
                self.cell_patches.push(patch);
            }
        }
        Ok(())
    }
}

/// Validates a member effect and returns its sampling reach in device
/// pixels — how far beyond the member bounds its samples can land.
/// Invalid parameters fail like other invalid input: an explicit
/// [`RenderError::Render`].
fn effect_reach(effect: &cherenkov::BackdropEffect) -> Result<f32, RenderError> {
    match effect {
        cherenkov::BackdropEffect::Color(matrix) => {
            if matrix.0.iter().all(|v| v.is_finite()) {
                Ok(0.0)
            } else {
                Err(RenderError::Render(
                    "backdrop colour effect has a non-finite matrix entry".into(),
                ))
            }
        }
        cherenkov::BackdropEffect::Refraction(r) => {
            if r.depth.is_finite() && r.depth > 0.0 && r.strength.is_finite() && r.strength >= 0.0 {
                Ok(r.strength)
            } else {
                Err(RenderError::Render(
                    "backdrop refraction needs depth > 0 and strength >= 0, finite".into(),
                ))
            }
        }
        cherenkov::BackdropEffect::Rim(r) => {
            if r.width.is_finite()
                && r.width > 0.0
                && r.gain.is_finite()
                && r.color.iter().all(|v| v.is_finite())
            {
                Ok(0.0)
            } else {
                Err(RenderError::Render(
                    "backdrop rim needs width > 0 and a finite colour and gain".into(),
                ))
            }
        }
        cherenkov::BackdropEffect::Shader(s) => {
            if s.uniforms.len() <= 64
                && s.uniforms.iter().all(|v| v.is_finite())
                && effect.reach().is_finite()
            {
                Ok(effect.reach())
            } else {
                Err(RenderError::Render(
                    "backdrop shader effect needs at most 64 finite uniforms".into(),
                ))
            }
        }
    }
}

/// Pushes a member effect's parameters into `stops` and returns its
/// `(kind, stop count)`: `Color` packs three row stops (the row's
/// `[r, g, b, bias]` in `color`), `Refraction` one stop (`depth`,
/// `strength` in `color.xy`), `Rim` two stops (`(width, r, g, b)` and
/// `(a, gain)`), `Shader` the uniforms packed four per stop, zero-filled.
fn push_effect_stops(stops: &mut Vec<Stop>, effect: &cherenkov::BackdropEffect) -> (u32, u32) {
    use super::instance::{EFFECT_COLOR, EFFECT_REFRACTION, EFFECT_RIM, EFFECT_SHADER};
    let push = |stops: &mut Vec<Stop>, v: [f32; 4]| {
        stops.push(Stop {
            color: v,
            offset: 0.0,
            pad: [0.0; 3],
        });
    };
    match effect {
        cherenkov::BackdropEffect::Color(matrix) => {
            for row in matrix.0.as_chunks::<4>().0 {
                push(stops, *row);
            }
            (EFFECT_COLOR, 3)
        }
        cherenkov::BackdropEffect::Refraction(r) => {
            push(stops, [r.depth, r.strength, 0.0, 0.0]);
            (EFFECT_REFRACTION, 1)
        }
        cherenkov::BackdropEffect::Rim(r) => {
            push(stops, [r.width, r.color[0], r.color[1], r.color[2]]);
            push(stops, [r.color[3], r.gain, 0.0, 0.0]);
            (EFFECT_RIM, 2)
        }
        cherenkov::BackdropEffect::Shader(s) => {
            let mut count = 0;
            for chunk in s.uniforms.chunks(4) {
                let mut v = [0.0; 4];
                v[..chunk.len()].copy_from_slice(chunk);
                push(stops, v);
                count += 1;
            }
            (EFFECT_SHADER, count)
        }
    }
}

/// Whether `transform` keeps axis-aligned rectangles axis-aligned (a
/// scale, translation, or quarter turn): a rect clip under it stays a
/// device-aligned rectangle.
pub(super) fn axis_aligned(transform: Affine) -> bool {
    let [c0, c1, c2, c3, _, _] = transform.as_coeffs();
    (c1 == 0.0 && c2 == 0.0) || (c0 == 0.0 && c3 == 0.0)
}

/// The device-space bounding box of `clip` under `transform` — the shape
/// the merged clip machinery would clip against, any transform.
fn clip_device_bounds(transform: Affine, clip: &ShapeData) -> Result<Rect, RenderError> {
    let (local, extra) = match clip {
        ShapeData::Rect(r) => (*r, Affine::IDENTITY),
        ShapeData::Path { elements, .. } => {
            let path = elements.iter().copied().collect::<BezPath>();
            (kurbo::Shape::bounding_box(&path), Affine::IDENTITY)
        }
        _ => match box_shape(clip)? {
            Some(boxed) => (boxed.bounds, boxed.extra),
            None => return Ok(Rect::ZERO),
        },
    };
    let t = transform * extra;
    let corners = [
        Point::new(local.x0, local.y0),
        Point::new(local.x1, local.y0),
        Point::new(local.x0, local.y1),
        Point::new(local.x1, local.y1),
    ]
    .map(|p| t * p);
    let (mut min, mut max) = (corners[0], corners[0]);
    for p in &corners[1..] {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    Ok(Rect::new(min.x, min.y, max.x, max.y))
}

/// The device-space rectangle of a rect under an axis-aligned transform.
fn device_rect(t: Affine, r: Rect) -> Rect {
    let p0 = t * Point::new(r.x0, r.y0);
    let p1 = t * Point::new(r.x1, r.y1);
    Rect::new(
        p0.x.min(p1.x),
        p0.y.min(p1.y),
        p0.x.max(p1.x),
        p0.y.max(p1.y),
    )
}

/// The globals uniform for one pass: `size` is the target region's pixel
/// size, `origin` its device-space origin and `space` the space its
/// premultiplied pixels are stored in.
pub const fn globals(size: [f32; 2], origin: [f32; 2], space: cherenkov::BlendSpace) -> Globals {
    Globals {
        attachment_origin: origin,
        size,
        origin,
        space: match space {
            cherenkov::BlendSpace::Linear => 0,
            cherenkov::BlendSpace::SrgbEncoded => 1,
        },
        pad: 0,
    }
}

/// An instance's device-space bounds: `bounds` transformed by `affine`,
/// unless the kind is already device space (`KIND_GLYPH`, `KIND_SPAN`).
#[expect(
    clippy::many_single_char_names,
    reason = "a..f are the conventional affine coefficient names"
)]
fn device_bbox(inst: &Instance) -> [f32; 4] {
    if matches!(inst.meta[0], KIND_GLYPH | KIND_SPAN | KIND_REGION) {
        return inst.bounds;
    }
    let [a, b, c, d, e, f, _, _] = inst.affine;
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for (x, y) in [
        (inst.bounds[0], inst.bounds[1]),
        (inst.bounds[2], inst.bounds[1]),
        (inst.bounds[0], inst.bounds[3]),
        (inst.bounds[2], inst.bounds[3]),
    ] {
        let dx = a.mul_add(x, c.mul_add(y, e));
        let dy = b.mul_add(x, d.mul_add(y, f));
        min[0] = min[0].min(dx);
        min[1] = min[1].min(dy);
        max[0] = max[0].max(dx);
        max[1] = max[1].max(dy);
    }
    [min[0], min[1], max[0], max[1]]
}

/// Whether two `[x0, y0, x1, y1]` boxes overlap in area.
fn boxes_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// Whether every instance's device bbox is pairwise non-overlapping —
/// O(n²) up to 256 instances, a sort-by-x sweep beyond.
fn bboxes_disjoint(instances: &[Instance]) -> bool {
    if instances.len() <= 256 {
        let mut boxes = Vec::with_capacity(instances.len());
        for inst in instances {
            let a = device_bbox(inst);
            if boxes.iter().any(|b| boxes_overlap(*b, a)) {
                return false;
            }
            boxes.push(a);
        }
        return true;
    }
    let mut boxes = Vec::with_capacity(instances.len());
    for inst in instances {
        let b = device_bbox(inst);
        // Splitting a draw into border strips can put intersecting boxes a
        // few instances apart. Check a bounded prefix pairwise before sorting;
        // after that, checking neighbors keeps the extra work linear.
        let recent = if boxes.len() < 32 {
            boxes.as_slice()
        } else {
            &boxes[boxes.len() - 1..]
        };
        if recent.iter().any(|a| boxes_overlap(*a, b)) {
            return false;
        }
        boxes.push(b);
    }
    boxes.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut reach = f32::NEG_INFINITY;
    let mut y0 = f32::INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for b in boxes {
        if b[0] >= reach {
            reach = b[2];
            y0 = b[1];
            y1 = b[3];
        } else {
            // x overlaps the running interval: check its y span.
            if b[1] < y1 && b[3] > y0 {
                return false;
            }
            reach = reach.max(b[2]);
            y0 = y0.min(b[1]);
            y1 = y1.max(b[3]);
        }
    }
    true
}

/// The union device bbox of `instances` clipped to the surface, inflated
/// by one pixel and integer-rounded, as `(x, y, w, h)`; `[0, 0, 0, 0]`
/// when empty.
fn tight_region(instances: &[Instance], width: u32, height: u32) -> [u32; 4] {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for b in instances.iter().map(device_bbox) {
        if b.iter().any(|v| !v.is_finite()) {
            continue;
        }
        min[0] = min[0].min(b[0]);
        min[1] = min[1].min(b[1]);
        max[0] = max[0].max(b[2]);
        max[1] = max[1].max(b[3]);
    }
    padded_region(min, max, width, height)
}

/// The `[x, y, w, h]` texel region covering a float bbox: floor−1 / ceil+1,
/// clamped to the surface, `[0, 0, 0, 0]` when empty.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "region coordinates are finite, non-negative and below the surface size"
)]
fn padded_region(min: [f32; 2], max: [f32; 2], width: u32, height: u32) -> [u32; 4] {
    let x0 = (min[0].floor() - 1.0).max(0.0);
    let y0 = (min[1].floor() - 1.0).max(0.0);
    let x1 = (max[0].ceil() + 1.0).min(width as f32);
    let y1 = (max[1].ceil() + 1.0).min(height as f32);
    if x1 <= x0 || y1 <= y0 {
        return [0, 0, 0, 0];
    }
    [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32]
}

/// Porter-Duff operators where a transparent source writes over the
/// destination instead of leaving it unchanged. These composite over the
/// layer's whole clip (or parent), not the tight content region.
const fn is_destructive(blend: cherenkov::BlendMode) -> bool {
    use cherenkov::BlendMode as B;
    matches!(
        blend,
        B::Clear | B::Src | B::SrcIn | B::SrcOut | B::DestIn | B::DestAtop
    )
}

/// The device region a destructive composite covers: the clip's extent,
/// or the whole parent when there is no clip. Without an axis-aligned
/// rect, the conservative bbox is the clip shape's half extents mapped to
/// device space (`inv` maps device to clip-local, so `inv⁻¹` maps back);
/// the clip shape itself still bounds coverage per pixel.
#[expect(
    clippy::cast_possible_truncation,
    reason = "clip extents fit the f32 surface space"
)]
fn clip_region(clip: Option<DeviceClip>, width: u32, height: u32) -> [u32; 4] {
    let Some(clip) = clip else {
        return [0, 0, width, height];
    };
    if let Some(rect) = clip.aligned_rect {
        return padded_region(
            [rect.x0 as f32, rect.y0 as f32],
            [rect.x1 as f32, rect.y1 as f32],
            width,
            height,
        );
    }
    let inv = clip.inv.inverse();
    let (hx, hy) = clip.shape.half.into();
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for (x, y) in [(hx, hy), (-hx, hy), (hx, -hy), (-hx, -hy)] {
        let p = inv * Point::new(f64::from(x), f64::from(y));
        min[0] = min[0].min(p.x as f32);
        min[1] = min[1].min(p.y as f32);
        max[0] = max[0].max(p.x as f32);
        max[1] = max[1].max(p.y as f32);
    }
    padded_region(min, max, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_margin_preserves_exact_transform_results() {
        let mut frame = Frame::default();
        let mut lowering = Lowering::new(&mut frame, (64, 64));
        let transforms = [
            Affine::IDENTITY,
            Affine::scale_non_uniform(2.0, 3.0),
            Affine::rotate(0.7),
            Affine::new([1.0, 0.2, -0.3, 2.0, 4.0, 5.0]),
            Affine::new([-0.0, 0.0, 0.0, -0.0, 0.0, 0.0]),
            Affine::new([f64::MIN_POSITIVE, 0.0, 0.0, 1.0, 0.0, 0.0]),
            Affine::new([f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0]),
            Affine::new([f64::NAN, 0.0, 0.0, f64::NAN, 0.0, 0.0]),
        ];
        for transform in transforms.into_iter().cycle().take(24) {
            let expected = aa_margin(transform).to_bits();
            assert_eq!(lowering.margin(transform).to_bits(), expected);
            assert_eq!(lowering.margin(transform).to_bits(), expected);
            let [a, b, c, d, _, _] = transform.as_coeffs();
            let translated = Affine::new([a, b, c, d, 123.0, -456.0]);
            assert_eq!(lowering.margin(translated).to_bits(), expected);
        }
    }

    #[test]
    fn large_opacity_groups_detect_adjacent_and_distant_overlaps() {
        let mut instances: Vec<_> = (0_u16..300)
            .map(|index| {
                let mut instance = Instance::new(KIND_SPAN);
                let x = f32::from(index) * 2.0;
                instance.bounds = [x, 0.0, x + 1.0, 1.0];
                instance
            })
            .collect();
        assert!(bboxes_disjoint(&instances));
        let last = instances[299].bounds;
        instances[299].bounds = instances[0].bounds;
        assert!(!bboxes_disjoint(&instances));
        instances[299].bounds = last;
        let within_prefix = instances[15].bounds;
        instances[15].bounds = instances[0].bounds;
        assert!(!bboxes_disjoint(&instances));
        instances[15].bounds = within_prefix;
        instances[1].bounds = instances[0].bounds;
        assert!(!bboxes_disjoint(&instances));
    }

    /// An adapter plus device, or `None` where no GPU exists.
    fn device_and_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .next()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn draw(
        lowering: &mut Lowering<'_>,
        command: &cherenkov::Command,
        glyphs: &GlyphContext<'_>,
    ) -> Result<(), RenderError> {
        use cherenkov::lowering::Compiler as _;
        let mut pending = Vec::new();
        let mut compiler = super::super::prepared::Lowerer {
            fonts: glyphs.fonts,
            images: glyphs.images,
            pending: &mut pending,
        };
        let mut ops = Vec::new();
        compiler.draw(command, Affine::IDENTITY, &mut ops)?;
        for op in ops {
            lowering.realize(&op, None, glyphs, &cherenkov::DisplayList::default())?;
        }
        Ok(())
    }

    /// An isolated group's composite quad is a `KIND_SPAN`: full coverage
    /// over its device-space region, not an SDF edge that would half-cover
    /// the rim texels.
    #[test]
    fn a_composite_is_a_full_coverage_span() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let atlas = Atlas::new(&device, u64::MAX);
        let fonts = FxHashMap::default();
        let images = FxHashMap::default();
        let bitmaps = FxHashMap::default();
        let mut frame = Frame::default();
        let mut lowering = Lowering::new(&mut frame, (64, 64));
        lowering.begin_pass(Target::Part(0), None);
        let glyphs = GlyphContext {
            atlas: &atlas,
            live_stamp: atlas.live_stamp(),
            fonts: &fonts,
            images: &images,
            bitmaps: &bitmaps,
            content: &FxHashMap::default(),
        };
        let prefix = cherenkov::Command::Fill {
            shape: ShapeData::Rect(Rect::new(0.0, 0.0, 2.0, 2.0)),
            paint: cherenkov::Paint::Solid(WorkingColor::new([0.0, 1.0, 0.0, 1.0])),
        };
        draw(&mut lowering, &prefix, &glyphs).expect("prefix");
        let mut calls = 0;
        // Two overlapping rects defeat the pass-through speculation, so
        // the group really isolates into a scratch and composites back.
        lowering
            .isolate(
                None,
                None,
                0.5,
                cherenkov::BlendMode::Normal,
                cherenkov::BlendSpace::Linear,
                |s, g| {
                    calls += 1;
                    draw(
                        s,
                        &cherenkov::Command::Fill {
                            shape: ShapeData::Rect(Rect::new(4.0, 4.0, 20.0, 20.0)),
                            paint: cherenkov::Paint::Solid(WorkingColor::new([1.0, 0.0, 0.0, 1.0])),
                        },
                        g,
                    )?;
                    draw(
                        s,
                        &cherenkov::Command::Fill {
                            shape: ShapeData::Rect(Rect::new(12.0, 12.0, 28.0, 28.0)),
                            paint: cherenkov::Paint::Solid(WorkingColor::new([0.0, 0.0, 1.0, 1.0])),
                        },
                        g,
                    )
                },
                &glyphs,
            )
            .expect("isolate");
        assert_eq!(calls, 1, "promote the speculative output without replay");
        draw(&mut lowering, &prefix, &glyphs).expect("suffix");
        lowering.finish_pass();
        assert_eq!(frame.passes.len(), 3);
        assert_eq!(frame.passes[0].target, Target::Part(0));
        assert_eq!(frame.passes[0].ranges[0].instances, 0..1);
        assert_eq!(frame.passes[1].target, Target::Scratch(0));
        assert_eq!(frame.passes[1].ranges[0].instances, 1..3);
        assert_eq!(frame.passes[2].target, Target::Part(0));
        assert_eq!(frame.passes[2].ranges[0].instances, 3..4);
        assert_eq!(frame.passes[2].ranges[1].instances, 4..5);
        let composite = frame
            .instances
            .iter()
            .find(|i| i.meta[1] == PAINT_TEXTURE)
            .expect("the composite instance");
        assert_eq!(composite.meta[0], KIND_SPAN, "composites are spans");
    }

    fn one_rect_body(s: &mut Lowering<'_>, g: &GlyphContext<'_>) -> Result<(), RenderError> {
        draw(
            s,
            &cherenkov::Command::Fill {
                shape: ShapeData::Rect(Rect::new(4.0, 4.0, 20.0, 20.0)),
                paint: cherenkov::Paint::Solid(WorkingColor::new([1.0, 0.0, 0.0, 1.0])),
            },
            g,
        )
    }

    /// A destructive blend composites over the whole clip (or parent),
    /// not the tight region of the layer's content.
    #[test]
    fn destructive_composite_covers_the_whole_parent() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let atlas = Atlas::new(&device, u64::MAX);
        let fonts = FxHashMap::default();
        let images = FxHashMap::default();
        let bitmaps = FxHashMap::default();
        let glyphs = GlyphContext {
            atlas: &atlas,
            live_stamp: atlas.live_stamp(),
            fonts: &fonts,
            images: &images,
            bitmaps: &bitmaps,
            content: &FxHashMap::default(),
        };
        let mut frame = Frame::default();
        let mut lowering = Lowering::new(&mut frame, (64, 64));
        lowering.begin_pass(Target::Part(0), None);
        lowering
            .isolate(
                None,
                None,
                1.0,
                cherenkov::BlendMode::Clear,
                cherenkov::BlendSpace::Linear,
                |s, g| one_rect_body(s, g),
                &glyphs,
            )
            .expect("destructive isolate");
        lowering
            .isolate(
                None,
                None,
                1.0,
                cherenkov::BlendMode::Multiply,
                cherenkov::BlendSpace::Linear,
                |s, g| one_rect_body(s, g),
                &glyphs,
            )
            .expect("tight isolate");
        lowering.finish_pass();
        let scratch: Vec<_> = frame
            .passes
            .iter()
            .filter(|p| matches!(p.target, Target::Scratch(_)))
            .map(|p| p.region)
            .collect();
        assert_eq!(scratch, vec![[0, 0, 64, 64], [1, 1, 22, 22]]);
        let composites: Vec<_> = frame
            .instances
            .iter()
            .filter(|i| i.meta[1] == PAINT_TEXTURE)
            .map(|i| i.bounds)
            .collect();
        assert_eq!(
            composites[0],
            [0.0, 0.0, 64.0, 64.0],
            "the destructive composite covers the whole parent"
        );
        assert_eq!(
            composites[1],
            [1.0, 1.0, 23.0, 23.0],
            "the multiply composite keeps the tight region"
        );
    }

    /// A clipped destructive composite covers the padded clip rect and
    /// carries the clip on the composite instance.
    #[test]
    fn destructive_composite_is_bounded_by_the_clip() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let atlas = Atlas::new(&device, u64::MAX);
        let fonts = FxHashMap::default();
        let images = FxHashMap::default();
        let bitmaps = FxHashMap::default();
        let glyphs = GlyphContext {
            atlas: &atlas,
            live_stamp: atlas.live_stamp(),
            fonts: &fonts,
            images: &images,
            bitmaps: &bitmaps,
            content: &FxHashMap::default(),
        };
        let clip = DeviceClip {
            inv: Affine::translate(Vec2::new(-20.0, -20.0)),
            shape: Shape::rect([10.0, 10.0]),
            aligned_rect: Some(Rect::new(10.0, 10.0, 30.0, 30.0)),
            mask: None,
        };
        let mut frame = Frame::default();
        let mut lowering = Lowering::new(&mut frame, (64, 64));
        lowering.begin_pass(Target::Part(0), None);
        lowering
            .isolate(
                Some(clip),
                None,
                1.0,
                cherenkov::BlendMode::DestAtop,
                cherenkov::BlendSpace::Linear,
                |s, g| one_rect_body(s, g),
                &glyphs,
            )
            .expect("clipped destructive isolate");
        lowering.finish_pass();
        let scratch = frame
            .passes
            .iter()
            .find(|p| matches!(p.target, Target::Scratch(_)))
            .expect("scratch pass");
        assert_eq!(scratch.region, [9, 9, 22, 22]);
        let composite = frame
            .instances
            .iter()
            .find(|i| i.meta[1] == PAINT_TEXTURE)
            .expect("the composite instance");
        assert_eq!(composite.bounds, [9.0, 9.0, 31.0, 31.0]);
        assert_ne!(
            composite.meta[3] & (FLAG_HAS_CLIP << 24),
            0,
            "the composite carries the effective clip"
        );
    }

    #[test]
    fn is_destructive_lists_the_six_operators() {
        use cherenkov::BlendMode as B;
        let all = [
            B::Normal,
            B::Multiply,
            B::Screen,
            B::Overlay,
            B::Darken,
            B::Lighten,
            B::ColorDodge,
            B::ColorBurn,
            B::HardLight,
            B::SoftLight,
            B::Difference,
            B::Exclusion,
            B::Hue,
            B::Saturation,
            B::Color,
            B::Luminosity,
            B::Clear,
            B::Src,
            B::Dst,
            B::DestOver,
            B::SrcIn,
            B::DestIn,
            B::SrcOut,
            B::DestOut,
            B::SrcAtop,
            B::DestAtop,
            B::Xor,
            B::PlusLighter,
        ];
        let destructive: Vec<_> = all.into_iter().filter(|b| is_destructive(*b)).collect();
        assert_eq!(
            destructive,
            [
                B::Clear,
                B::Src,
                B::SrcIn,
                B::DestIn,
                B::SrcOut,
                B::DestAtop
            ]
        );
    }

    /// A shadow whose negative spread collapses the shape's box emits no
    /// quad — a zero-area shape casts nothing.
    #[test]
    fn a_collapsed_shadow_emits_no_quads() {
        let mut frame = Frame::default();
        let mut lowering = Lowering::new(&mut frame, (64, 64));
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let atlas = Atlas::new(&device, u64::MAX);
        let fonts = FxHashMap::default();
        let images = FxHashMap::default();
        let bitmaps = FxHashMap::default();
        let glyphs = GlyphContext {
            atlas: &atlas,
            live_stamp: atlas.live_stamp(),
            fonts: &fonts,
            images: &images,
            bitmaps: &bitmaps,
            content: &FxHashMap::default(),
        };
        // Half extents [20, 5]: a spread of -20 inverts both.
        let bar = ShapeData::Rect(kurbo::Rect::new(20.0, 20.0, 60.0, 30.0));
        let collapsed =
            cherenkov::Shadow::new(2.0, WorkingColor::new([0.0, 0.0, 0.0, 1.0])).spread(-20.0);
        draw(
            &mut lowering,
            &cherenkov::Command::Shadow {
                shape: bar.clone(),
                shadow: collapsed,
            },
            &glyphs,
        )
        .expect("collapsed shadow is not an error");
        assert!(
            lowering.frame.instances.is_empty(),
            "a collapsed box emits no quad"
        );
        // A milder negative spread that leaves the box positive still
        // emits — and its radii clamp at zero rather than going negative.
        let shrunk =
            cherenkov::Shadow::new(2.0, WorkingColor::new([0.0, 0.0, 0.0, 1.0])).spread(-4.0);
        draw(
            &mut lowering,
            &cherenkov::Command::Shadow {
                shape: bar,
                shadow: shrunk,
            },
            &glyphs,
        )
        .expect("shadow");
        assert_eq!(lowering.frame.instances.len(), 1);
        assert!(
            lowering.frame.instances[0]
                .shape
                .half
                .iter()
                .all(|h| *h > 0.0)
        );
        assert!(
            lowering.frame.instances[0]
                .shape
                .radii
                .iter()
                .all(|r| *r >= 0.0)
        );
    }

    fn area(r: Rect) -> f64 {
        r.width() * r.height()
    }

    fn disjoint(strips: &[Rect]) -> bool {
        strips.iter().enumerate().all(|(i, a)| {
            strips[i + 1..].iter().all(|b| {
                let i = a.intersect(*b);
                !(i.width() > 0.0 && i.height() > 0.0)
            })
        })
    }

    fn inside(strips: &[Rect], b: Rect) -> bool {
        strips.iter().all(|r| r.intersect(b) == *r)
    }

    #[expect(clippy::suboptimal_flops, reason = "integer-valued geometry is exact")]
    #[test]
    fn cover_strips_full_cover() {
        let b = Rect::new(-20.0, -20.0, 20.0, 20.0);
        let c = Cover {
            wide: Rect::new(-15.0, -5.0, 15.0, 5.0),
            tall: Rect::new(-5.0, -15.0, 5.0, 15.0),
        };
        let strips: Vec<Rect> = cover_strips(b, c).collect();
        assert_eq!(strips.len(), 8);
        assert!(disjoint(&strips));
        assert!(inside(&strips, b));
        let total: f64 = strips.iter().map(|r| area(*r)).sum();
        assert_eq!(total, 1600.0 - (30.0 * 10.0 + 10.0 * 30.0 - 10.0 * 10.0));
    }

    #[test]
    fn cover_strips_one_empty_box() {
        let b = Rect::new(-20.0, -20.0, 20.0, 20.0);
        let c = Cover {
            wide: Rect::new(-15.0, -5.0, 15.0, 5.0),
            tall: Rect::new(-5.0, 0.0, 5.0, 0.0),
        };
        let strips: Vec<Rect> = cover_strips(b, c).collect();
        assert_eq!(strips.len(), 4);
        assert!(disjoint(&strips));
        assert!(inside(&strips, b));
        let total: f64 = strips.iter().map(|r| area(*r)).sum();
        assert_eq!(total, 1600.0 - 300.0);
    }

    #[test]
    fn cover_strips_both_empty() {
        let b = Rect::new(-20.0, -20.0, 20.0, 20.0);
        let c = Cover {
            wide: Rect::ZERO,
            tall: Rect::new(0.0, -5.0, 0.0, 5.0),
        };
        let strips: Vec<Rect> = cover_strips(b, c).collect();
        assert_eq!(strips, vec![b]);
    }

    #[test]
    fn cover_strips_clips_to_b() {
        let b = Rect::new(-20.0, -20.0, 20.0, 20.0);
        let c = Cover {
            wide: Rect::new(-15.0, -5.0, 40.0, 5.0),
            tall: Rect::new(-5.0, -40.0, 5.0, 15.0),
        };
        let strips: Vec<Rect> = cover_strips(b, c).collect();
        assert!(disjoint(&strips));
        assert!(inside(&strips, b));
        let covered = area(c.wide.intersect(b)) + area(c.tall.intersect(b))
            - area(c.wide.intersect(b).intersect(c.tall.intersect(b)));
        let total: f64 = strips.iter().map(|r| area(*r)).sum();
        assert_eq!(total, area(b) - covered);
    }

    fn sorted(clusters: Vec<Cluster>) -> Vec<([u32; 4], Vec<u32>)> {
        let mut out: Vec<_> = clusters
            .into_iter()
            .map(|c| {
                let mut members = c.members;
                members.sort_unstable();
                (c.bbox, members)
            })
            .collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn cluster_overlapping_rects_always_merge() {
        let rects = [[0, 0, 10, 10], [5, 5, 10, 10], [8, 2, 4, 4]];
        let merged = sorted(cluster(&rects, 0));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].0, [0, 0, 15, 15]);
        assert_eq!(merged[0].1, vec![0, 1, 2]);
    }

    #[test]
    fn cluster_touching_rects_always_merge() {
        let rects = [[0, 0, 10, 10], [10, 0, 10, 10]];
        let merged = cluster(&rects, 0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].bbox, [0, 0, 20, 10]);
    }

    #[test]
    fn cluster_far_rects_stay_apart() {
        let rects = [[0, 0, 10, 10], [1000, 1000, 10, 10]];
        assert_eq!(cluster(&rects, OVERHEAD_PX).len(), 2);
    }

    #[test]
    fn cluster_waste_gate_is_strictly_less() {
        // Two 10x10 rects 10px apart: bbox 30x10 = 300, waste = 100.
        let rects = [[0, 0, 10, 10], [20, 0, 10, 10]];
        assert_eq!(cluster(&rects, 100).len(), 2);
        let merged = cluster(&rects, 101);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].bbox, [0, 0, 30, 10]);
    }

    #[test]
    fn cluster_is_order_independent() {
        let rects = [
            [0, 0, 10, 10],
            [500, 0, 10, 10],
            [4, 4, 10, 10],
            [504, 4, 10, 10],
            [2000, 2000, 5, 5],
        ];
        let mut rev = rects;
        rev.reverse();
        let a = sorted(cluster(&rects, 50));
        let b = sorted(cluster(&rev, 50));
        let mut b_sorted = b;
        for (_, members) in &mut b_sorted {
            for m in members.iter_mut() {
                #[expect(clippy::cast_possible_truncation, reason = "test member count")]
                let n = rects.len() as u32;
                *m = n - 1 - *m;
            }
            members.sort_unstable();
        }
        b_sorted.sort_unstable();
        assert_eq!(a, b_sorted);
    }

    #[test]
    fn cluster_member_cap_forces_union() {
        #[expect(clippy::cast_possible_truncation, reason = "the member cap fits u32")]
        let rects: Vec<[u32; 4]> = (0..=MAX_CLUSTER_MEMBERS as u32)
            .map(|i| [i * 10_000, 0, 10, 10])
            .collect();
        let merged = cluster(&rects, 0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].members.len(), MAX_CLUSTER_MEMBERS + 1);
    }

    #[test]
    fn cluster_chain_merges_through_bridges() {
        // a--b overlap, then ab--c merges within overhead (waste 50).
        let rects = [[0, 0, 10, 10], [5, 0, 10, 10], [20, 0, 10, 10]];
        let merged = cluster(&rects, 60);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].bbox, [0, 0, 30, 10]);
    }

    /// A plan of `promoted` paint-order layers and `trailing` — the
    /// placement internals the walk reads are only the layer ids.
    fn plan(promoted: &[LayerId], trailing: bool) -> crate::render::planes::Plan {
        crate::render::planes::Plan {
            planes: promoted
                .iter()
                .map(|&layer| crate::render::planes::Placement {
                    raster: Affine::IDENTITY,
                    source: crate::render::planes::Source::Frame,
                    layer,
                    size: (8, 8),
                    opacity: 1.0,
                    path: Vec::new(),
                })
                .collect(),
            rejected: Vec::new(),
            trailing,
        }
    }

    /// A promoted layer the plan opens a part after ends the engine part
    /// it sits in: layers painted before it draw into part 0, layers
    /// after it into a transparent part 1, and its own content is left
    /// to its plane.
    #[test]
    fn a_promoted_layer_splits_the_surface_into_parts() {
        use cherenkov::testing::LayerOp;
        use cherenkov::{Draw as _, Picture, SurfaceTree};
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let atlas = Atlas::new(&device, u64::MAX);
        let (below, video, above) = (LayerId::new(1), LayerId::new(2), LayerId::new(3));
        let mut tree = SurfaceTree::new();
        for id in [below, video, above] {
            tree.apply(LayerOp::Create(id));
            tree.apply(LayerOp::Push {
                parent: tree.root(),
                child: id,
            });
        }
        let rect = |x: f64| {
            ContentData::new(Picture::record(|c| {
                c.fill(Rect::new(x, 0.0, x + 8.0, 8.0), WorkingColor::WHITE);
            }))
        };
        let lower = |plan: &crate::render::planes::Plan| {
            let mut caches: FxHashMap<LayerId, ContentData> =
                [(below, rect(0.0)), (above, rect(16.0))]
                    .into_iter()
                    .collect();
            let fonts = FxHashMap::default();
            let images = FxHashMap::default();
            let bitmaps = FxHashMap::default();
            let glyphs = GlyphContext {
                atlas: &atlas,
                live_stamp: atlas.live_stamp(),
                fonts: &fonts,
                images: &images,
                bitmaps: &bitmaps,
                content: &FxHashMap::default(),
            };
            let mut frame = Frame::default();
            let mut lowering = Lowering::new(&mut frame, (32, 32));
            lowering.prepare(&mut caches, &glyphs).expect("prepared");
            lowering
                .run(
                    &tree,
                    &mut caches,
                    WorkingColor::BLACK,
                    &glyphs,
                    &FxHashMap::default(),
                    FxHashMap::default(),
                    plan,
                )
                .expect("lowered");
            frame
                .passes
                .iter()
                .map(|p| (p.target, p.clear, p.ranges.len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lower(&plan(&[], false)),
            [(Target::Part(0), Some([0.0, 0.0, 0.0, 1.0]), 1)]
        );
        assert_eq!(
            lower(&plan(&[video], true)),
            [
                (Target::Part(0), Some([0.0, 0.0, 0.0, 1.0]), 1),
                (Target::Part(1), Some([0.0; 4]), 1),
            ]
        );
        // Nothing painted after the promoted layer opens no part: the
        // plan counts one, so the walk must emit one.
        assert_eq!(
            lower(&plan(&[above], false)),
            [(Target::Part(0), Some([0.0, 0.0, 0.0, 1.0]), 1)]
        );
        // Two promoted layers with the second painted last: the
        // separator part between them still opens, nothing after it.
        // Recorded pixels on the second plane are captured separately,
        // so the separator has no engine draws.
        assert_eq!(
            lower(&plan(&[video, above], false)),
            [
                (Target::Part(0), Some([0.0, 0.0, 0.0, 1.0]), 1),
                (Target::Part(1), Some([0.0; 4]), 0),
            ]
        );
    }
}
