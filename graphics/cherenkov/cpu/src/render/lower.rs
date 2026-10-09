//! Lowering: a surface's layer tree and display lists become one flat list
//! of rasterization [`Item`]s in device space.

mod silhouette;

use cherenkov::lowering::{Realization, shape_outline};
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use cherenkov::kurbo::{Affine, BezPath, PathEl, Point, Rect};
use cherenkov::lowering::rounded_box::{BoxForm, RoundedBox, box_form};
use cherenkov::{BlendMode, FillRule, FrameId, GlyphRun, GlyphStyle, ShapeData};

use cherenkov::{LayerId, RenderError, SurfaceId, SurfaceTree};

use super::filter::{Erased, Registry};
use super::prepared::Op;
use crate::names;
use crate::render::paint::PaintData;
use crate::render::raster::{Edge, coverage_mask};

/// Curve-to-path and stroke tolerance in device pixels.
pub const FLATTEN_TOL: f64 = 0.02;

/// An integer device-space rectangle (x ∈ `[x0, x1)`, y ∈ `[y0, y1)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    /// Left edge.
    pub x0: i32,
    /// Top edge.
    pub y0: i32,
    /// Right edge.
    pub x1: i32,
    /// Bottom edge.
    pub y1: i32,
}

/// A rasterized clip: an integer rect fast path or a full-surface
/// coverage mask.
#[derive(Debug, PartialEq)]
pub enum ClipMask {
    /// Axis-aligned rect with integer edges: coverage is 1 inside, 0
    /// outside.
    Rect(IRect),
    /// Exact-area coverage of a general clip shape, `w * h` cells.
    Cover(Vec<f32>),
}

/// A clip shared by items; cheap to clone.
pub type ClipRef = Arc<ClipMask>;

/// One rasterization item of a lowered frame.
#[derive(Clone)]
pub enum Item {
    /// A filled, flattened polygon.
    Draw {
        /// Directed edges in device space.
        edges: Arc<[Edge]>,
        /// Device-space bounding box of the edges.
        bbox: IRect,
        /// The fill rule.
        rule: FillRule,
        /// The paint evaluator.
        paint: PaintData,
        /// The clip in force.
        clip: Option<ClipRef>,
    },
    /// A Gaussian-blurred, axis-aligned rounded box.
    Shadow {
        /// The device-space axis-aligned rounded box half extents and
        /// centre, `[cx, cy, hx, hy]`.
        rbox: [f32; 4],
        /// Per-corner radii.
        radii: [f32; 4],
        /// The effective blur sigma (`sqrt(sigma² + 1/6)`).
        sigma_eff: f32,
        /// Premultiplied colour.
        color: [f32; 4],
        /// Device-space bounding box.
        bbox: IRect,
        /// The clip in force.
        clip: Option<ClipRef>,
    },
    /// A rasterized glyph mask instance.
    Glyph {
        /// The mask slot, filled before the band pass.
        slot: crate::render::glyph::GlyphSlot,
        /// The glyph's integer device origin (x).
        x: i32,
        /// The glyph's integer device origin (y).
        y: i32,
        /// The paint evaluator.
        paint: PaintData,
        /// The clip in force.
        clip: Option<ClipRef>,
    },
    /// Owned convolved silhouette coverage in surface coordinates.
    Silhouette {
        /// Completed immutable coverage, retained with its command.
        slot: crate::render::glyph::GlyphSlot,
        /// Premultiplied shadow colour.
        paint: PaintData,
        /// Clip applied after convolution.
        clip: Option<ClipRef>,
    },
    /// Start a fresh transparent scratch layer. The level stores
    /// premultiplied pixels in `space`: the isolate's declared
    /// `blend_space` when it composites in isolation — semantically
    /// isolated or translucent — or the enclosing level's space when it
    /// is pass-through: a pass-through level shares the space it merges
    /// back into.
    PushIsolate {
        /// The level's storage space.
        space: cherenkov::BlendSpace,
    },
    /// Composite the scratch layer onto what lies below it.
    PopIsolate {
        /// The opacity multiplier.
        opacity: f32,
        /// Group compositing mode and working space.
        blend: (BlendMode, cherenkov::BlendSpace),
        /// The clip in force at the pop.
        clip: Option<ClipRef>,
    },
    /// Start rendering a filtered scope with a vertical apron.
    PushFilter {
        /// The index of the paired [`Item::PopFilter`].
        end: u32,
        /// Rows of apron above and below each band.
        apron: usize,
        /// The scope's storage space: the isolate's declared
        /// `blend_space` — members composite in it before the filter
        /// sees the result.
        space: cherenkov::BlendSpace,
    },
    /// Filter and composite the current scratch window.
    PopFilter {
        /// The filter and its sampled parameter values.
        filter: FrameFilter,
        /// The opacity multiplier.
        opacity: f32,
        /// Group compositing mode and working space.
        blend: (BlendMode, cherenkov::BlendSpace),
        /// The clip in force at the pop.
        clip: Option<ClipRef>,
    },
    /// Capture the rows a backdrop group's members can sample: flatten
    /// the trailing `flatten` looked-through isolation levels —
    /// pass-through and translucent alike — over the nearest semantic
    /// level's contents and run the group's chain on the copy. Boxed: a
    /// capture is rare and large — it must not grow `Item`.
    Capture(Box<CaptureItem>),
    /// Composite a projective layer's local image, projected into this
    /// raster through the anisotropic filter. Boxed: rare and large.
    Project(Box<ProjectItem>),
    /// Sample a group's captured backdrop as a member's bottom-most
    /// content, inside the member's clip, over `bounds`.
    Sample {
        /// The group's renderer key (`BackdropId::raw()`).
        group: u64,
        /// The sampled rect: member ∩ region, in device pixels.
        bounds: IRect,
        /// The clip in force at the member.
        clip: Option<ClipRef>,
        /// What the sample becomes once composited.
        effect: SampleEffect,
        /// The union field the member folds into, when the group has one
        /// (or the member draws an `outer` band on its own).
        union: Option<UnionSample>,
    },
}

/// A member clip as the SDF reads evaluate it: its centred rounded box
/// and the device → box-local affine that maps a sample point into it —
/// the GPU's `clip_inv` and `Shape` pair.
#[derive(Clone, Copy, Debug)]
pub struct BoxClip {
    /// Device → box-local affine in kurbo coefficient order
    /// `(a, b, c, d, e, f)`: `x′ = a·x + c·y + e`,
    /// `y′ = b·x + d·y + f`.
    pub inv: [f32; 6],
    /// The centred box.
    pub shape: RoundedBox,
}

impl BoxClip {
    /// The box `shape` placed by the clip-local `extra` under the
    /// clip's `transform`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the engines evaluate SDF fields in f32"
    )]
    fn new(extra: Affine, shape: RoundedBox, transform: Affine) -> Self {
        Self {
            inv: (transform * extra).inverse().as_coeffs().map(|v| v as f32),
            shape,
        }
    }
}

/// The union-field data a member's sample composites against: every
/// member's clip box in paint order, the smoothing distance,
/// this member's index and its `outer` extent.
#[derive(Clone, Debug)]
pub struct UnionSample {
    /// Each member's clip box, in paint order. A lone `outer`-band
    /// member of a non-union group is its own one-member list with
    /// `k == 0`.
    pub members: Arc<[BoxClip]>,
    /// The group's smoothing distance `k`; `0` folds to the member's
    /// own distance.
    pub k: f32,
    /// This member's index in `members`.
    pub ord: u32,
    /// The member's `outer` extent: the composite's coverage is the
    /// antialiased ownership weight times the coverage of
    /// `field < outer`.
    pub outer: f32,
}

/// A projected composite's payload, boxed inside [`Item::Project`].
#[derive(Clone)]
pub struct ProjectItem {
    /// The local image and its placement.
    pub placed: crate::render::projective::Placed,
    /// The opacity multiplier.
    pub opacity: f32,
    /// The ancestor clip in force; the layer's own clip is already in the
    /// local image.
    pub clip: Option<ClipRef>,
}

/// The capture item's payload, boxed inside [`Item::Capture`].
#[derive(Clone)]
pub struct CaptureItem {
    /// The group's renderer key (`BackdropId::raw()`).
    pub group: u64,
    /// The capture scale `s`: `region` lies on the capture grid, whose
    /// texel `i` covers the device interval `[i/s, (i+1)/s)`.
    pub scale: cherenkov::CaptureScale,
    /// The pyramid's level count: levels `1..n` reduce the filtered
    /// level 0 by exact 2×2 boxes.
    pub levels: u32,
    /// The capture rect in capture texels (`union ⊕ apron` on the grid,
    /// clamped to the grid's extent).
    pub region: IRect,
    /// The members' union rows, clamped to the surface.
    pub union: IRect,
    /// The apron rows, in capture texels, the chain reads past the
    /// sampled rows.
    pub apron: usize,
    /// Extra rows around each band the capture covers: the deepest
    /// enclosing filter-scope apron plus effect sampling reach over the
    /// group's members.
    pub reach: usize,
    /// Whether members displace their capture reads.
    pub displaced: bool,
    /// The prepared chain, when the group is filtered.
    pub filter: Option<FrameFilter>,
    /// Looked-through isolation levels on the stack flattened over the
    /// nearest semantic level: pass-through and `opacity < 1` levels
    /// alike, composited as though each were at full opacity.
    pub flatten: usize,
}

/// How a backdrop sample composites into the member's layer. `None` is
/// the plain sample; the rest are #116's per-member effects.
#[derive(Clone, Debug)]
pub enum SampleEffect {
    /// Composite the captured rows unchanged under the member clip.
    None,
    /// A 3×4 premultiplied colour matrix (filtrate `ColorMatrix`
    /// layout): `dot(row, c)` per channel, alpha passes through.
    Color([f32; 12]),
    /// An effect reading the member clip's signed distance.
    Sdf(SdfEffect),
}

/// An SDF-reading member effect and the clip boundary it reads.
#[derive(Clone, Debug)]
pub struct SdfEffect {
    /// The member clip the distance and normal are read from.
    pub clip: Arc<BoxClip>,
    /// Which effect the distance and normal feed.
    pub kind: SdfKind,
}

/// The SDF effect evaluated per member pixel (the engine's
/// [`cherenkov::Refraction`] and [`cherenkov::Rim`]).
#[derive(Clone, Copy, Debug)]
pub enum SdfKind {
    /// `q = p - n · strength · t²` with `t = clamp(1 + d / depth, 0, 1)`.
    Refraction {
        /// Fade depth inside the edge, device pixels.
        depth: f32,
        /// Maximum displacement at the edge, device pixels.
        strength: f32,
    },
    /// `c.rgb += color.rgb · color.a · gain · t²` with
    /// `t = clamp(1 + d / width, 0, 1)`, alpha unchanged.
    Rim {
        /// Rim width inside the edge, device pixels.
        width: f32,
        /// Highlight colour, straight-alpha linear Display P3.
        color: [f32; 4],
        /// Highlight gain.
        gain: f32,
    },
    /// The trilinear `backdrop_sample_level` at
    /// `interior + (edge − interior)·t` with
    /// `t = clamp(1 + d / depth, 0, 1)`.
    Level {
        /// Fade depth inside the edge, device pixels.
        depth: f32,
        /// Pyramid level at the clip's edge (`t = 1`).
        edge: f32,
        /// Pyramid level deep inside the clip (`t = 0`).
        interior: f32,
    },
}

pub(super) type FrameFilter = (Arc<dyn Erased + Send + Sync>, Arc<[f32]>);

/// A layer's retained content and device output.
pub type ContentData = cherenkov::lowering::Content<Op, Emission>;

/// Per-operation device output under its sampled placement.
pub enum Emission {
    /// Rasterization items of a drawing operation.
    Draw(DeviceData<Vec<Item>>),
    /// Combined coverage of a clipping operation; None clips everything away.
    Clip(DeviceData<Option<ClipRef>>),
}

/// Device placement paired with its realized output.
pub struct DeviceData<T> {
    transform: Affine,
    clip: Option<ClipRef>,
    size: (usize, usize),
    output: T,
}

impl<T> DeviceData<T> {
    /// Heap bytes: the placement's clip mask plus `inner` on the output.
    pub fn heap_bytes(&self, inner: impl FnOnce(&T) -> u64) -> u64 {
        self.clip.as_deref().map_or(0, clip_bytes) + inner(&self.output)
    }
}

/// Heap bytes of a rasterized clip: the coverage mask or nothing for the
/// rect fast path.
pub const fn clip_bytes(clip: &ClipMask) -> u64 {
    match clip {
        ClipMask::Rect(_) => 0,
        ClipMask::Cover(mask) => (mask.capacity() * size_of::<f32>()) as u64,
    }
}

/// A glyph mask request lowering emits: everything needed to rasterize
/// the mask in parallel, plus the slot the [`Item::Glyph`] reads.
pub struct GlyphReq {
    /// The cache key.
    pub key: crate::render::glyph::GlyphKey,
    /// The engine font id.
    pub font: u64,
    /// The glyph index.
    pub glyph_id: u32,
    /// The run's size.
    pub size: f32,
    /// The quantized subpixel offset of the glyph origin.
    pub subpixel: (f32, f32),
    /// The device transform's 2x2 as f32.
    pub matrix: [f32; 4],
    /// The run's variation coordinates.
    pub coords: std::sync::Arc<[i16]>,
    /// The slot the emitted item reads.
    pub slot: crate::render::glyph::GlyphSlot,
}

/// The union group's member cap: the per-pixel fold keeps every member's
/// distance and gradient in registers, matching the GPU shader's arrays.
const UNION_MAX_MEMBERS: usize = cherenkov::BackdropUnion::MAX_MEMBERS as usize;

/// The plan for one backdrop group: the capture point (its first member
/// in paint order), the capture rect and every member's device bounds for
/// its sampling composite.
struct BackdropPlan {
    /// The first member layer in paint order — its entry emits the capture.
    first: LayerId,
    /// The union of members' clip bounds before the footprint apron.
    union: Rect,
    /// The union's device rows, clamped to the surface.
    union_rows: (usize, usize),
    /// The group's capture spec (scale and level count).
    spec: cherenkov::BackdropSpec,
    /// The capture rect in capture texels.
    region: IRect,
    /// The chain's apron in texel rows around the sampled rows.
    apron: usize,
    /// The device rows the capture's window reaches past the sampled
    /// rows: `apron` for a 1:1 capture, the texel window mapped back to
    /// device rows otherwise.
    device_apron: usize,
    /// Extra rows around each band the capture must cover: the deepest
    /// member scope apron plus effect reach over the members.
    reach: usize,
    /// The prepared chain, when the group is filtered.
    filter: Option<FrameFilter>,
    /// Each member layer's plan entry, keyed by layer.
    members: FxHashMap<LayerId, Member>,
    /// The members' layers in paint order: the union fold and the
    /// ownership tie-break are index-stable.
    order: Vec<LayerId>,
    /// The group's union smoothing distance; `0` when the group has no
    /// union field.
    union_k: f32,
    /// Every member's clip box in `order`, when the group unions.
    union_members: Arc<[BoxClip]>,
    /// The innermost filter scope the capture item lands in.
    scope: Option<LayerId>,
}

impl BackdropPlan {
    /// Places the capture on a `width × h` surface: the union's rows, and
    /// the union on the capture grid inflated by the chain `footprint`'s
    /// apron in texels — plus one texel for the bilinear taps of a reduced
    /// capture, and on a levelled group every deeper level's read
    /// footprint inflated by the apron — clamped to the grid's extent and
    /// rounded out to whole texels.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "footprints are non-negative and bounded by the surface height"
    )]
    fn place(&mut self, footprint: filtrate_core::Footprint, (width, h): (usize, usize)) {
        let (uw, uh) = (self.union.width(), self.union.height());
        let y0 = self.union.y0.floor().max(0.0) as usize;
        let y1 = (self.union.y1.ceil().min(h as f64) as usize).max(y0);
        self.union_rows = (y0, y1.min(h));
        if uw.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || uh.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
        {
            return;
        }
        // The footprint resolves against the capture's size in texels.
        let s = f64::from(self.spec.scale().get());
        let levels = self.spec.levels().get();
        let a = (f64::from(footprint.extent).mul_add(uw.max(uh) * s, f64::from(footprint.pixels))
            / 2.0f64.mul_add(-f64::from(footprint.extent), 1.0))
        .ceil();
        // A reduced capture is sampled bilinearly at `p · s`: an edge
        // pixel's second tap lies one texel past the union on the grid, so
        // the region covers that texel too.
        let taps = if self.spec.scale().is_full() {
            0.0
        } else {
            1.0
        };
        // The capture grid's extent in texels: `⌈len · s⌉`.
        let (gw, gh) = ((width as f64 * s).ceil(), (h as f64 * s).ceil());
        let (mut x0, mut y0, mut x1, mut y1) = (
            self.union.x0.mul_add(s, -a - taps).floor().max(0.0),
            self.union.y0.mul_add(s, -a - taps).floor().max(0.0),
            self.union.x1.mul_add(s, a + taps).ceil().min(gw),
            self.union.y1.mul_add(s, a + taps).ceil().min(gh),
        );
        // Levels 1..n: a bilinear read at level `k` taps level-k texels
        // `floor(q·s/2^k − 0.5)` through `+1`, and level-k texel `t`
        // reduces capture texels `[2^k·t, 2^k·(t+1))`. Each level's read
        // footprint is inflated by the chain's apron so every filtered
        // texel it reduces is computed from uncut chain input: a member's
        // result never depends on the other members.
        for k in 1..levels {
            let div = f64::from(1u32 << k);
            x0 = x0
                .min((self.union.x0 * s / div - 0.5).floor().mul_add(div, -a))
                .max(0.0);
            y0 = y0
                .min((self.union.y0 * s / div - 0.5).floor().mul_add(div, -a))
                .max(0.0);
            x1 = x1
                .max(((self.union.x1 * s / div - 0.5).floor() + 2.0).mul_add(div, a))
                .min(gw);
            y1 = y1
                .max(((self.union.y1 * s / div - 0.5).floor() + 2.0).mul_add(div, a))
                .min(gh);
        }
        // The region's origin sits on the deepest level's grid so level
        // `k` texel `(i, j)` covers capture texels `[2^k·i, 2^k·(i+1))`;
        // its far edge stays grid-clipped.
        let grid = f64::from(1u32 << (levels - 1));
        x0 = (x0 / grid).floor() * grid;
        y0 = (y0 / grid).floor() * grid;
        x1 = ((x1 / grid).ceil() * grid).min(gw);
        y1 = ((y1 / grid).ceil() * grid).min(gh);
        let region = IRect {
            x0: x0 as i32,
            y0: y0 as i32,
            x1: x1 as i32,
            y1: y1 as i32,
        };
        self.apron = a.min(gh) as usize;
        self.device_apron = if self.spec.scale().is_full() && levels == 1 {
            self.apron
        } else {
            // The texel window reaches `apron` texels past the kept
            // texels, which reach the sampled rows' bilinear taps plus a
            // margin and, on a levelled capture, the deepest level's
            // parent rows (see `raster::capture_rows`). All within
            // `(apron + margin + 2 + extra) / s` device rows of them.
            //
            // `extra` is 1.5 blocks: a deepest-level texel is a block of
            // `B = 2^(n−1)` capture texels. A sample at capture row `p`
            // sits at level row `p / B − 0.5`, so its bilinear taps are
            // level rows `i = ⌊p / B − 0.5⌋` and `i + 1`, whose parent
            // rows span `[i·B, (i + 2)·B)`. With `i ≥ p / B − 1.5` and
            // `i + 2 ≤ p / B + 1.5`, that is within `1.5·B` capture rows
            // of `p` on either side: the block plus the half-block the
            // taps sit further out.
            let extra = if levels > 1 {
                let block = 1usize << (levels - 1);
                block + block.div_ceil(2)
            } else {
                0
            };
            (((self.apron + super::raster::SAMPLE_MARGIN + 2 + extra) as f64 / s).ceil() as usize)
                .min(h)
        };
        if region.x0 < region.x1 && region.y0 < region.y1 {
            self.region = region;
        }
    }
}

/// One backdrop member's plan entry.
struct Member {
    /// The member's device-space clip bounds.
    bounds: Rect,
    /// The member's draw bounds: `bounds` further inflated by the
    /// union's `r(n)` and `outer` plus 1.5px of antialiasing, so bridge
    /// pixels between members are captured and painted.
    draw: Rect,
    /// The member's effect sampling reach — it inflates the capture
    /// footprint, never the draw bounds.
    reach: f64,
    /// The member's own clip's analytic box, when the group unions its
    /// members or the member draws an `outer` band.
    field: Option<BoxClip>,
    /// The member's `outer` extent.
    outer: f32,
    /// The member's index in the group's paint order.
    ord: u32,
    /// The innermost enclosing filter scope.
    scope: Option<LayerId>,
    /// The member's resolved per-member effect; `None` when its clip
    /// has no area and the member samples nothing.
    effect: Option<SampleEffect>,
}

/// The lowering walk state for one surface frame.
pub struct Lowering<'a, 'b> {
    items: &'a mut Vec<Item>,
    filters: Option<&'a mut Registry>,
    frame: FrameId,
    used_filters: FxHashSet<u64>,
    /// Backdrop groups referenced by members this frame.
    used_groups: FxHashSet<u64>,
    /// Glyph mask requests emitted during the walk.
    pub glyphs: Vec<GlyphReq>,
    pub glyphs_rasterized: u32,
    fonts: &'b mut FxHashMap<u64, super::font::Font>,
    bitmap_fonts: &'b FxHashMap<u64, Arc<super::bitmap::BitmapFont>>,
    bitmap_cache: &'b mut super::bitmap::BitmapCache,
    width: usize,
    height: usize,
    transform: Affine,
    /// Whether an enclosing layer's transform or scroll track is running;
    /// content then snaps its translation to the ¼-pixel grid.
    animating: bool,
    clip: Option<ClipRef>,
    /// Whether each open isolation level is looked through by a backdrop
    /// capture (`true` when its blend is `Normal` in the linear space —
    /// pass-through and translucent levels alike), in emission order.
    iso_kinds: Vec<bool>,
    /// The storage space of each open isolation level, in emission
    /// order, parallel to [`Lowering::iso_kinds`].
    iso_spaces: Vec<cherenkov::BlendSpace>,
    /// Backdrop groups planned before lowering, by group id.
    backdrops: FxHashMap<u64, BackdropPlan>,
    /// The apron rows each filtered layer's scope needs beyond its own
    /// footprint, by layer: a scope that directly contains captures grows
    /// to cover their `apron + reach`.
    scope_aprons: FxHashMap<LayerId, usize>,
    /// Source commands resolved this frame.
    pub commands_lowered: u32,
    /// Content layers composed this frame.
    pub layers_composed: u32,
    /// The layer this lowering starts at: the tree root for a surface, or
    /// a projective layer rendering its local image, with the transform
    /// that replaces its placement (layer space to texels).
    local: Option<(LayerId, Affine)>,
    /// Projective layers' images placed in this raster, by layer.
    projected: FxHashMap<LayerId, crate::render::projective::Placed>,
}

/// The largest singular value of `t`'s linear part — the worst-case factor
/// by which a user-space distance error can grow under the transform.
#[expect(
    clippy::many_single_char_names,
    reason = "a/b/c/d are the conventional affine matrix coefficient names"
)]
fn sigma_max(t: Affine) -> f64 {
    let [a, b, c, d, _, _] = t.as_coeffs();
    let p = a.mul_add(a, b * b) + c.mul_add(c, d * d);
    let det = a.mul_add(d, -(b * c));
    let disc = p.mul_add(p, (-4.0 * det) * det).sqrt();
    p.midpoint(disc).sqrt()
}

/// Resolves a member's engine effect into a [`SampleEffect`], also
/// returning its sampling reach (the member's bounds grow by it).
/// `clip` is the member clip's box; a path clip has none and gives SDF
/// effects no analytic boundary — the same `backdrop-effect-sdf-path`
/// error as the GPU slice. A `Shader` effect is a GPU-only capability on
/// this backend.
fn member_effect(
    effect: Option<&cherenkov::BackdropEffect>,
    clip: Option<BoxClip>,
) -> Result<(SampleEffect, f64), RenderError> {
    let Some(effect) = effect else {
        return Ok((SampleEffect::None, 0.0));
    };
    let kind = match effect {
        cherenkov::BackdropEffect::Color(m) => {
            return if m.0.iter().all(|v| v.is_finite()) {
                Ok((SampleEffect::Color(m.0), 0.0))
            } else {
                Err(RenderError::Render(
                    "backdrop colour effect has a non-finite matrix entry".into(),
                ))
            };
        }
        cherenkov::BackdropEffect::Refraction(r) => {
            if !(r.depth.is_finite()
                && r.depth > 0.0
                && r.strength.is_finite()
                && r.strength >= 0.0)
            {
                return Err(RenderError::Render(
                    "backdrop refraction needs depth > 0 and strength >= 0, finite".into(),
                ));
            }
            SdfKind::Refraction {
                depth: r.depth,
                strength: r.strength,
            }
        }
        cherenkov::BackdropEffect::Rim(r) => {
            if !(r.width.is_finite()
                && r.width > 0.0
                && r.gain.is_finite()
                && r.color.iter().all(|v| v.is_finite()))
            {
                return Err(RenderError::Render(
                    "backdrop rim needs width > 0 and a finite colour and gain".into(),
                ));
            }
            SdfKind::Rim {
                width: r.width,
                color: r.color,
                gain: r.gain,
            }
        }
        cherenkov::BackdropEffect::Level(r) => SdfKind::Level {
            depth: r.depth(),
            edge: r.edge(),
            interior: r.interior(),
        },
        cherenkov::BackdropEffect::Shader(_) => {
            return Err(RenderError::Unsupported(names::BACKDROP_SHADER));
        }
    };
    let clip = clip.ok_or(RenderError::Unsupported(names::BACKDROP_EFFECT_SDF_PATH))?;
    Ok((
        SampleEffect::Sdf(SdfEffect {
            clip: Arc::new(clip),
            kind,
        }),
        f64::from(effect.reach()),
    ))
}

/// Flattens `path` (already in device space) into directed edges for
/// coverage rasterization, implicit-close applied: horizontal edges
/// carry no area and drop out.
#[expect(clippy::cast_possible_truncation, reason = "geometry is f32")]
fn flatten_edges(path: BezPath, tol: f64) -> Vec<Edge> {
    let mut edges = Vec::new();
    let mut cur = Point::ZERO;
    let mut start = Point::ZERO;
    let close = |edges: &mut Vec<Edge>, cur: Point, start: Point| {
        // Fills implicitly close open subpaths.
        if cur != start {
            edges.push(Edge {
                x0: cur.x as f32,
                y0: cur.y as f32,
                x1: start.x as f32,
                y1: start.y as f32,
            });
        }
    };
    kurbo::flatten(path, tol, |el| match el {
        PathEl::MoveTo(p) => {
            close(&mut edges, cur, start);
            cur = p;
            start = p;
        }
        PathEl::LineTo(p) => {
            edges.push(Edge {
                x0: cur.x as f32,
                y0: cur.y as f32,
                x1: p.x as f32,
                y1: p.y as f32,
            });
            cur = p;
        }
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
            // `kurbo::flatten` never emits curves.
            debug_assert!(false, "flatten emits only lines");
        }
        PathEl::ClosePath => {
            close(&mut edges, cur, start);
            cur = start;
        }
    });
    close(&mut edges, cur, start);
    #[expect(clippy::float_cmp, reason = "horizontal edges carry no area")]
    edges.retain(|e| e.y0 != e.y1);
    edges
}

/// Resolves overlapping windings: `Some` swaps in boundary edges whose
/// winding is 0 or 1 everywhere, drawn under `NonZero`. `None` keeps the
/// original edges and rule untouched.
pub fn resolve_edges(edges: Vec<Edge>, rule: FillRule) -> (Vec<Edge>, FillRule) {
    let segments: Vec<(f32, f32, f32, f32)> =
        edges.iter().map(|e| (e.x0, e.y0, e.x1, e.y1)).collect();
    cherenkov::lowering::resolve_winding(&segments, rule).map_or((edges, rule), |resolved| {
        (
            resolved
                .into_iter()
                .map(|(x0, y0, x1, y1)| Edge { x0, y0, x1, y1 })
                .collect(),
            FillRule::NonZero,
        )
    })
}

/// The bounding box of `edges` as an integer rect intersected with the
/// surface.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "edge coordinates fit i32 on a real surface"
)]
fn bbox_of(edges: &[Edge], w: usize, h: usize) -> IRect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for e in edges {
        x0 = x0.min(e.x0.min(e.x1));
        y0 = y0.min(e.y0.min(e.y1));
        x1 = x1.max(e.x0.max(e.x1));
        y1 = y1.max(e.y0.max(e.y1));
    }
    IRect {
        x0: (x0.floor() as i32).max(0).min(w as i32),
        y0: (y0.floor() as i32).max(0).min(h as i32),
        x1: (x1.ceil() as i32).max(0).min(w as i32),
        y1: (y1.ceil() as i32).max(0).min(h as i32),
    }
}

/// Whether the transform's 2x2 is axis-aligned (`b == c == 0`, or a 90°
/// rotation with `a == d == 0`).
fn axis_aligned(transform: Affine) -> bool {
    let [c0, c1, c2, c3, _, _] = transform.as_coeffs();
    (c1 == 0.0 && c2 == 0.0) || (c0 == 0.0 && c3 == 0.0)
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

/// The device-space bounding box of `clip` under `transform` — the shape
/// the merged clip machinery would clip against, any transform. A shape
/// with no path (`Line`) has no bounds.
fn clip_device_bounds(transform: Affine, clip: &ShapeData) -> Rect {
    use kurbo::Shape as _;
    let Some((path, _)) = shape_outline(clip, FLATTEN_TOL) else {
        return Rect::ZERO;
    };
    let local = path.bounding_box();
    let corners = [
        Point::new(local.x0, local.y0),
        Point::new(local.x1, local.y0),
        Point::new(local.x0, local.y1),
        Point::new(local.x1, local.y1),
    ]
    .map(|p| transform * p);
    let (mut min, mut max) = (corners[0], corners[0]);
    for p in &corners[1..] {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    Rect::new(min.x, min.y, max.x, max.y)
}

/// Whether all four edges of `r` are within `1e-6` of integers.
fn integer_edges(r: Rect) -> Option<IRect> {
    let close = |v: f64| (v - v.round()).abs() <= 1e-6;
    if close(r.x0) && close(r.y0) && close(r.x1) && close(r.y1) {
        Some(IRect {
            #[expect(clippy::cast_possible_truncation)]
            x0: r.x0.round() as i32,
            #[expect(clippy::cast_possible_truncation)]
            y0: r.y0.round() as i32,
            #[expect(clippy::cast_possible_truncation)]
            x1: r.x1.round() as i32,
            #[expect(clippy::cast_possible_truncation)]
            y1: r.y1.round() as i32,
        })
    } else {
        None
    }
}

impl<'a, 'b> Lowering<'a, 'b> {
    /// Starts a lowering into `items` for a `w` × `h` surface.
    pub fn new(
        items: &'a mut Vec<Item>,
        size: (u32, u32),
        filters: Option<&'a mut Registry>,
        frame: FrameId,
        fonts: &'b mut FxHashMap<u64, super::font::Font>,
        bitmap_fonts: &'b FxHashMap<u64, Arc<super::bitmap::BitmapFont>>,
        bitmap_cache: &'b mut super::bitmap::BitmapCache,
    ) -> Self {
        Self {
            items,
            filters,
            frame,
            used_filters: FxHashSet::default(),
            used_groups: FxHashSet::default(),
            glyphs: Vec::new(),
            glyphs_rasterized: 0,
            fonts,
            bitmap_fonts,
            bitmap_cache,
            width: size.0 as usize,
            height: size.1 as usize,
            transform: Affine::IDENTITY,
            animating: false,
            clip: None,
            iso_kinds: Vec::new(),
            iso_spaces: Vec::new(),
            backdrops: FxHashMap::default(),
            scope_aprons: FxHashMap::default(),
            commands_lowered: 0,
            layers_composed: 0,
            local: None,
            projected: FxHashMap::default(),
        }
    }

    /// Renders the local image of projective layer `root` under
    /// `transform` (layer space to texels) instead of the surface, and
    /// places nested projective layers' images from `projected`. Without
    /// a call, the lowering renders the surface from the tree root.
    pub fn project(
        &mut self,
        local: Option<(LayerId, Affine)>,
        projected: FxHashMap<LayerId, crate::render::projective::Placed>,
    ) {
        self.local = local;
        self.projected = projected;
    }

    /// The layer the walk starts at.
    fn start(&self, tree: &SurfaceTree) -> LayerId {
        self.local.map_or_else(|| tree.root(), |(id, _)| id)
    }

    /// Whether `id` composes by projection into this raster: projective,
    /// and not the layer whose local image this lowering renders.
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
            Some((root, local)) if root == id => {
                (local, local * Affine::translate(-node.scroll_offset))
            }
            _ => (parent * node.transform, parent * node.content_transform()),
        }
    }

    /// Lowers a surface's sampled [`SurfaceTree`]. `caches` holds each
    /// layer's render-side content; the clear colour is applied by the
    /// rasterizer, so the lowering emits only the layers' items. `fonts`
    /// is mutable: lowering fills each font's `COLRv1` node-tree cache.
    pub fn run(
        &mut self,
        surface: SurfaceId,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
        images: &FxHashMap<u64, Arc<super::image::CpuImage>>,
    ) -> Result<(), RenderError> {
        let mut lowerer = super::prepared::Lowerer {
            images,
            fonts: &mut *self.fonts,
            bitmap_fonts: self.bitmap_fonts,
        };
        for content in caches.values_mut() {
            self.commands_lowered += content.prepare(&mut lowerer)?;
        }
        // The registry borrow must leave `self` for the planning walk.
        let mut filters = self.filters.take();
        let planned = filters.as_deref_mut().map_or(Ok(()), |registry| {
            self.plan_backdrops(tree, surface, registry)
        });
        self.filters = filters;
        planned?;
        self.layer(self.start(tree), tree, caches)
    }

    pub fn take_used_filters(&mut self) -> FxHashSet<u64> {
        std::mem::take(&mut self.used_filters)
    }

    /// The backdrop groups members referenced this frame.
    pub fn take_used_groups(&mut self) -> FxHashSet<u64> {
        std::mem::take(&mut self.used_groups)
    }

    /// One layer of the backdrop planning walk, mirroring `layer`'s
    /// transform math: the clip sits in `parent * node.transform` space,
    /// children in `parent * node.content_transform()` space. `scopes`
    /// holds the enclosing filtered layers, innermost last.
    #[expect(
        clippy::too_many_arguments,
        reason = "the walk threads both trees and the scope stack"
    )]
    fn plan_layer(
        &mut self,
        id: LayerId,
        tree: &SurfaceTree,
        groups: &mut FxHashMap<u64, super::filter::PreparedBackdrop>,
        filters: &mut Registry,
        surface: SurfaceId,
        parent: Affine,
        scopes: &mut Vec<LayerId>,
    ) -> Result<(), RenderError> {
        let node = tree.layer(id);
        if self.projects(id, tree) {
            // Its members plan in its own local lowering.
            return Ok(());
        }
        let (transform, children) = self.placement(id, node, parent);
        if let Some(sample) = &node.backdrop {
            let gid = sample.group();
            let g = gid.raw();
            self.used_groups.insert(g);
            let prepared = match groups.entry(g) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(filters.prepare_backdrop(surface, gid, self.frame).map_err(
                        |error| match error {
                            RenderError::Render(_) => RenderError::Render(format!(
                                "layer {id:?} samples unknown backdrop group {gid:?}"
                            )),
                            other => other,
                        },
                    )?)
                }
            };
            let clip = node
                .clip
                .as_ref()
                .ok_or(RenderError::Unsupported(names::BACKDROP_UNCLIPPED))?;
            self.plan_member(
                id,
                sample,
                clip,
                transform,
                prepared,
                scopes.last().copied(),
            )?;
        }
        if node.filter.is_some() {
            scopes.push(id);
        }
        for child in &node.children {
            self.plan_layer(*child, tree, groups, filters, surface, children, scopes)?;
        }
        if node.filter.is_some() {
            scopes.pop();
        }
        Ok(())
    }

    /// Plans layer `id`'s backdrop sample of group `prepared`: the member's
    /// device clip bounds, its effect and, when the group unions its
    /// members or the member draws an `outer` band, the clip's box its
    /// field reads. `scope` is the innermost enclosing filtered layer.
    ///
    /// A clip without area — a line, or a circle or ellipse with a zero
    /// radius — draws nothing, so the member keeps its place in the
    /// group's paint order but samples nothing, as on the GPU; a union or
    /// `outer` member needs its clip's box as its field and rejects it.
    /// A path clip has no box: only a member that reads no distance may
    /// use one.
    fn plan_member(
        &mut self,
        id: LayerId,
        sample: &cherenkov::BackdropSample,
        clip: &ShapeData,
        transform: Affine,
        prepared: &super::filter::PreparedBackdrop,
        scope: Option<LayerId>,
    ) -> Result<(), RenderError> {
        let g = sample.group().raw();
        let outer = sample.outer_extent().get();
        let spec = prepared.spec;
        let field_member = spec.union_field().is_some() || outer > 0.0;
        let (field, effect, reach) = match box_form(clip) {
            BoxForm::Empty if field_member => {
                return Err(RenderError::Unsupported(
                    names::BACKDROP_UNION_DEGENERATE_MEMBER,
                ));
            }
            // The member still orders the group — the capture lands at
            // the first member in paint order — but samples nothing.
            BoxForm::Empty => (
                None,
                None,
                sample.effect().map_or(0.0, |e| f64::from(e.reach())),
            ),
            form => {
                let clip_box = match form {
                    BoxForm::Box { extra, shape } => Some(BoxClip::new(extra, shape, transform)),
                    _ => None,
                };
                let field = field_member
                    .then(|| {
                        clip_box.ok_or(RenderError::Unsupported(names::BACKDROP_EFFECT_SDF_PATH))
                    })
                    .transpose()?;
                let (effect, reach) = member_effect(sample.effect(), clip_box)?;
                (field, Some(effect), reach)
            }
        };
        let member = clip_device_bounds(transform, clip);
        let plan = self.backdrops.entry(g).or_insert_with(|| BackdropPlan {
            first: id,
            union: member,
            union_rows: (0, 0),
            spec,
            region: IRect {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            },
            apron: 0,
            device_apron: 0,
            reach: 0,
            filter: prepared.filter.clone(),
            members: FxHashMap::default(),
            order: Vec::new(),
            union_k: 0.0,
            union_members: Arc::from([]),
            scope,
        });
        // The capture footprint inflates the clip bounds by the
        // effect's reach — the union pad lands in `plan_union`.
        plan.union = plan.union.union(member.inflate(reach, reach));
        let ord =
            u32::try_from(plan.order.len()).expect("a group's members never exceed MAX_MEMBERS");
        plan.order.push(id);
        plan.members.insert(
            id,
            Member {
                bounds: member,
                draw: member,
                reach,
                field,
                outer,
                ord,
                scope,
                effect,
            },
        );
        Ok(())
    }

    /// Plans every backdrop group: a paint-order walk collecting each
    /// member's device-space clip bounds, then the capture region — the
    /// union on the capture grid inflated by the chain footprint's apron
    /// in texels, intersected with the grid's extent and rounded outward
    /// to whole texels.
    ///
    /// A member inside a filter scope samples rows within that scope's
    /// window (`band ± apron`), and every member's samples reach its
    /// effect's `reach` past the sampled pixel; each group's `reach` is
    /// the deepest `apron + effect reach` over its members, and a scope
    /// directly containing a capture grows its apron to `apron + reach`
    /// so the capture's window fits. Both bounds are monotone in each
    /// other and capped at the surface height, so the fixed point is
    /// found by iteration.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "footprints are non-negative and bounded by the surface height"
    )]
    fn plan_backdrops(
        &mut self,
        tree: &SurfaceTree,
        surface: SurfaceId,
        filters: &mut Registry,
    ) -> Result<(), RenderError> {
        let mut groups = FxHashMap::default();
        self.plan_layer(
            self.start(tree),
            tree,
            &mut groups,
            filters,
            surface,
            Affine::IDENTITY,
            &mut Vec::new(),
        )?;
        if self.backdrops.is_empty() {
            return Ok(());
        }
        let h = self.height;
        // Union fields: inflate every member's draw bounds by `r(n) +
        // outer` plus 1.5px of antialiasing so bridge pixels between
        // members and the `outer` band are captured and drawn, then
        // recompute the capture's union from the inflated bounds. The
        // member clip boxes, in paint order, become the fold's operands.
        for plan in self.backdrops.values_mut() {
            Self::plan_union(plan)?;
        }
        for (gid, plan) in &mut self.backdrops {
            let footprint = groups[gid].footprint;
            if footprint.extent.partial_cmp(&0.5) != Some(std::cmp::Ordering::Less) {
                return Err(RenderError::Unsupported(names::BACKDROP_FOOTPRINT));
            }
            plan.place(footprint, (self.width, h));
        }
        // The apron a scope needs around each band — its own filter's
        // footprint, or `capture apron + reach` for scopes a capture
        // lands directly in.
        let mut aprons: FxHashMap<LayerId, usize> = FxHashMap::default();
        for plan in self.backdrops.values() {
            for &scope in plan
                .members
                .values()
                .filter_map(|m| m.scope.as_ref())
                .chain(plan.scope.iter())
            {
                if let std::collections::hash_map::Entry::Vacant(e) = aprons.entry(scope) {
                    let node = tree.layer(scope);
                    let filter = node.filter.expect("a scope is a filtered layer");
                    let (_, _, footprint) = filters.prepare(filter, self.frame, (self.width, h))?;
                    if !footprint.is_finite() || footprint < 0.0 {
                        return Err(RenderError::Render(format!(
                            "filter {} has invalid CPU footprint {footprint}",
                            filter.raw()
                        )));
                    }
                    e.insert(if footprint >= h as f32 {
                        h
                    } else {
                        footprint.ceil() as usize
                    });
                }
            }
        }
        loop {
            let mut changed = false;
            for plan in self.backdrops.values_mut() {
                let reach = plan
                    .members
                    .values()
                    .map(|m| m.scope.map_or(0, |s| aprons[&s]) + m.reach.ceil() as usize)
                    .fold(0, usize::max);
                changed |= reach != plan.reach;
                plan.reach = reach;
            }
            for plan in self.backdrops.values() {
                if let Some(scope) = plan.scope {
                    let needed = (plan.device_apron + plan.reach).min(h);
                    let apron = aprons.get_mut(&scope).expect("capture scope registered");
                    if *apron < needed {
                        *apron = needed;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        self.scope_aprons = aprons;
        Ok(())
    }

    /// Inflates a backdrop plan's members for the union field: draw bounds
    /// grow by `r(n) + outer` plus 1.5px of antialiasing so bridge pixels
    /// between members and the `outer` band are captured and drawn, and the
    /// capture's union recomputes from the inflated bounds. The member
    /// clip boxes, in paint order, become the fold's operands.
    fn plan_union(plan: &mut BackdropPlan) -> Result<(), RenderError> {
        let union_spec = plan.spec.union_field();
        let needs_field = union_spec.is_some() || plan.members.values().any(|m| m.outer > 0.0);
        if !needs_field {
            return Ok(());
        }
        let n = plan.order.len();
        if union_spec.is_some() && n > UNION_MAX_MEMBERS {
            return Err(RenderError::Unsupported(names::BACKDROP_UNION_MEMBERS));
        }
        let k = union_spec.map_or(0.0, |u| f64::from(u.get()));
        let r = union_spec.map_or(0.0, |u| u.inflation(n));
        let mut union = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        // Member clip boxes are fold operands, so they are collected only
        // when the group has a union — a non-union member's outer band
        // runs against its own field at raster time and no other
        // member's box exists.
        let mut members = Vec::new();
        if union_spec.is_some() {
            members.reserve_exact(n);
        }
        for &id in &plan.order {
            let member = plan
                .members
                .get_mut(&id)
                .expect("order lists planned members");
            let pad = if union_spec.is_some() || member.outer > 0.0 {
                r + f64::from(member.outer) + 1.5
            } else {
                // No field: the member is not inflated at all.
                0.0
            };
            member.draw = member.bounds.inflate(pad, pad);
            // The capture footprint carries the effect's reach too;
            // the draw bounds do not.
            union = union.union(
                member
                    .bounds
                    .inflate(member.reach + pad, member.reach + pad),
            );
            if union_spec.is_some() {
                members.push(member.field.expect("union members carry clip boxes"));
            }
        }
        plan.union = union;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "smoothing distances fit in f32"
        )]
        if k > 0.0 {
            plan.union_k = k as f32;
            plan.union_members = members.into();
        }
        Ok(())
    }

    /// Emits the member's `Sample` item, covering the member's draw
    /// bounds ∩ region, at full strength — the member's own scope
    /// attenuates it at composite. A union member's item carries the
    /// group's shared field and the ancestors-only clip (`ancestors`);
    /// a non-union member's carries the clip in force, unchanged.
    fn emit_backdrop_sample(&mut self, gid: u64, member: LayerId, ancestors: Option<&ClipRef>) {
        let Some(plan) = self.backdrops.get(&gid) else {
            return;
        };
        if plan.region.x0 >= plan.region.x1 || plan.region.y0 >= plan.region.y1 {
            return;
        }
        let Some(entry) = plan.members.get(&member) else {
            return;
        };
        let Some(effect) = entry.effect.clone() else {
            return;
        };
        let bounds = entry.draw;
        // The device rect the region's texels cover.
        let s = f64::from(plan.spec.scale().get());
        let region = Rect::new(
            f64::from(plan.region.x0) / s,
            f64::from(plan.region.y0) / s,
            f64::from(plan.region.x1) / s,
            f64::from(plan.region.y1) / s,
        );
        let bounds = bounds.intersect(region);
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "bounds are finite and inside the surface"
        )]
        let bounds = IRect {
            x0: (bounds.x0.floor() as i32).clamp(0, self.width as i32),
            y0: (bounds.y0.floor() as i32).clamp(0, self.height as i32),
            x1: (bounds.x1.ceil() as i32).clamp(0, self.width as i32),
            y1: (bounds.y1.ceil() as i32).clamp(0, self.height as i32),
        };
        // The union-fold data and clip the item carries: a union member
        // composites under the ancestors-only clip against the shared
        // field; a lone `outer` member gets its own one-member field;
        // anything else composes under the clip in force, as today.
        let (clip, union) = if plan.union_k > 0.0 {
            (
                ancestors.cloned(),
                Some(UnionSample {
                    members: plan.union_members.clone(),
                    k: plan.union_k,
                    ord: entry.ord,
                    outer: entry.outer,
                }),
            )
        } else if entry.outer > 0.0 {
            (
                ancestors.cloned(),
                Some(UnionSample {
                    members: Arc::from([entry.field.expect("outer members carry clip boxes")]),
                    k: 0.0,
                    ord: 0,
                    outer: entry.outer,
                }),
            )
        } else {
            (self.clip.clone(), None)
        };
        self.items.push(Item::Sample {
            group: gid,
            bounds,
            clip,
            effect,
            union,
        });
    }

    /// The coverage a clip contributes at `(px, py)`.
    fn clip_cov_at(clip: &ClipMask, px: usize, py: usize, w: usize) -> f32 {
        match clip {
            ClipMask::Rect(r) => f32::from(
                px >= usize::try_from(r.x0).unwrap_or(0)
                    && px < usize::try_from(r.x1).unwrap_or(0)
                    && py >= usize::try_from(r.y0).unwrap_or(0)
                    && py < usize::try_from(r.y1).unwrap_or(0),
            ),
            ClipMask::Cover(mask) => mask[py * w + px],
        }
    }

    /// A clip shape under the current transform becomes a [`ClipRef`],
    /// combined (coverage product) with the clip already in force.
    fn make_clip(&self, shape: &ShapeData) -> Option<ClipRef> {
        // The rect fast path: an axis-aligned rect with integer-ish edges.
        let new_rect = match shape {
            ShapeData::Rect(r) if axis_aligned(self.transform) => device_rect(self.transform, *r),
            _ => Rect::ZERO, // sentinel: not applicable
        };
        let new_is_rect = matches!(shape, ShapeData::Rect(_))
            && axis_aligned(self.transform)
            && integer_edges(new_rect).is_some();
        if let Some(current) = &self.clip {
            if new_is_rect
                && let ClipMask::Rect(cur) = current.as_ref()
                && let Some(r) = integer_edges(new_rect)
            {
                return Some(Arc::new(ClipMask::Rect(IRect {
                    x0: cur.x0.max(r.x0),
                    y0: cur.y0.max(r.y0),
                    x1: cur.x1.min(r.x1),
                    y1: cur.y1.min(r.y1),
                })));
            }
        } else if new_is_rect {
            return integer_edges(new_rect).map(|r| Arc::new(ClipMask::Rect(r)));
        }
        // General path: rasterize the new clip's coverage over the full
        // surface, multiplied by the coverage of the clip already in
        // force (coverage product — the oracle intersects geometry; the
        // product is the accepted approximation, as on the GPU slice).
        let (w, h) = (self.width, self.height);
        let sm = sigma_max(self.transform).max(1e-12);
        let tol_u = FLATTEN_TOL / sm;
        let (path, rule) = shape_outline(shape, tol_u)?;
        let edges = flatten_edges(self.transform * path, FLATTEN_TOL);
        let (edges, rule) = resolve_edges(edges, rule);
        let mut mask = coverage_mask(&edges, rule, w, h);
        if let Some(current) = &self.clip {
            for (i, m) in mask.iter_mut().enumerate() {
                let (px, py) = (i % w, i / w);
                *m *= Self::clip_cov_at(current, px, py, w);
            }
        }
        Some(Arc::new(ClipMask::Cover(mask)))
    }

    /// Applies `clip` around `body`: `None` passes through.
    fn with_clip(
        &mut self,
        shape: Option<&ShapeData>,
        body: impl FnOnce(&mut Self) -> Result<(), RenderError>,
    ) -> Result<(), RenderError> {
        let Some(shape_data) = shape else {
            return body(self);
        };
        let Some(clip) = self.make_clip(shape_data) else {
            // A clip path with no area clips everything away.
            return Ok(());
        };
        let saved = self.clip.replace(clip);
        let result = body(self);
        self.clip = saved;
        result
    }

    /// Emits the isolation pair around `body`.
    fn isolate(
        &mut self,
        opacity: f32,
        blend: (BlendMode, cherenkov::BlendSpace),
        inner_clip: Option<ClipRef>,
        outer_clip: Option<ClipRef>,
        body: impl FnOnce(&mut Self) -> Result<(), RenderError>,
    ) -> Result<(), RenderError> {
        let saved = std::mem::replace(&mut self.clip, inner_clip);
        // A backdrop capture looks through a level exactly when it is
        // `Normal`-blended in the linear space: pass-through and
        // `opacity < 1` levels alike. A non-linear-space isolate changes
        // pixels even when fully transparent and normally blended: it
        // stays semantic.
        let looked_through =
            blend.0 == BlendMode::Normal && blend.1 == cherenkov::BlendSpace::Linear;
        // A level that composites in isolation — semantic or translucent —
        // keeps its declared space for members to composite in; a
        // pass-through level shares the space it merges back into.
        let space = if opacity >= 1.0 && looked_through {
            self.iso_spaces
                .last()
                .copied()
                .unwrap_or(cherenkov::BlendSpace::Linear)
        } else {
            blend.1
        };
        self.iso_kinds.push(looked_through);
        self.iso_spaces.push(space);
        self.items.push(Item::PushIsolate { space });
        let result = body(self);
        self.iso_kinds.pop();
        self.iso_spaces.pop();
        self.clip = saved;
        self.items.push(Item::PopIsolate {
            opacity,
            blend,
            clip: outer_clip,
        });
        result
    }

    fn filter_isolate(
        &mut self,
        id: cherenkov::FilterId,
        opacity: f32,
        blend: (BlendMode, cherenkov::BlendSpace),
        clip: Option<ClipRef>,
        body: impl FnOnce(&mut Self) -> Result<(), RenderError>,
        scope: Option<LayerId>,
    ) -> Result<(), RenderError> {
        let (filter, params, footprint) = self
            .filters
            .as_deref_mut()
            .ok_or_else(|| RenderError::Render(format!("unregistered filter {}", id.raw())))?
            .prepare(id, self.frame, (self.width, self.height))?;
        if !footprint.is_finite() || footprint < 0.0 {
            return Err(RenderError::Render(format!(
                "filter {} has invalid CPU footprint {footprint}",
                id.raw()
            )));
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "the nonnegative footprint is compared with the bounded surface height first"
        )]
        let footprint_apron = if footprint >= self.height as f32 {
            self.height
        } else {
            footprint.ceil() as usize
        };
        // A scope containing a backdrop capture grows to cover the
        // capture's apron and reach, so its window always holds the rows
        // the capture reads.
        let apron = scope
            .and_then(|id| self.scope_aprons.get(&id).copied())
            .unwrap_or(0)
            .max(footprint_apron);
        self.used_filters.insert(id.raw());
        let push = self.items.len();
        // The scope stores its declared space: members composite in it
        // and the filter sees the result.
        self.items.push(Item::PushFilter {
            end: 0,
            apron,
            space: blend.1,
        });
        // The scope's nested run gets a fresh isolation stack; the kinds
        // and spaces stacks mirror it, seeded at the scope's own space.
        let saved_kinds = std::mem::take(&mut self.iso_kinds);
        let saved_spaces = std::mem::replace(&mut self.iso_spaces, vec![blend.1]);
        let result = body(self);
        self.iso_kinds = saved_kinds;
        self.iso_spaces = saved_spaces;
        let end = u32::try_from(self.items.len())
            .map_err(|_| RenderError::Render(format!("filter {} scope is too large", id.raw())))?;
        self.items[push] = Item::PushFilter {
            end,
            apron,
            space: blend.1,
        };
        self.items.push(Item::PopFilter {
            filter: (filter, params),
            opacity,
            blend,
            clip,
        });
        result
    }

    /// A layer: push its transform, then clip, then isolate for opacity,
    /// then content followed by children. The clip applies in `transform`
    /// space; content and children draw in `content_transform` space,
    /// which is where `scroll_offset` bites.
    fn layer(
        &mut self,
        id: LayerId,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
    ) -> Result<(), RenderError> {
        let node = tree.layer(id);
        if self.projects(id, tree) {
            return self.projected_layer(id, node);
        }
        let local_root = self.local.is_some_and(|(root, _)| root == id);
        let (opacity, blend) = if local_root {
            // Outer opacity and blend apply when the image composes.
            (1.0, BlendMode::Normal)
        } else {
            (node.opacity, node.blend)
        };
        let backdrop = node.backdrop.as_ref().map(|sample| sample.group().raw());
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
        if let Some(gid) = backdrop {
            self.emit_capture(gid, id);
        }
        let outer = self.clip.clone();
        // A filtered member composites as a whole — its sample and its
        // filtered content — with the member's opacity and blend. When
        // neither is a no-op an outer member scope composites at
        // (opacity, blend) under `composite_clip` below; the sample draws
        // into it at full strength under the member clip, outside the
        // filter, beside a nested filter scope at opacity 1, `Normal`
        // blend over the items.
        let member_scope = backdrop
            .zip(node.filter)
            .filter(|_| opacity < 1.0 || blend != BlendMode::Normal);
        let result = self.with_clip(node.clip.as_ref(), |s| {
            s.transform = content_space;
            // The composite's clip: for destructive operators the
            // operator applies over the layer's effective clip, so the
            // combined clip is the bound; for every other mode a
            // transparent source leaves the destination unchanged, so
            // the clip in force before the layer's own clip suffices —
            // the layer clip's coverage is already on the content.
            let composite_clip = if crate::render::blend::is_destructive(blend) {
                s.clip.clone()
            } else {
                outer.clone()
            };
            match member_scope {
                Some((gid, filter)) => s.isolate(
                    opacity,
                    (blend, cherenkov::BlendSpace::Linear),
                    s.clip.clone(),
                    composite_clip,
                    |s| {
                        s.emit_backdrop_sample(gid, id, outer.as_ref());
                        let clip = s.clip.clone();
                        s.filter_isolate(
                            filter,
                            1.0,
                            (BlendMode::Normal, cherenkov::BlendSpace::Linear),
                            clip,
                            |s| s.layer_items(id, node, tree, caches),
                            Some(id),
                        )
                    },
                ),
                None => {
                    if let Some(filter) = node.filter {
                        // Opaque `Normal`-blended member (or no member at all):
                        // the sample lands in the enclosing plane before the
                        // isolation — the filter never covers it.
                        if let Some(gid) = backdrop {
                            s.emit_backdrop_sample(gid, id, outer.as_ref());
                        }
                        let clip = s.clip.clone();
                        s.filter_isolate(
                            filter,
                            opacity,
                            (blend, cherenkov::BlendSpace::Linear),
                            clip,
                            |s| s.layer_items(id, node, tree, caches),
                            Some(id),
                        )
                    } else if opacity < 1.0
                        || blend != BlendMode::Normal
                        // The root already renders into the surface target, and a
                        // local root into its image.
                        || (id != s.start(tree) && node.blends_within())
                    {
                        // An unfiltered member's sample is its plane's
                        // bottom-most content: the level's opacity and blend
                        // apply to it exactly as to the member's items, once.
                        s.isolate(
                            opacity,
                            (blend, cherenkov::BlendSpace::Linear),
                            s.clip.clone(),
                            composite_clip,
                            |s| {
                                if let Some(gid) = backdrop {
                                    s.emit_backdrop_sample(gid, id, outer.as_ref());
                                }
                                s.layer_items(id, node, tree, caches)
                            },
                        )
                    } else {
                        if let Some(gid) = backdrop {
                            // Nothing isolates: the member's content plane is
                            // the enclosing one.
                            s.emit_backdrop_sample(gid, id, outer.as_ref());
                        }
                        s.layer_items(id, node, tree, caches)
                    }
                }
            }
        });
        self.transform = saved;
        self.animating = saved_animating;
        result
    }

    /// Emits group `gid`'s capture when `id` is its first member in paint
    /// order and its region is non-empty.
    fn emit_capture(&mut self, gid: u64, id: LayerId) {
        if let Some(plan) = self.backdrops.get(&gid)
            && plan.first == id
            && plan.region.x0 < plan.region.x1
            && plan.region.y0 < plan.region.y1
        {
            let region = plan.region;
            let union_rows = plan.union_rows;
            let apron = plan.apron;
            let reach = plan.reach;
            let spec = plan.spec;
            let filter = plan.filter.clone();
            let displaced = plan.members.values().any(|member| member.reach > 0.0);
            let flatten = self.iso_kinds.iter().rev().take_while(|&&k| k).count();
            self.items.push(Item::Capture(Box::new(CaptureItem {
                group: gid,
                scale: spec.scale(),
                levels: spec.levels().get(),
                region,
                union: IRect {
                    x0: 0,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_possible_wrap,
                        reason = "union rows lie inside the surface"
                    )]
                    y0: union_rows.0 as i32,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_possible_wrap,
                        reason = "surface width is small"
                    )]
                    x1: self.width as i32,
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_possible_wrap,
                        reason = "union rows lie inside the surface"
                    )]
                    y1: union_rows.1 as i32,
                },
                apron,
                reach,
                displaced,
                filter,
                flatten,
            })));
        }
    }

    /// A projective layer composes its completed local image: the image
    /// already carries the layer's content, clip and filter, so the
    /// projected sample is drawn under the ancestor clip only, and the
    /// layer's opacity and blend apply once. A destructive blend keeps its
    /// operator domain: the ancestor clip intersected with the projected
    /// layer clip, never the image's alpha.
    fn projected_layer(
        &mut self,
        id: LayerId,
        node: &cherenkov::LayerNode,
    ) -> Result<(), RenderError> {
        let Some(placed) = self.projected.get(&id).cloned() else {
            // No area this frame: behind the viewer, edge-on or off the
            // raster.
            return Ok(());
        };
        let outer = self.clip.clone();
        if node.blend == BlendMode::Normal {
            self.items.push(Item::Project(Box::new(ProjectItem {
                placed,
                opacity: node.opacity,
                clip: outer,
            })));
            return Ok(());
        }
        let composite_clip = if crate::render::blend::is_destructive(node.blend) {
            let clip = node
                .clip
                .as_ref()
                .expect("a planned projective layer has a clip");
            #[expect(clippy::cast_precision_loss, reason = "raster sizes fit f64")]
            let viewport =
                Rect::new(0.0, 0.0, self.width as f64, self.height as f64).inflate(1.0, 1.0);
            let (device, rule) = cherenkov::lowering::projective::project_clip(
                &placed.to_parent,
                clip,
                FLATTEN_TOL / placed.density,
                viewport,
            )
            .ok_or_else(|| {
                RenderError::Render(format!("projective layer {id:?} clip has no outline"))
            })?;
            let saved = std::mem::replace(&mut self.transform, Affine::IDENTITY);
            let clip = self.make_clip(&ShapeData::Path {
                elements: device.elements().into(),
                rule,
            });
            self.transform = saved;
            // A projected clip with no area bounds the operator to nothing.
            let Some(clip) = clip else { return Ok(()) };
            Some(clip)
        } else {
            outer.clone()
        };
        self.isolate(
            node.opacity,
            (node.blend, cherenkov::BlendSpace::Linear),
            outer.clone(),
            composite_clip,
            |s| {
                s.items.push(Item::Project(Box::new(ProjectItem {
                    placed,
                    opacity: 1.0,
                    clip: outer,
                })));
                Ok(())
            },
        )
    }

    /// Content first, then children — the engine's layer ordering.
    fn layer_items(
        &mut self,
        id: LayerId,
        node: &cherenkov::LayerNode,
        tree: &SurfaceTree,
        caches: &mut FxHashMap<LayerId, ContentData>,
    ) -> Result<(), RenderError> {
        if let Some(content) = caches.get_mut(&id) {
            let (ops, emissions) = content.prepared();
            let changed = self.ops(ops, emissions, 0, ops.len())?;
            self.layers_composed += u32::from(changed);
        }
        for child in &node.children {
            self.layer(*child, tree, caches)?;
        }
        Ok(())
    }

    /// Assemble scopes around retained draw instances under the layer state.
    fn ops(
        &mut self,
        ops: &[Op],
        emissions: &mut [Realization<Emission>],
        mut i: usize,
        end: usize,
    ) -> Result<bool, RenderError> {
        let mut changed = false;
        while i < end {
            match &ops[i] {
                Op::BeginClip { local, shape, end } => {
                    let saved = self.transform;
                    self.transform = saved * *local;
                    let clip = self.cached_clip(shape, &mut emissions[i]);
                    self.transform = saved;
                    if let Some(clip) = clip {
                        let outer = self.clip.replace(clip);
                        changed |= self.ops(ops, emissions, i + 1, *end as usize)?;
                        self.clip = outer;
                    }
                    self.transform = saved;
                    i = *end as usize;
                }
                Op::BeginIsolate {
                    filter,
                    opacity,
                    blend,
                    space,
                    end,
                } => {
                    let clip = self.clip.clone();
                    if let Some(filter) = filter {
                        self.filter_isolate(
                            *filter,
                            *opacity,
                            (*blend, *space),
                            clip,
                            |s| {
                                changed |= s.ops(ops, emissions, i + 1, *end as usize)?;
                                Ok(())
                            },
                            None,
                        )?;
                    } else {
                        self.isolate(*opacity, (*blend, *space), clip.clone(), clip, |s| {
                            changed |= s.ops(ops, emissions, i + 1, *end as usize)?;
                            Ok(())
                        })?;
                    }
                    i = *end as usize;
                }
                Op::End => unreachable!("paired scope consumes its end"),
                op => changed |= self.leaf(op, &mut emissions[i])?,
            }
            i += 1;
        }
        Ok(changed)
    }

    /// Key comparison uses Arc identity for retained masks, without scanning
    /// their coverage on a cache hit.
    fn placement_matches<T>(&self, data: &DeviceData<T>) -> bool {
        data.transform == self.transform
            && data.clip == self.clip
            && data.size == (self.width, self.height)
    }

    fn device_data<T>(&self, output: T) -> DeviceData<T> {
        DeviceData {
            transform: self.transform,
            clip: self.clip.clone(),
            size: (self.width, self.height),
            output,
        }
    }

    fn cached_clip(&self, shape: &ShapeData, cache: &mut Realization<Emission>) -> Option<ClipRef> {
        if cache.valid
            && let Some(Emission::Clip(data)) = &cache.data
            && self.placement_matches(data)
        {
            return data.output.clone();
        }
        let clip = self.make_clip(shape);
        cache.data = Some(Emission::Clip(self.device_data(clip.clone())));
        cache.valid = true;
        clip
    }

    /// Reuse coverage until either its source command or device placement changes.
    fn leaf(&mut self, op: &Op, cache: &mut Realization<Emission>) -> Result<bool, RenderError> {
        if cache.valid
            && let Some(Emission::Draw(data)) = &cache.data
            && self.placement_matches(data)
        {
            self.items.extend_from_slice(&data.output);
            return Ok(false);
        }
        let mut items = match cache.data.take() {
            Some(Emission::Draw(data)) => data.output,
            None => Vec::new(),
            Some(Emission::Clip(_)) => unreachable!("structural changes replace the cache layout"),
        };
        items.clear();
        let mut compose = Lowering::new(
            &mut items,
            (0, 0),
            None,
            FrameId::new(0),
            &mut *self.fonts,
            self.bitmap_fonts,
            &mut *self.bitmap_cache,
        );
        compose.width = self.width;
        compose.height = self.height;
        compose.transform = self.transform;
        compose.clip = self.clip.clone();
        compose.realize(op)?;
        self.glyphs.append(&mut compose.glyphs);
        self.glyphs_rasterized += compose.glyphs_rasterized;
        self.items.extend_from_slice(&items);
        cache.data = Some(Emission::Draw(self.device_data(items)));
        cache.valid = true;
        Ok(true)
    }

    fn realize(&mut self, op: &Op) -> Result<(), RenderError> {
        match op {
            Op::Fill {
                local,
                shape,
                paint,
            } => {
                self.transform *= *local;
                self.fill(shape, paint);
                Ok(())
            }
            Op::Stroke {
                local,
                shape,
                stroke,
                paint,
            } => {
                self.transform *= *local;
                self.stroke(shape, stroke, paint);
                Ok(())
            }
            Op::Shadow {
                local,
                shape,
                shadow,
            } => {
                self.transform *= *local;
                self.shadow(shape, shadow)
            }
            Op::Glyphs { local, run, paint } => {
                self.transform *= *local;
                self.glyph_run(run, paint)
            }
            Op::BitmapGlyph {
                local,
                font,
                glyph,
                origin,
                size,
            } => {
                self.transform *= *local;
                self.bitmap_glyph(*font, *glyph, *origin, *size)
            }
            _ => unreachable!("scope is composed, never realized as a leaf"),
        }
    }

    /// `Fill`: a flattened polygon of edges in device space.
    fn fill(&mut self, shape: &ShapeData, paint: &PaintData) {
        let sm = sigma_max(self.transform).max(1e-12);
        let tol_u = FLATTEN_TOL / sm;
        let Some((path, rule)) = shape_outline(shape, tol_u) else {
            return;
        };
        let edges = flatten_edges(self.transform * path, FLATTEN_TOL);
        let (edges, rule) = resolve_edges(edges, rule);
        if edges.is_empty() {
            return;
        }
        let paint = paint.transformed(self.transform.inverse());
        let bbox = bbox_of(&edges, self.width, self.height);
        if bbox.x0 >= bbox.x1 || bbox.y0 >= bbox.y1 {
            return;
        }
        self.items.push(Item::Draw {
            edges: edges.into(),
            bbox,
            rule,
            paint,
            clip: self.clip.clone(),
        });
    }

    /// `Stroke`: `kurbo::stroke` the content-space path, transform,
    /// flatten and fill non-zero — dashes included, like the oracle.
    fn stroke(&mut self, shape: &ShapeData, stroke: &kurbo::Stroke, paint: &PaintData) {
        let sm = sigma_max(self.transform).max(1e-12);
        let tol_u = FLATTEN_TOL / sm;
        let path = match shape {
            ShapeData::Line(l) => {
                let mut p = BezPath::new();
                p.move_to(l.p0);
                p.line_to(l.p1);
                p
            }
            shape => {
                let Some((path, _)) = shape_outline(shape, tol_u) else {
                    return;
                };
                path
            }
        };
        let outline = kurbo::stroke(path, stroke, &kurbo::StrokeOpts::default(), tol_u);
        let edges = flatten_edges(self.transform * outline, FLATTEN_TOL);
        let (edges, _) = resolve_edges(edges, FillRule::NonZero);
        if edges.is_empty() {
            return;
        }
        let paint = paint.transformed(self.transform.inverse());
        let bbox = bbox_of(&edges, self.width, self.height);
        self.items.push(Item::Draw {
            edges: edges.into(),
            bbox,
            rule: FillRule::NonZero,
            paint,
            clip: self.clip.clone(),
        });
    }

    /// `Shadow`: a Gaussian-blurred rounded box in device space.
    ///
    /// Only shapes whose corners are circular under the transform —
    /// `Rect`, `RoundedRect`, `Circle` — under an axis-aligned
    /// transform; other silhouettes use coverage convolution.
    /// Radii are handled per corner (the closed form works per corner).
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::many_single_char_names,
        reason = "shadow geometry is f32 and surface sizes fit i32"
    )]
    fn shadow(&mut self, shape: &ShapeData, shadow: &cherenkov::Shadow) -> Result<(), RenderError> {
        cherenkov::lowering::shadow::check_sigma(shadow.sigma)?;
        // The shape as a centred rect plus per-corner radii, in content
        // space.
        let (rect, radii) = match shape {
            ShapeData::Rect(r) => (*r, [0.0; 4]),
            ShapeData::RoundedRect(rr) => {
                let r = rr.rect();
                let limit = r.width().min(r.height()) / 2.0;
                let radii = rr.radii();
                (
                    r,
                    [
                        radii.top_left.clamp(0.0, limit),
                        radii.top_right.clamp(0.0, limit),
                        radii.bottom_right.clamp(0.0, limit),
                        radii.bottom_left.clamp(0.0, limit),
                    ],
                )
            }
            ShapeData::Circle(c) => (
                Rect::from_center_size(c.center, (c.radius * 2.0, c.radius * 2.0)),
                [c.radius; 4],
            ),
            ShapeData::Line(_) => return Err(RenderError::Unsupported(names::SHADOW)),
            _ => return self.silhouette(shape, shadow),
        };
        if !axis_aligned(self.transform) {
            return self.silhouette(shape, shadow);
        }
        let [a, b, c, d, _, _] = self.transform.as_coeffs();
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        let smax = sx.max(sy).max(1e-12);
        // The device-space box: transform the rect, offset by the linear
        // part applied to the shadow offset.
        let dr = device_rect(self.transform, rect);
        let (cx, cy) = (
            c.mul_add(
                shadow.offset.y,
                a.mul_add(shadow.offset.x, dr.x0.midpoint(dr.x1)),
            ),
            d.mul_add(
                shadow.offset.y,
                b.mul_add(shadow.offset.x, dr.y0.midpoint(dr.y1)),
            ),
        );
        let spread = shadow.spread * smax;
        let (hx, hy) = (
            (dr.width() / 2.0 + spread).max(0.0),
            (dr.height() / 2.0 + spread).max(0.0),
        );
        let limit = hx.min(hy);
        let radii = radii.map(|r| {
            if r > 0.0 {
                r.mul_add(smax, spread).clamp(0.0, limit)
            } else {
                0.0
            }
        });
        let sigma_eff =
            ((shadow.sigma * smax).mul_add(shadow.sigma * smax, 1.0 / 6.0)).sqrt() as f32;
        let margin = f64::from(sigma_eff).mul_add(3.0, 1.0);
        let (w, h) = (self.width as i32, self.height as i32);
        let bbox = IRect {
            x0: ((cx - hx - margin).floor() as i32).clamp(0, w),
            y0: ((cy - hy - margin).floor() as i32).clamp(0, h),
            x1: ((cx + hx + margin).ceil() as i32).clamp(0, w),
            y1: ((cy + hy + margin).ceil() as i32).clamp(0, h),
        };
        if bbox.x0 >= bbox.x1 || bbox.y0 >= bbox.y1 {
            return Ok(());
        }
        let [r, g, bl, al] = shadow.color.components;
        self.items.push(Item::Shadow {
            rbox: [cx as f32, cy as f32, hx as f32, hy as f32],
            radii: radii.map(|r| r as f32),
            sigma_eff,
            color: [r * al, g * al, bl * al, al],
            bbox,
            clip: self.clip.clone(),
        });
        Ok(())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "glyph placement is represented at raster precision"
    )]
    fn bitmap_glyph(
        &mut self,
        font_id: u64,
        glyph_id: u32,
        origin: [f32; 2],
        size: f32,
    ) -> Result<(), RenderError> {
        let bitmap_font = self
            .bitmap_fonts
            .get(&font_id)
            .ok_or_else(|| RenderError::Font(format!("unregistered bitmap font {font_id}")))?;
        let device_ppem = (f64::from(size) * sigma_max(self.transform)) as f32;
        let strike = bitmap_font.select(device_ppem);
        let key = super::bitmap::BitmapKey {
            font: font_id,
            strike,
            glyph: glyph_id,
        };
        let bitmap = if let Some(bitmap) = self.bitmap_cache.get(&key) {
            bitmap
        } else {
            let font = self
                .fonts
                .get(&font_id)
                .ok_or_else(|| RenderError::Font(format!("unregistered font {font_id}")))?;
            let Some(decoded) = super::bitmap::decode(
                &font.data.data,
                font.data.index,
                bitmap_font,
                strike,
                glyph_id,
            )?
            else {
                return Ok(());
            };
            let bitmap = super::bitmap::cpu_bitmap(decoded)?;
            self.bitmap_cache.insert(key, bitmap)?;
            self.glyphs_rasterized += 1;
            self.bitmap_cache
                .get(&key)
                .expect("inserted bitmap is present in the cache")
        };
        let rect = Rect::new(
            f64::from(size).mul_add(bitmap.em.x0, f64::from(origin[0])),
            f64::from(size).mul_add(bitmap.em.y0, f64::from(origin[1])),
            f64::from(size).mul_add(bitmap.em.x1, f64::from(origin[0])),
            f64::from(size).mul_add(bitmap.em.y1, f64::from(origin[1])),
        );
        let image_transform = Affine::translate((rect.x0, rect.y0))
            * Affine::scale_non_uniform(
                rect.width() / f64::from(bitmap.image.width),
                rect.height() / f64::from(bitmap.image.height),
            );
        let paint = PaintData::bitmap(Arc::clone(&bitmap.image), image_transform);
        self.fill(&ShapeData::Rect(rect), &paint);
        Ok(())
    }

    /// `Glyphs`: one mask request per positioned glyph. Font lookup and
    /// mask rasterization happen on the render thread before the band
    /// pass; the slot is filled by then.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::many_single_char_names,
        reason = "glyph device coordinates fit i32 on a real surface"
    )]
    fn glyph_run(&mut self, run: &GlyphRun, paint: &PaintData) -> Result<(), RenderError> {
        if matches!(run.style, GlyphStyle::Stroke(_)) {
            return Err(RenderError::Unsupported(names::GLYPH_STROKE));
        }
        let paint = paint.transformed(self.transform.inverse());
        let [a, b, c, d, ..] = self.transform.as_coeffs();
        let matrix = [a as f32, b as f32, c as f32, d as f32];
        let coords = run.coords.clone();
        for glyph in run.glyphs.iter() {
            if glyph.transform.is_some() {
                return Err(RenderError::Unsupported("glyph-transform"));
            }
            let o = self.transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
            let (ix, iy) = (o.x.floor(), o.y.floor());
            let (fx, fy) = (o.x - ix, o.y - iy);
            let subpixel = (fx as f32, fy as f32);
            let key = crate::render::glyph::glyph_key(run, glyph.id, subpixel, self.transform);
            let slot: crate::render::glyph::GlyphSlot =
                std::sync::Arc::new(std::sync::OnceLock::new());
            self.glyphs.push(GlyphReq {
                key,
                font: run.font.raw(),
                glyph_id: glyph.id,
                size: run.size,
                subpixel,
                matrix,
                coords: coords.clone(),
                slot: slot.clone(),
            });
            self.items.push(Item::Glyph {
                slot,
                x: ix as i32,
                y: iy as i32,
                paint: paint.clone(),
                clip: self.clip.clone(),
            });
        }
        Ok(())
    }
}
/// Heap residency of completed silhouette coverage owned by this content.
pub fn silhouette_bytes(content: &ContentData) -> u64 {
    content
        .realizations()
        .iter()
        .filter_map(|entry| entry.data.as_ref())
        .map(|emission| {
            let Emission::Draw(data) = emission else {
                return 0;
            };
            data.output
                .iter()
                .filter_map(|item| {
                    let Item::Silhouette { slot, .. } = item else {
                        return None;
                    };
                    slot.get().map(|mask| {
                        u64::try_from(mask.cov.capacity() * size_of::<f32>()).unwrap_or(u64::MAX)
                    })
                })
                .sum::<u64>()
        })
        .sum()
}
