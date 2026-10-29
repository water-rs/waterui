//! The glyph atlas and glyph-run lowering.

use std::collections::hash_map::DefaultHasher;
use std::hash::{BuildHasher, Hash, Hasher};
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use kurbo::{Affine, PathEl, Vec2};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::F2Dot14;

use crate::render::raster::{self, Raster};
use cherenkov::RenderError;

/// Initial atlas edge length.
const ATLAS_START: u32 = 1024;
/// Largest atlas edge length.
const ATLAS_MAX: u32 = 8192;
/// Texels of padding around each cell.
const PAD: u32 = 1;

/// Registered font data: the file bytes and collection index.
pub struct FontData {
    pub data: Arc<[u8]>,
    pub index: u32,
    pub units_per_em: u16,
    pub default_bounds: kurbo::Rect,
    /// Validated once at registration; plain runs do not reparse font tables.
    pub has_colr: bool,
    /// Resolved once at registration like `has_colr`: `bitmap` is `Some`.
    pub has_bitmap: bool,
    /// Bitmap strike sizes validated at registration.
    pub bitmap: Option<Arc<super::bitmap::BitmapFont>>,
    /// Built font-space `COLRv1` pictures, per `(glyph id, coords hash,
    /// paint hash)` — content is size-independent, so it is keyed without
    /// the placement. Interior mutability, not shared: parallel lowering
    /// works on a per-thread snapshot.
    pub colr: std::cell::RefCell<FxHashMap<(u32, u64, u64), cherenkov::Picture>>,
    /// Font-space bounds for glyphs used at non-default variation locations.
    bounds: std::cell::RefCell<Vec<InstanceBounds>>,
}

#[derive(Clone)]
struct InstanceBounds {
    coords: Arc<[i16]>,
    glyphs: Vec<(u32, Option<kurbo::Rect>)>,
}

/// A font validated on the caller thread by `Renderer::prepare_font`:
/// everything registration needs, so `add_font` cannot fail.
#[derive(Debug)]
pub struct PreparedFont {
    pub data: Arc<[u8]>,
    pub index: u32,
    pub units_per_em: u16,
    pub default_bounds: kurbo::Rect,
    pub has_colr: bool,
    pub bitmap: Option<Arc<super::bitmap::BitmapFont>>,
}

impl From<PreparedFont> for FontData {
    fn from(font: PreparedFont) -> Self {
        Self {
            data: font.data,
            index: font.index,
            units_per_em: font.units_per_em,
            default_bounds: font.default_bounds,
            has_colr: font.has_colr,
            has_bitmap: font.bitmap.is_some(),
            bitmap: font.bitmap,
            colr: std::cell::RefCell::new(FxHashMap::default()),
            bounds: std::cell::RefCell::default(),
        }
    }
}

impl FontData {
    /// A per-thread copy: shares the font bytes, clones the COLR cache.
    /// Workers each own one so `colr` can stay a plain `RefCell`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn snapshot(&self) -> Self {
        Self {
            data: self.data.clone(),
            index: self.index,
            units_per_em: self.units_per_em,
            default_bounds: self.default_bounds,
            has_colr: self.has_colr,
            has_bitmap: self.has_bitmap,
            bitmap: self.bitmap.clone(),
            colr: std::cell::RefCell::new(self.colr.borrow().clone()),
            bounds: std::cell::RefCell::new(self.bounds.borrow().clone()),
        }
    }

    pub fn bounds_bytes(&self) -> u64 {
        let instances = self.bounds.borrow();
        (size_of_val(&self.units_per_em)
            + size_of_val(&self.default_bounds)
            + instances.capacity() * size_of::<InstanceBounds>()
            + instances
                .iter()
                .map(|instance| {
                    size_of_val(instance.coords.as_ref())
                        + instance.glyphs.capacity() * size_of::<(u32, Option<kurbo::Rect>)>()
                })
                .sum::<usize>()) as u64
    }

    pub fn clear_bounds(&self) {
        *self.bounds.borrow_mut() = Vec::new();
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn merge_bounds(&self, other: Self) {
        let mut instances = self.bounds.borrow_mut();
        for incoming in other.bounds.into_inner() {
            if let Some(existing) = instances.iter_mut().find(|i| i.coords == incoming.coords) {
                for (id, bounds) in incoming.glyphs {
                    if let Err(index) = existing.glyphs.binary_search_by_key(&id, |entry| entry.0) {
                        existing.glyphs.insert(index, (id, bounds));
                    }
                }
            } else {
                instances.push(incoming);
            }
        }
    }
}

/// A glyph cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphKey {
    /// The engine font id.
    font: u64,
    /// The glyph index.
    glyph: u32,
    /// `(size * 64).round()` — 1/64th-pixel size granularity.
    size_bits: u32,
    /// Exact subpixel position: `f32` bits of `fx` | `f32` bits of `fy`.
    subpixel: u64,
    /// f32 bits of the device transform's 2x2.
    matrix: [u32; 4],
    /// Hash of the run's variation coordinates.
    coords_hash: u64,
}

// Match the derived field hash byte-for-byte, but submit one contiguous write
// to the atlas hasher. The array's length prefix is part of that original hash.
impl Hash for GlyphKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        let matrix = 24 + size_of::<usize>();
        let coords = matrix + 16;
        let mut bytes = [0; 48 + size_of::<usize>()];
        bytes[..8].copy_from_slice(&self.font.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.glyph.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.size_bits.to_ne_bytes());
        bytes[16..24].copy_from_slice(&self.subpixel.to_ne_bytes());
        bytes[24..matrix].copy_from_slice(&4usize.to_ne_bytes());
        for (dst, value) in bytes[matrix..coords]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.matrix)
        {
            *dst = value.to_ne_bytes();
        }
        bytes[coords..].copy_from_slice(&self.coords_hash.to_ne_bytes());
        state.write(&bytes);
    }
}

impl GlyphKey {
    /// Change just the glyph and subpixel position; run identity stays exact.
    pub fn at(mut self, glyph: u32, subpixel: (f32, f32)) -> Self {
        self.glyph = glyph;
        self.subpixel = u64::from(subpixel.0.to_bits()) | (u64::from(subpixel.1.to_bits()) << 32);
        self
    }
}

/// An atlas cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// Texel origin.
    pub x: u16,
    /// Texel origin.
    pub y: u16,
    /// Cell size in texels; 0 for an empty outline.
    pub w: u16,
    /// Cell size in texels.
    pub h: u16,
    /// Offset of the cell's left edge from the glyph's integer device origin.
    pub left: i32,
    /// Offset of the cell's top edge from the glyph's integer device origin.
    pub top: i32,
}

/// A cached glyph: the cell the lowering reads and the shelf the cell
/// lives on. A glyph with no outline is cached too, so its miss is not
/// retried every frame, but it owns no cell and so names no shelf —
/// `shelf` is `None` — and it sits in neither `live` nor `slot_keys`
/// (#2327).
#[derive(Clone, Copy)]
struct CachedGlyph {
    entry: Entry,
    shelf: Option<u32>,
}

/// One atlas cell emitted by the path rasterizer: a device-space quad
/// sampling `w` × `h` coverage texels at `(x, y)`.
#[derive(Clone, Copy, Debug)]
pub struct PathCell {
    /// Device-space quad `(x0, y0, x1, y1)`.
    pub rect: [f32; 4],
    /// Atlas texel origin.
    pub x: u16,
    /// Atlas texel origin.
    pub y: u16,
    /// Shelf the cell lives on, for [`Atlas::begin_commit`] touches.
    pub slot: u32,
    /// Full columns inside this cell: `start | end << 16`, relative to x0.
    /// Zero means that the entire cell needs sampled coverage.
    pub interior: u32,
}

/// What a cached path draw replays: full-coverage spans and atlas cells,
/// in device space shifted by the cache's stored offset.
#[derive(Clone, Debug, Default)]
pub struct PathEmit {
    /// Device rectangles of contiguous full-coverage columns.
    pub spans: Vec<[f32; 4]>,
    /// Partial-coverage atlas cells.
    pub cells: Vec<PathCell>,
    /// Shelves the cells live on — deduplicated at admission so a hit
    /// pins them without a live-map lookup. A range into the atlas's
    /// `emit_slots` arena, which outlives this record so a replay pin
    /// stays meaningful after an eviction.
    pub slots: Range<usize>,
    /// The retained-geometry digest the lowering that emitted this
    /// record assigned: a coverage hit returns it for the content's
    /// ownership marks without rebuilding or hashing the key. Identity
    /// only — geometry reuse still compares the full `GeometryKey`
    /// (water-rs/waterui#3306). `None` for records built outside the
    /// retained route.
    pub geometry: Option<NonZeroU64>,
}

impl PathEmit {
    /// This emission shifted by `(dx, dy)` device pixels.
    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "device coords are f32")]
    pub fn translated(&self, dx: f64, dy: f64) -> Self {
        let shift = |r: [f32; 4]| {
            [
                f64::from(r[0]) + dx,
                f64::from(r[1]) + dy,
                f64::from(r[2]) + dx,
                f64::from(r[3]) + dy,
            ]
        };
        Self {
            spans: self
                .spans
                .iter()
                .map(|r| shift(*r).map(|v| v as f32))
                .collect(),
            cells: self
                .cells
                .iter()
                .map(|c| PathCell {
                    rect: shift(c.rect).map(|v| v as f32),
                    x: c.x,
                    y: c.y,
                    slot: c.slot,
                    interior: c.interior,
                })
                .collect(),
            slots: self.slots.clone(),
            geometry: self.geometry,
        }
    }
}

/// A coverage mask in the atlas: a path clip rasterized into one cell.
#[derive(Clone, Copy, Debug)]
pub struct MaskCell {
    /// Device-space origin of the mask.
    pub device: [f32; 2],
    /// Atlas texel origin of the cell.
    pub atlas: [f32; 2],
    /// Cell size in pixels.
    pub size: [f32; 2],
    /// The mask's device-space bounding rect `(x0, y0, x1, y1)` — the
    /// clip's analytic shape shrinks to it.
    pub rect: [f32; 4],
    /// Shelf the cell lives on, for [`Atlas::begin_commit`] touches.
    pub slot: u32,
}

impl MaskCell {
    /// This mask shifted by `(dx, dy)` device pixels; the atlas texels
    /// do not move.
    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "device coords are f32")]
    pub fn translated(&self, dx: f64, dy: f64) -> Self {
        Self {
            device: [
                (f64::from(self.device[0]) + dx) as f32,
                (f64::from(self.device[1]) + dy) as f32,
            ],
            atlas: self.atlas,
            size: self.size,
            rect: [
                (f64::from(self.rect[0]) + dx) as f32,
                (f64::from(self.rect[1]) + dy) as f32,
                (f64::from(self.rect[2]) + dx) as f32,
                (f64::from(self.rect[3]) + dy) as f32,
            ],
            slot: self.slot,
        }
    }
}

/// Masks larger than this many texels — or that the atlas cannot hold —
/// get their own `R8Unorm` texture instead of an atlas cell.
pub const MASK_TEXTURE_TEXELS: u64 = 256 * 256;

/// A path-clip mask on its own texture, bound at group-1 binding 3.
struct MaskTexture {
    /// The mask data; `cell.atlas` stays `[0, 0]` (unused).
    cell: MaskCell,
    /// Retained so a native executor can transition the resource its
    /// descriptor names, not just bind the view.
    #[cfg_attr(
        not(all(unix, not(target_vendor = "apple"))),
        expect(
            dead_code,
            reason = "only the native Vulkan executor transitions these resources; Metal binds the views"
        )
    )]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// `w` × `h`, the texture's GPU bytes.
    bytes: u64,
}

/// One shelf of the packer: a row of cells sharing a height class. A
/// shelf is also the eviction slot: reclaiming frees the whole row.
#[derive(Clone)]
struct Shelf {
    y: u32,
    h: u32,
    x: u32,
    /// `x` at the start of the current commit — the upload region's left
    /// edge; `0` for a shelf opened or reclaimed inside the commit.
    base: u32,
    /// Commit ordinal of the last hit or allocation (`Atlas::tick`).
    last_used: u64,
    /// Commits this shelf has served since its (re)admission; `0` marks
    /// the probationary segment of the eviction order.
    hits: u32,
    /// A dead shelf's band sits in `vacant` until reclaimed.
    live: bool,
    /// Band epoch — bumped when a band is admitted on this slot and when
    /// it dies, so an emission's stored `(slot, epoch)` reference fails
    /// once the band it sampled is gone (#119).
    epoch: u64,
}

/// The shelf layout plus reclaim bookkeeping — the part of [`Atlas`] a
/// plan dry-run clones to simulate placements without touching the
/// live caches.
///
/// The atlas is paged (#211): the texture is `size` × `size * pages`,
/// shelf `y` coordinates are global (`page * size + local`), and `tops`
/// carries one virgin watermark per page. Bands never span a page
/// boundary, so dead-band merges and frontier recycling are per-page.
#[derive(Clone)]
struct Layout {
    shelves: Vec<Shelf>,
    /// Dead shelf indices, their bands reclaimable by a same-or-smaller
    /// height class.
    vacant: Vec<u32>,
    /// Virgin-space watermark per page: page `p`'s bands are laid
    /// consecutively from `y = p * size` up to `p * size + tops[p]`.
    tops: Vec<u32>,
    /// Next band epoch — monotonic, so a band's epoch changes whenever
    /// the band occupying a slot changes (#119).
    next_epoch: u64,
}

/// Approximate per-admission CPU bookkeeping the live map, slot keys and
/// the wider [`Entry`] add — folded into `cpu_bytes` so the cache pays
/// for the retained structure it needs (#119).
const LIVE_ENTRY_BYTES: u64 = 120;

/// CPU bytes one cached `PathEmit` costs: its heap lists, the inline
/// retained-geometry digest, and the shared per-admission bookkeeping
/// — the one formula admission, eviction and relocation account by.
fn path_entry_bytes(emit: &PathEmit) -> u64 {
    (emit.spans.len() * 16
        + emit.cells.len() * size_of::<PathCell>()
        + emit.slots.len() * 4
        + size_of::<Option<NonZeroU64>>()) as u64
        + LIVE_ENTRY_BYTES
}

/// A live admission's bookkeeping: which cache map holds the entry and,
/// for glyphs, the full key needed to remove it.
struct LiveEntry {
    kind: LiveKind,
}

/// Which cache a live key indexes.
enum LiveKind {
    /// `map`, keyed by the full `GlyphKey`.
    Glyph(GlyphKey),
    /// `paths`, keyed by the live key itself.
    Path,
    /// `masks`, keyed by the live key itself.
    Mask,
}

/// The cache entry a migrated cell belongs to, for rebuilding origins
/// after [`Atlas::relocate`] moves it.
#[derive(Clone, Copy)]
enum CellOwner {
    /// `map`, by full key.
    Glyph(GlyphKey),
    /// `paths`, by key and the cell's index in `PathEmit::cells`.
    Path(u64, usize),
    /// `masks`, by key.
    Mask(u64),
}

/// A preflighted cell migration: the owner whose record the new
/// placement patches, old and new exact texel origins, the new shelf
/// and the copied extent.
struct CellMove {
    owner: CellOwner,
    src: (u32, u32),
    dst: (u32, u32),
    dst_slot: u32,
    w: u32,
    h: u32,
}

/// Where a relocated cell's coverage landed: its exact new texel
/// origin.
#[derive(Clone, Copy, Debug)]
pub struct CellLocation {
    /// New atlas texel origin.
    pub x: u32,
    /// New atlas texel origin.
    pub y: u32,
}

/// The ephemeral record [`Atlas::relocate`] hands the caller: where each
/// migrated cell's texels went, plus the old layout's band epochs and
/// live bits needed to check whether a retained emission's `(slot,
/// epoch)` references were still valid when the repack ran. New shelves'
/// epochs continue the old layout's counter, so no old reference can
/// alias the new layout.
pub struct Relocation {
    /// `(old x, old y)` → new placement, one per migrated cell.
    cells: rustc_hash::FxHashMap<(u32, u32), CellLocation>,
    /// `(epoch, live)` per old shelf slot.
    old_shelves: Vec<(u64, bool)>,
}

impl Relocation {
    /// The new placement of the cell whose old texel origin was
    /// `(x, y)`; `None` when the repack dropped it.
    pub fn cell(&self, x: u32, y: u32) -> Option<CellLocation> {
        self.cells.get(&(x, y)).copied()
    }

    /// Whether the old layout's `slot` held a live band at `epoch` when
    /// the repack ran — the validity check for references recorded
    /// before it.
    pub fn old_ref_live(&self, slot: u32, epoch: u64) -> bool {
        usize::try_from(slot)
            .ok()
            .and_then(|i| self.old_shelves.get(i))
            .is_some_and(|&(ep, live)| live && ep == epoch)
    }

    /// Cells the repack carried over.
    pub fn moved(&self) -> usize {
        self.cells.len()
    }
}

/// The `R8Unorm` coverage atlas.
pub struct Atlas {
    /// The atlas texture — `None` until the first committed cell writes
    /// it; an idle engine holds no coverage storage (#170).
    texture: Option<wgpu::Texture>,
    /// The 1×1 placeholder `view` binds until the first committed cell.
    /// Retained so a native executor can transition the resource its
    /// descriptor names, whether or not storage exists.
    #[cfg_attr(
        not(all(unix, not(target_vendor = "apple"))),
        expect(
            dead_code,
            reason = "only the native Vulkan executor transitions these resources; Metal binds the views"
        )
    )]
    placeholder: wgpu::Texture,
    /// The group-0 binding: the atlas view once allocated, otherwise a
    /// 1×1 `R8Unorm` placeholder — the minimal valid binding for a
    /// binding the frame's instances never sample (#170).
    view: wgpu::TextureView,
    /// Page edge: the texture is `size` × `size * pages`.
    size: u32,
    /// The largest atlas edge the GPU budget allows.
    cap: u32,
    /// `cap`-square pages the texture carries.
    pages: u32,
    /// Bumped whenever the texture is recreated, so stale bind groups are
    /// rebuilt.
    generation: u64,
    /// Cell placement: live and dead shelves plus the virgin watermark.
    layout: Layout,
    /// Live-key hashes per shelf, parallel to `layout.shelves`: the keys
    /// an eviction of that shelf must drop from the caches.
    slot_keys: Vec<rustc_hash::FxHashSet<u64>>,
    /// Live admissions by live key — the index evictions clean through.
    live: rustc_hash::FxHashMap<u64, LiveEntry>,
    /// Eviction counter; bumped per reclaimed shelf so emissions check
    /// their references once per evicting commit, not once per hit.
    clock: u64,
    /// Commit ordinal; marks `last_used` for pins and the eviction order.
    tick: u64,
    /// Whether `alloc` may evict cold shelves: set for the commit of a
    /// batch that did not fit (`AtlasPlan::Recycle`), off otherwise.
    evicting: bool,
    /// Bytes evicted this commit, drained by the caller for diagnostics.
    evicted: Vec<(u64, bool)>,
    map: rustc_hash::FxHashMap<GlyphKey, CachedGlyph>,
    /// Rasterized path emissions, keyed by content hash.
    paths: rustc_hash::FxHashMap<u64, PathEmit>,
    /// Slot lists the `PathEmit::slots` ranges address. Entries
    /// outlive their emission record so a lowered leaf's slot ranges
    /// stay readable through the commit that admits them; a range an
    /// evicted emission leaves is recycled through `slot_dead` →
    /// `slot_holes` rather than dropped, since same-commit readers
    /// may still address it (#119).
    emit_slots: Vec<u32>,
    /// `emit_slots` ranges an evicting commit may hand to a new
    /// emission — dead before this commit started.
    slot_holes: Vec<Range<usize>>,
    /// `emit_slots` ranges this commit's evictions released;
    /// promoted to `slot_holes` at the next `begin_commit`.
    slot_dead: Vec<Range<usize>>,
    /// Rasterized path-clip masks, keyed by content hash.
    masks: rustc_hash::FxHashMap<u64, MaskCell>,
    /// Path-clip masks too large for the atlas, on their own textures,
    /// keyed by content hash.
    mask_textures: rustc_hash::FxHashMap<u64, MaskTexture>,
    /// The largest edge the device accepts for a 2D texture.
    texture_limit: u32,
    /// GPU bytes held by `mask_textures`.
    mask_texture_bytes: u64,
    /// Bumped on every `mask_textures` change so stale bind groups are
    /// rebuilt.
    mask_texture_gen: u64,
    /// The GPU byte budget for `mask_textures` (`budget / 16`).
    mask_budget: u64,
    /// The GPU byte budget the atlas's pages may reach (`budget / 8`).
    page_budget: u64,
    /// Sum of cell texels, an approximation of the CPU cache size.
    cpu_bytes: u64,
    /// Upload assembly scratch for [`Self::upload_committed`]: the span
    /// buffer is rebuilt in place each commit instead of reallocated.
    upload_scratch: Vec<u8>,
    /// Per-plan scratch: the dedupe sets, pending cell list and cached
    /// cell list are rebuilt in place instead of allocated per call.
    dedupe_glyphs: rustc_hash::FxHashSet<GlyphKey>,
    dedupe_paths: rustc_hash::FxHashSet<u64>,
    dedupe_masks: rustc_hash::FxHashSet<u64>,
    plan_cells: Vec<(u32, u32)>,
    plan_all: Vec<(u32, u32)>,
    /// Per-shelf cell counts, scatter cursors and the flat cell order
    /// for [`Self::upload_committed`], rebuilt in place each commit.
    bucket_counts: Vec<u32>,
    bucket_next: Vec<u32>,
    bucket_order: Vec<u32>,
}

impl Atlas {
    /// A new `ATLAS_START` atlas, capped by the GPU byte budget (one texel
    /// per byte).
    ///
    /// No texture is allocated: coverage storage arrives at the first
    /// committed cell, and `view` binds a 1×1 placeholder until then
    /// (#170's "storage absent until the first atlas-backed primitive").
    pub fn new(device: &wgpu::Device, budget: u64) -> Self {
        let placeholder = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas placeholder"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = placeholder.create_view(&wgpu::TextureViewDescriptor::default());
        crate::diag::create(device, "glyph atlas placeholder", 1);
        let texture_limit = device.limits().max_texture_dimension_2d;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            reason = "atlas sizes are small"
        )]
        let cap = ATLAS_MAX
            .min(texture_limit)
            .min((budget as f64).sqrt() as u32)
            .max(ATLAS_START);
        Self {
            texture: None,
            placeholder,
            view,
            size: ATLAS_START,
            cap,
            pages: 1,
            generation: 0,
            layout: Layout {
                shelves: Vec::new(),
                vacant: Vec::new(),
                tops: vec![0],
                next_epoch: 0,
            },
            page_budget: budget / 8,
            slot_keys: Vec::new(),
            live: rustc_hash::FxHashMap::default(),
            clock: 0,
            tick: 0,
            evicting: false,
            evicted: Vec::new(),
            map: rustc_hash::FxHashMap::default(),
            paths: rustc_hash::FxHashMap::default(),
            emit_slots: Vec::new(),
            slot_holes: Vec::new(),
            slot_dead: Vec::new(),
            masks: rustc_hash::FxHashMap::default(),
            mask_textures: rustc_hash::FxHashMap::default(),
            texture_limit,
            mask_texture_bytes: 0,
            mask_texture_gen: 0,
            mask_budget: budget / 16,
            cpu_bytes: 0,
            upload_scratch: Vec::new(),
            dedupe_glyphs: rustc_hash::FxHashSet::default(),
            dedupe_paths: rustc_hash::FxHashSet::default(),
            dedupe_masks: rustc_hash::FxHashSet::default(),
            plan_cells: Vec::new(),
            bucket_counts: Vec::new(),
            bucket_next: Vec::new(),
            bucket_order: Vec::new(),
            plan_all: Vec::new(),
        }
    }

    /// Increments whenever the atlas texture is recreated.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    fn allocate(
        device: &wgpu::Device,
        size: u32,
        pages: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size * pages,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            // COPY_SRC: `relocate` copies live cells texture-to-texture
            // instead of re-rasterizing them (#3302).
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// The texture view bound in group 0 — the placeholder while no
    /// cell has committed.
    pub const fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The texture `view` names: the atlas storage once a cell has
    /// committed, the placeholder before that.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub fn texture(&self) -> &wgpu::Texture {
        self.texture.as_ref().unwrap_or(&self.placeholder)
    }

    /// Allocates the atlas texture at the first committed cell. The
    /// generation bump rebuilds group-0 bindings against the real view.
    fn ensure_storage(&mut self, device: &wgpu::Device) {
        if self.texture.is_some() {
            return;
        }
        let (texture, view) = Self::allocate(device, self.size, self.pages);
        self.texture = Some(texture);
        self.view = view;
        self.generation = self
            .generation
            .checked_add(1)
            .expect("atlas generation overflow");
        crate::diag::create(device, "glyph atlas", self.gpu_bytes());
    }

    /// Atlas byte size on the GPU — 0 while no cell has committed.
    pub fn gpu_bytes(&self) -> u64 {
        if self.texture.is_none() {
            return 0;
        }
        u64::from(self.size) * u64::from(self.size) * u64::from(self.pages)
    }

    /// Approximate CPU-side cache bytes.
    pub const fn cpu_bytes(&self) -> u64 {
        self.cpu_bytes
    }

    /// Drops every cached glyph of `font`. The freed texels stay claimed
    /// in the shelf layout until eviction or a grow reclaims them; path
    /// and mask cells are font-independent and stay. An outline-less
    /// glyph leaves only the map: it was never indexed or charged.
    pub fn remove_font(&mut self, font: u64) {
        let dropped: Vec<(u64, u64, usize)> = self
            .map
            .extract_if(|key, _| key.font == font)
            .filter_map(|(key, cached)| {
                cached.shelf.map(|slot| {
                    (
                        live_hash(&key),
                        u64::from(cached.entry.w) * u64::from(cached.entry.h),
                        usize::try_from(slot).expect("shelf count fits usize"),
                    )
                })
            })
            .collect();
        for (hk, bytes, slot) in dropped {
            self.live.remove(&hk);
            self.slot_keys[slot].remove(&hk);
            self.cpu_bytes = self.cpu_bytes.saturating_sub(bytes + LIVE_ENTRY_BYTES);
        }
    }

    /// Clears every entry without freeing the texture.
    pub fn clear(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("atlas generation overflow");
        self.map.clear();
        self.paths.clear();
        self.emit_slots.clear();
        self.slot_holes.clear();
        self.slot_dead.clear();
        self.masks.clear();
        self.layout.shelves.clear();
        self.layout.vacant.clear();
        self.layout.tops.clear();
        self.layout.tops.resize(self.pages as usize, 0);
        self.slot_keys.clear();
        self.live.clear();
        self.evicting = false;
        self.evicted.clear();
        self.cpu_bytes = 0;
    }

    /// A cached path emission.
    pub fn path(&self, key: u64) -> Option<&PathEmit> {
        self.paths.get(&key)
    }

    /// Caches a path emission.
    pub fn insert_path(&mut self, key: u64, emit: PathEmit) {
        self.cpu_bytes += path_entry_bytes(&emit);
        self.paths.insert(key, emit);
    }

    /// A cached path-clip mask.
    pub fn mask(&self, key: u64) -> Option<&MaskCell> {
        self.masks.get(&key)
    }

    /// Caches a path-clip mask.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "cell sizes are small positive floats"
    )]
    pub fn insert_mask(&mut self, key: u64, mask: MaskCell) {
        self.cpu_bytes += (mask.size[0] * mask.size[1]) as u64 + LIVE_ENTRY_BYTES;
        self.masks.insert(key, mask);
    }

    /// Uploads one commit batch: a single `write_texture` per
    /// contiguous newly allocated shelf region. Cells between the new
    /// ones keep their old texels — the regions cover only newly
    /// claimed spans — and no full-size CPU shadow of the atlas is
    /// kept to assemble them (#169 A3). Each shelf's `base` marks where
    /// the commit started writing.
    ///
    /// The batch moves in: its texel arenas are copied into GPU staging
    /// and dropped with the call, and the emptied container comes back
    /// for the next commit to refill — only its list capacities are
    /// retained (#2671).
    pub fn upload_committed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut cells: CellWrites,
    ) -> CellWrites {
        if cells.is_empty() {
            // A commit can stage an arena without placing a cell from it
            // (a failed lowering, an empty batch); it still ends here.
            cells.clear();
            return cells;
        }
        self.ensure_storage(device);
        let n = self.layout.shelves.len();
        self.bucket_counts.clear();
        self.bucket_counts.resize(n, 0);
        for cell in cells.cells() {
            self.bucket_counts[cell.shelf] += 1;
        }
        self.bucket_next.clear();
        let mut acc = 0u32;
        for &count in &self.bucket_counts {
            self.bucket_next.push(acc);
            acc += count;
        }
        self.bucket_order.clear();
        self.bucket_order.resize(cells.len(), 0);
        for (idx, cell) in cells.cells().iter().enumerate() {
            let slot = cell.shelf;
            self.bucket_order[self.bucket_next[slot] as usize] =
                u32::try_from(idx).expect("upload batch fits u32");
            self.bucket_next[slot] += 1;
        }
        for (i, shelf) in self.layout.shelves.iter().enumerate() {
            let x0 = shelf.base;
            if !shelf.live || x0 >= shelf.x {
                continue;
            }
            let (w, h) = (shelf.x - x0, shelf.h);
            // After the scatter `bucket_next` holds each bucket's end
            // offset; start = end - count.
            let end = self.bucket_next[i] as usize;
            let len = self.bucket_counts[i] as usize;
            let bucket = &self.bucket_order[end - len..end];
            // One cell that fills the whole new span uploads its own
            // texels without assembly.
            let single = match bucket {
                &[idx] => {
                    let cell = &cells.cells()[idx as usize];
                    (cell.x == x0 && cell.w == w && cell.h == h).then_some(cell)
                }
                _ => None,
            };
            let data: &[u8] = if let Some(cell) = single {
                crate::diag::atlas_cell(device, (cell.x, cell.y, cell.w, cell.h));
                cells.texels(cell)
            } else {
                self.upload_scratch.clear();
                self.upload_scratch.resize((w * h) as usize, 0);
                for &idx in bucket {
                    let cell = &cells.cells()[idx as usize];
                    let xo = (cell.x - x0) as usize;
                    let yo = (cell.y - shelf.y) as usize;
                    for (row, line) in cells.texels(cell).chunks_exact(cell.w as usize).enumerate()
                    {
                        let dst = (yo + row) * w as usize + xo;
                        self.upload_scratch[dst..dst + cell.w as usize].copy_from_slice(line);
                    }
                    crate::diag::atlas_cell(device, (cell.x, cell.y, cell.w, cell.h));
                }
                &self.upload_scratch
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: self.texture.as_ref().expect("atlas storage allocated"),
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: x0,
                        y: shelf.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            crate::diag::upload(
                device,
                "glyph atlas",
                u64::from(w) * u64::from(h),
                Some((x0, shelf.y, w, h)),
            );
        }
        cells.clear();
        cells
    }

    /// The placement decision a pending batch's dry run supports
    /// (#169 A3): whether the shelves take every cell as-is, or bounded
    /// eviction of untouched shelves places them (#119), or the atlas
    /// must resize to a different `(edge, pages)`, or the batch exceeds
    /// even the emptied atlas. The dry run simulates the same eviction the
    /// commit applies, so a `FitsEviction` verdict guarantees the
    /// evicting commit succeeds and `Resize` means the batch's cell set
    /// needs a different `(edge, pages)` — more pages than a peak frame
    /// left behind, or fewer once it packs below (#211). A batch that
    /// packs nowhere still grows when a cell needs a wider edge or the
    /// batch's shelf footprint fits a bigger shape — in-place eviction
    /// can never place a cell wider than the live edge (#234); only a
    /// batch exceeding the largest allowed shape reports exhaustion.
    pub fn plan(&mut self, rasters: &[&PendingRaster]) -> AtlasPlan {
        self.batch_cells(rasters);
        let mut probe = self.layout.clone();
        let mut evicted = false;
        let fits = self.plan_cells.iter().all(|&(w, h)| {
            loop {
                if alloc_on(&mut probe, self.size, self.tick, w, h).is_some() {
                    break true;
                }
                if evict_layout(&mut probe, self.size, self.tick).is_none() {
                    break false;
                }
                evicted = true;
            }
        });
        if fits {
            // A paged atlas outlives the frame that needed its pages:
            // once the live coverage packs onto fewer, release the rest
            // — the texture returns to the size the current content
            // needs instead of holding the peak (#211). Live = the
            // batch's pending cells plus every cached cell the repack
            // carries; the rest is dead weight it drops. The probe
            // packs exactly the cells a repacked commit places, so the
            // verdict transfers.
            if self.pages > 1 {
                self.live_cells();
                // Probe smallest texture first — so the released atlas
                // lands on the shape a fresh engine reaches for the same
                // content, not just any smaller area (#211).
                let area = u64::from(self.size) * u64::from(self.size) * u64::from(self.pages);
                let mut candidates: Vec<(u32, u32)> = Vec::new();
                let mut size = ATLAS_START;
                while size <= self.size {
                    if u64::from(size) * u64::from(size) < area {
                        candidates.push((size, 1));
                    }
                    size = size.saturating_mul(2);
                }
                for pages in 2..self.pages {
                    if u64::from(self.cap) * u64::from(self.cap) * u64::from(pages) < area {
                        candidates.push((self.cap, pages));
                    }
                }
                for (size, pages) in candidates {
                    let mut probe = Layout {
                        shelves: Vec::new(),
                        vacant: Vec::new(),
                        tops: vec![0; pages as usize],
                        next_epoch: 0,
                    };
                    if self
                        .plan_all
                        .iter()
                        .all(|&(w, h)| alloc_on(&mut probe, size, self.tick, w, h).is_some())
                    {
                        return AtlasPlan::Resize(size, pages);
                    }
                }
            }
            return if evicted {
                AtlasPlan::FitsEviction
            } else {
                AtlasPlan::Fits
            };
        }
        // Placement failed even with every untouchable shelf gone. The
        // repack keeps only the live set — the pending cells plus the
        // surviving cached cells — so the target probe packs exactly
        // those. Unreferenced cache would bloat — or break — it (#211).
        self.live_cells();
        let shapes = self.grow_shapes();
        for &(size, pages) in &shapes {
            let mut probe = Layout {
                shelves: Vec::new(),
                vacant: Vec::new(),
                tops: vec![0; pages as usize],
                next_epoch: 0,
            };
            if self
                .plan_all
                .iter()
                .all(|&(w, h)| alloc_on(&mut probe, size, self.tick, w, h).is_some())
            {
                return AtlasPlan::Resize(size, pages);
            }
        }
        // The batch packs at no shape the device allows — but eviction
        // on the live atlas only reclaims shelves, it cannot grow the
        // edge, so a cell wider than the live edge could never place
        // however much of the batch would fit at a bigger edge (#234).
        self.partial_grow().unwrap_or(AtlasPlan::Recycle)
    }

    /// The shape a batch that packs nowhere still grows to (#234): the
    /// smallest `(edge, pages)` whose capacity holds the batch's real
    /// shelf footprint — band-quantised heights plus per-cell padding,
    /// what the shelves actually consume — so the retried commit places
    /// what fits and reports whatever still overflows. `None` only when
    /// a cell needs an edge past the cap, the batch exceeds even the
    /// largest allowed shape, or the live shape is already that shape.
    fn partial_grow(&self) -> Option<AtlasPlan> {
        let need_edge = self.plan_all.iter().fold(ATLAS_START, |edge, &(w, h)| {
            edge.max((w + 2 * PAD).max(band(h)).next_power_of_two())
        });
        if need_edge > self.cap {
            return None;
        }
        let footprint: u64 = self
            .plan_all
            .iter()
            .map(|&(w, h)| u64::from(w + 2 * PAD) * u64::from(band(h)))
            .sum();
        let mut size = need_edge;
        loop {
            let page_texels = u64::from(size) * u64::from(size);
            let pages = u32::try_from(footprint.div_ceil(page_texels))
                .unwrap_or(u32::MAX)
                .max(1);
            if pages <= self.max_pages(size) {
                return (size != self.size || pages != self.pages)
                    .then_some(AtlasPlan::Resize(size, pages));
            }
            if size == self.cap {
                return None;
            }
            size = (size * 2).min(self.cap);
        }
    }

    /// Every `(edge, pages)` shape larger than the live atlas, cheapest
    /// first: edge doublings to the cap, each with every page count the
    /// device and budget allow, plus the extra pages the live edge
    /// could still grow. The batch lands on the smallest texture that
    /// packs it — a bigger edge before another page only when it costs
    /// the same capacity or less.
    fn grow_shapes(&self) -> Vec<(u32, u32)> {
        let mut shapes = Vec::new();
        let mut size = self.size;
        for pages in self.pages + 1..=self.max_pages(self.size) {
            shapes.push((self.size, pages));
        }
        while size < self.cap {
            size = (size * 2).min(self.cap);
            for pages in 1..=self.max_pages(size) {
                shapes.push((size, pages));
            }
        }
        shapes.sort_by_key(|&(size, pages)| u64::from(size) * u64::from(size) * u64::from(pages));
        shapes
    }

    /// `(w, h)` of every cell a repacked commit would place, in
    /// placement order — surviving cached glyphs, paths, then masks, the
    /// order [`Self::each_surviving_cell`] and [`Self::relocate`] share
    /// so the accepted plan and the actual placements agree (#3302) —
    /// then the batch's pending cells (`plan_cells`, deduped misses
    /// only; pending-referenced cache entries arrive through the
    /// surviving set instead).
    fn live_cells(&mut self) {
        let mut plan_all = std::mem::take(&mut self.plan_all);
        plan_all.clear();
        self.each_surviving_cell(|_, _, w, h| plan_all.push((w, h)));
        plan_all.extend_from_slice(&self.plan_cells);
        self.plan_all = plan_all;
    }

    /// Whether `slot`'s shelf was touched this commit — the criterion
    /// the survival checks and the probe share (#3302). Indexes the
    /// current layout unchecked, like [`Self::shelf_epoch`]: callers
    /// hold current slots by construction.
    fn shelf_touched(&self, slot: u32) -> bool {
        touched_in(&self.layout, self.tick, slot)
    }

    /// Whether the glyph survives a repack: it owns no cell, its shelf
    /// was touched this commit, or the pending batch referenced its key.
    fn glyph_survives(&self, key: &GlyphKey, cached: &CachedGlyph) -> bool {
        cached
            .shelf
            .is_none_or(|slot| self.shelf_touched(slot) || self.dedupe_glyphs.contains(key))
    }

    /// Whether the path survives a repack: **every** cell's shelf was
    /// touched — an entry whose only touched shelves are shared with
    /// unrelated content drops whole, never in part — or the pending
    /// batch referenced its key. A span-only emission's `all` holds
    /// vacuously: it owns no pixels but stays a valid entry.
    fn path_survives(&self, key: u64, emit: &PathEmit) -> bool {
        emit.cells.iter().all(|c| self.shelf_touched(c.slot)) || self.dedupe_paths.contains(&key)
    }

    /// Whether the mask survives a repack: touched shelf or pending key.
    fn mask_survives(&self, key: u64, mask: &MaskCell) -> bool {
        self.shelf_touched(mask.slot) || self.dedupe_masks.contains(&key)
    }

    /// Every surviving cached cell in placement order — glyphs, paths,
    /// masks — yielding `(owner, old texel origin, w, h)`. Entries
    /// owning no pixels appear nowhere here but stay valid entries.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "cell rects and mask sizes are small positive floats"
    )]
    fn each_surviving_cell(&self, mut f: impl FnMut(CellOwner, (u32, u32), u32, u32)) {
        for (key, cached) in &self.map {
            if cached.shelf.is_some() && self.glyph_survives(key, cached) {
                f(
                    CellOwner::Glyph(*key),
                    (u32::from(cached.entry.x), u32::from(cached.entry.y)),
                    u32::from(cached.entry.w),
                    u32::from(cached.entry.h),
                );
            }
        }
        for (&key, emit) in &self.paths {
            if self.path_survives(key, emit) {
                for (i, cell) in emit.cells.iter().enumerate() {
                    f(
                        CellOwner::Path(key, i),
                        (u32::from(cell.x), u32::from(cell.y)),
                        (cell.rect[2] - cell.rect[0]).round().max(0.0) as u32,
                        (cell.rect[3] - cell.rect[1]).round().max(0.0) as u32,
                    );
                }
            }
        }
        for (&key, mask) in &self.masks {
            if self.mask_survives(key, mask) {
                f(
                    CellOwner::Mask(key),
                    (mask.atlas[0] as u32, mask.atlas[1] as u32),
                    mask.size[0].ceil().max(0.0) as u32,
                    mask.size[1].ceil().max(0.0) as u32,
                );
            }
        }
    }

    /// Whether every pending cell places as-is, with no eviction —
    /// the cheap pre-check the commit runs before deriving replay
    /// pins: a frame whose batch fits strictly needs no pin at all,
    /// and an empty batch skips even the layout clone (#119).
    pub fn fits_strict(&mut self, rasters: &[&PendingRaster]) -> bool {
        self.batch_cells(rasters);
        if self.plan_cells.is_empty() {
            return true;
        }
        let mut probe = self.layout.clone();
        self.plan_cells
            .iter()
            .all(|&(w, h)| alloc_on(&mut probe, self.size, self.tick, w, h).is_some())
    }

    /// Cells `rasters` still need beyond the live caches, in commit
    /// order, pooled into `plan_cells`. Keys repeated inside the batch
    /// resolve to the first store, exactly as the commit's cache
    /// inserts do.
    fn batch_cells(&mut self, rasters: &[&PendingRaster]) {
        self.dedupe_glyphs.clear();
        self.dedupe_paths.clear();
        self.dedupe_masks.clear();
        self.plan_cells.clear();
        for raster in rasters {
            match raster {
                PendingRaster::Glyph { key, w, h, .. } => {
                    // Every key is recorded — hits included — so the
                    // relocation's survival check sees the batch's cache
                    // references (#3302). Only a unique miss costs a cell.
                    let new_key = self.dedupe_glyphs.insert(*key);
                    if self.get(key).is_none() && new_key && *w > 0 {
                        self.plan_cells.push((*w, *h));
                    }
                }
                PendingRaster::Path {
                    key, cells: texels, ..
                } => {
                    let new_key = self.dedupe_paths.insert(*key);
                    if !self.paths.contains_key(key) && new_key {
                        self.plan_cells
                            .extend(texels.iter().map(|&(w, h, _)| (w, h)));
                    }
                }
                PendingRaster::Mask { key, w, h, .. } => {
                    let new_key = self.dedupe_masks.insert(*key);
                    if !self.masks.contains_key(key) && new_key {
                        self.plan_cells.push((*w, *h));
                    }
                }
                PendingRaster::MaskTexture { .. }
                | PendingRaster::Colr { .. }
                | PendingRaster::Bitmap { .. }
                | PendingRaster::GpuPath(_) => {}
            }
        }
    }

    /// Pages the atlas carries — the texture is `size` × `size * pages`.
    pub const fn pages(&self) -> u32 {
        self.pages
    }

    /// `(live shelves, virgin top, vacant bands)` for diagnostics.
    pub fn occupancy(&self) -> (usize, u32, usize) {
        (
            self.layout.shelves.iter().filter(|s| s.live).count(),
            self.layout.tops.iter().sum(),
            self.layout.vacant.len(),
        )
    }

    /// Whether a `w` × `h` cell fits in an empty atlas at the cap.
    pub const fn can_ever_fit(&self, w: u32, h: u32) -> bool {
        w + 2 * PAD <= self.cap && band(h) <= self.cap
    }

    /// Whether a `w` × `h` mask stays in the atlas: small enough and a
    /// cell could ever be allocated for it.
    pub fn mask_in_atlas(&self, w: u32, h: u32) -> bool {
        u64::from(w) * u64::from(h) <= MASK_TEXTURE_TEXELS && self.can_ever_fit(w, h)
    }

    /// Whether a `w` × `h` mask fits on a dedicated texture.
    pub const fn mask_texture_fits(&self, w: u32, h: u32) -> bool {
        w <= self.texture_limit && h <= self.texture_limit
    }

    /// A cached mask texture's cell.
    pub fn mask_texture(&self, key: u64) -> Option<&MaskCell> {
        self.mask_textures.get(&key).map(|t| &t.cell)
    }

    /// A cached mask texture's view, bound at group-1 binding 3.
    pub fn mask_texture_view(&self, key: u64) -> Option<&wgpu::TextureView> {
        self.mask_textures.get(&key).map(|t| &t.view)
    }

    /// The texture behind [`Self::mask_texture_view`], for the paths
    /// that transition the resource rather than bind it.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub fn mask_texture_resource(&self, key: u64) -> Option<&wgpu::Texture> {
        self.mask_textures.get(&key).map(|t| &t.texture)
    }

    /// Stores a mask on its own `w` × `h` `R8Unorm` texture, uploading
    /// `texels`. Idempotent on `key`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the pending raster record's fields, passed through"
    )]
    pub fn store_mask_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        mut cell: MaskCell,
        w: u32,
        h: u32,
        texels: &[u8],
    ) {
        if self.mask_textures.contains_key(&key) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("clip mask"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        crate::diag::create(device, "clip mask", u64::from(w) * u64::from(h));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        crate::diag::upload(
            device,
            "clip mask",
            u64::from(w) * u64::from(h),
            Some((0, 0, w, h)),
        );
        cell.atlas = [0.0, 0.0];
        let bytes = u64::from(w) * u64::from(h);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.mask_textures.insert(
            key,
            MaskTexture {
                cell,
                texture,
                view,
                bytes,
            },
        );
        self.mask_texture_bytes += bytes;
        self.cpu_bytes += bytes;
        self.mask_texture_gen = self
            .mask_texture_gen
            .checked_add(1)
            .expect("mask texture generation overflow");
    }

    /// Increments whenever `mask_textures` changes.
    pub const fn mask_texture_generation(&self) -> u64 {
        self.mask_texture_gen
    }

    /// GPU bytes held by mask textures.
    pub const fn mask_texture_bytes(&self) -> u64 {
        self.mask_texture_bytes
    }

    /// Whether `mask_textures` exceeds its budget.
    pub const fn mask_textures_over_budget(&self) -> bool {
        self.mask_texture_bytes > self.mask_budget
    }

    /// Drops every mask texture whose key `live` rejects. Frames and
    /// bind groups referencing a kept texture stay valid.
    pub fn evict_mask_textures(&mut self, device: &wgpu::Device, live: impl Fn(u64) -> bool) {
        let evicted: Vec<u64> = self
            .mask_textures
            .extract_if(|key, _| !live(*key))
            .map(|(_, t)| t.bytes)
            .collect();
        for bytes in &evicted {
            crate::diag::retire(
                device,
                crate::diag::RetireArgs {
                    label: "clip mask",
                    class: crate::diag::Class::MaskTexture,
                    bytes: *bytes,
                    used_in_latest_submit: true,
                    reason: "mask texture evict",
                },
            );
        }
        let freed: u64 = evicted.into_iter().sum();
        if freed > 0 {
            self.mask_texture_bytes = self.mask_texture_bytes.saturating_sub(freed);
            self.cpu_bytes = self.cpu_bytes.saturating_sub(freed);
            self.mask_texture_gen = self
                .mask_texture_gen
                .checked_add(1)
                .expect("mask texture generation overflow");
        }
    }

    /// Single-cell commit for tests: [`Self::place_glyph`] plus an
    /// immediate shelf-region upload. `None` when the atlas is full.
    #[cfg(test)]
    #[expect(
        clippy::too_many_arguments,
        reason = "the pending raster record's fields, passed through"
    )]
    fn store_glyph(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: GlyphKey,
        left: i32,
        top: i32,
        w: u32,
        h: u32,
        texels: &[u8],
    ) -> Option<(u32, u32)> {
        let mut uploads = CellWrites::default();
        let hit = self.get(&key).is_some();
        self.begin_commit(&[]);
        let out = self.place_glyph(key, left, top, w, h, texels.to_vec(), &mut uploads);
        let _ = self.upload_committed(device, queue, uploads);
        if !hit && w == 0 {
            crate::diag::atlas_cell(device, (0, 0, 0, 0));
        }
        out.map(|(origin, _)| origin)
    }

    /// The [`Self::store_glyph`] placement half: registers `key`'s cell
    /// without uploading, returning its texel origin and the shelf it
    /// lives on — `None` for a glyph with no outline, which owns no
    /// cell (#2327). On a miss the texels move into `writes` for the
    /// batch's shelf-region uploads (#169 A3). `None` when the cell
    /// does not fit — the caller plans first.
    #[expect(
        clippy::too_many_arguments,
        reason = "the pending raster record's fields, passed through"
    )]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "atlas cells fit u16: the atlas is at most 64kpx per axis"
    )]
    pub fn place_glyph(
        &mut self,
        key: GlyphKey,
        left: i32,
        top: i32,
        w: u32,
        h: u32,
        texels: Vec<u8>,
        writes: &mut CellWrites,
    ) -> Option<((u32, u32), Option<u32>)> {
        if let Some(cached) = self.map.get(&key).copied() {
            // A pending raster can hit a cold entry the lowering never
            // pinned — keep it out of this commit's eviction set (#119).
            // An outline-less glyph has no shelf to keep.
            if let Some(slot) = cached.shelf {
                self.touch(slot);
            }
            let origin = (u32::from(cached.entry.x), u32::from(cached.entry.y));
            return Some((origin, cached.shelf));
        }
        if w == 0 {
            self.map.insert(
                key,
                CachedGlyph {
                    entry: Entry::default(),
                    shelf: None,
                },
            );
            return Some(((0, 0), None));
        }
        let (cx, cy, slot) = self.alloc(w, h)?;
        self.cpu_bytes += u64::from(w) * u64::from(h) + LIVE_ENTRY_BYTES;
        let hk = live_hash(&key);
        let cell_shelf = u32::try_from(slot).expect("shelf count fits u32");
        let entry = Entry {
            x: cx as u16,
            y: cy as u16,
            w: w as u16,
            h: h as u16,
            left,
            top,
        };
        self.map.insert(
            key,
            CachedGlyph {
                entry,
                shelf: Some(cell_shelf),
            },
        );
        self.live.insert(
            hk,
            LiveEntry {
                kind: LiveKind::Glyph(key),
            },
        );
        self.slot_keys[slot].insert(hk);
        let len = texels.len();
        let arena = writes.push_arena(texels);
        writes.push(CellWrite {
            shelf: slot,
            x: cx,
            y: cy,
            w,
            h,
            arena,
            texels: 0..len,
        });
        Some(((cx, cy), Some(cell_shelf)))
    }

    /// The [`Self::store_path`] placement half: allocates each cell and
    /// patches its origin without uploading; `arena` is the batch's
    /// texel arena the cells' byte ranges index, already staged in
    /// `writes` (#169 A3). `None` when a cell does not fit — the caller
    /// plans first.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "atlas cells fit u16: the atlas is at most 64kpx per axis"
    )]
    pub fn place_path(
        &mut self,
        key: u64,
        emit: PathEmit,
        cells: Vec<CellTexels>,
        arena: u32,
        writes: &mut CellWrites,
    ) -> Option<()> {
        if let Some(emit) = self.paths.get(&key) {
            // Same: the hit's bands may be cold — pin them or a later
            // eviction this commit reclaims them (#119).
            let slots: Vec<u32> = emit.cells.iter().map(|c| c.slot).collect();
            for slot in slots {
                self.touch(slot);
            }
            return Some(());
        }
        let mut emit = emit;
        let mut slots: Vec<u32> = Vec::new();
        for (cell, (w, h, texels)) in emit.cells.iter_mut().zip(cells) {
            let (cx, cy, slot) = self.alloc(w, h)?;
            cell.x = cx as u16;
            cell.y = cy as u16;
            cell.slot = slot as u32;
            self.slot_keys[slot].insert(key);
            if !slots.contains(&(slot as u32)) {
                slots.push(slot as u32);
            }
            writes.push(CellWrite {
                shelf: slot,
                x: cx,
                y: cy,
                w,
                h,
                arena,
                texels,
            });
        }
        emit.slots = self.alloc_emit_slots(&slots);
        self.live.insert(
            key,
            LiveEntry {
                kind: LiveKind::Path,
            },
        );
        self.insert_path(key, emit);
        Some(())
    }

    /// Single-cell commit for tests: [`Self::place_mask`] plus an
    /// immediate shelf-region upload. `None` when the atlas is full.
    #[cfg(test)]
    #[expect(
        clippy::too_many_arguments,
        reason = "the device and queue plus the pending raster record's fields"
    )]
    fn store_mask(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        mask: MaskCell,
        w: u32,
        h: u32,
        texels: &[u8],
    ) -> Option<()> {
        let mut uploads = CellWrites::default();
        self.begin_commit(&[]);
        let out = self.place_mask(key, mask, w, h, texels.to_vec(), &mut uploads);
        let _ = self.upload_committed(device, queue, uploads);
        out
    }

    /// The [`Self::store_mask`] placement half: allocates the cell and
    /// sets `mask.atlas` without uploading; texels move into `writes`
    /// (#169 A3). `None` when the cell does not fit.
    #[expect(
        clippy::cast_precision_loss,
        reason = "atlas coords are well within f32"
    )]
    pub fn place_mask(
        &mut self,
        key: u64,
        mut mask: MaskCell,
        w: u32,
        h: u32,
        texels: Vec<u8>,
        writes: &mut CellWrites,
    ) -> Option<()> {
        if let Some(mask) = self.masks.get(&key) {
            // Same: pin the slot so this commit cannot evict it (#119).
            let slot = mask.slot;
            self.touch(slot);
            return Some(());
        }
        let (cx, cy, slot) = self.alloc(w, h)?;
        mask.atlas = [cx as f32, cy as f32];
        mask.slot = u32::try_from(slot).expect("shelf count fits u32");
        self.live.insert(
            key,
            LiveEntry {
                kind: LiveKind::Mask,
            },
        );
        self.slot_keys[slot].insert(key);
        self.insert_mask(key, mask);
        let len = texels.len();
        let arena = writes.push_arena(texels);
        writes.push(CellWrite {
            shelf: slot,
            x: cx,
            y: cy,
            w,
            h,
            arena,
            texels: 0..len,
        });
        Some(())
    }

    /// Reserves `slots` in the `emit_slots` arena — a dead range
    /// first, the tail otherwise (#119).
    fn alloc_emit_slots(&mut self, slots: &[u32]) -> Range<usize> {
        if let Some(pos) = self.slot_holes.iter().position(|r| r.len() >= slots.len()) {
            let hole = self.slot_holes.swap_remove(pos);
            let range = hole.start..hole.start + slots.len();
            self.emit_slots[range.clone()].copy_from_slice(slots);
            if hole.end > range.end {
                self.slot_holes.push(range.end..hole.end);
            }
            return range;
        }
        let start = self.emit_slots.len();
        self.emit_slots.extend_from_slice(slots);
        start..self.emit_slots.len()
    }

    /// The deduplicated shelf slots a `PathEmit::slots` range addresses.
    pub fn emit_slot_arena(&self, range: Range<usize>) -> &[u32] {
        &self.emit_slots[range]
    }

    /// The atlas origin of the cached mask for `key`.
    pub fn mask_origin(&self, key: u64) -> Option<[f32; 2]> {
        self.masks.get(&key).map(|m| m.atlas)
    }

    /// Resizes the atlas to `size` × `pages` pages (edge clamped to the
    /// cap, page count to [`Self::max_pages`]), dropping every cached
    /// entry. `#169 A3` passes the dry-run target, so a batch resizes
    /// once instead of doubling until it happens to fit; a paged atlas
    /// also shrinks back once a batch's cells pack onto fewer pages
    /// (#211). Growing in pages keeps the edge — and every cell's x
    /// bound — unchanged while the texture's y range extends.
    #[cfg(test)]
    pub fn resize_to(&mut self, device: &wgpu::Device, size: u32, pages: u32) {
        let size = size.min(self.cap);
        let pages = pages.clamp(1, self.max_pages(size));
        if size == self.size && pages == self.pages {
            return;
        }
        let old = self.gpu_bytes();
        let (texture, view) = Self::allocate(device, size, pages);
        self.texture = Some(texture);
        self.view = view;
        self.size = size;
        self.pages = pages;
        crate::diag::grow(
            device,
            "glyph atlas",
            crate::diag::Class::Atlas,
            old,
            self.gpu_bytes(),
            0,
            true,
        );
        self.clear();
    }

    /// Repacks the atlas at `size` × `pages` pages **without**
    /// discarding live coverage (#3302): surviving cells migrate into a
    /// freshly laid out layout of the target shape and their texels are
    /// copied texture-to-texture inside `encoder` — the caller owns the
    /// encoder's submission, the submit guard and tracking, and the old
    /// texture stays alive through the encoded copies. `None` when the
    /// surviving cells do not fit the target shape.
    ///
    /// Survival is the batch's: an entry keeps its pixels when its
    /// shelf was touched this commit or its key appears in the pending
    /// reference sets `batch_cells` last populated — call only inside a
    /// commit after `plan`/`fits_strict` ran. Entries owning no pixels
    /// (outline-less glyphs, span-only paths) always survive; cold
    /// entries drop and are reported through [`Self::take_evicted`]
    /// like reclaimed shelves. Dedicated mask textures are untouched.
    /// The replacement texture exists only when the old one did: an
    /// idle atlas stays storage-free and the next commit's
    /// [`Self::ensure_storage`] allocates at the new shape. `None`
    /// signals exhaustion to the caller; this method owns no retry.
    #[expect(
        clippy::too_many_lines,
        reason = "the repack is one atomic transition: preflight, copy, rebuild"
    )]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "atlas cells fit u16, atlas coords fit f32, cell sizes are small positive floats"
    )]
    pub fn relocate(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: u32,
        pages: u32,
    ) -> Option<Relocation> {
        let size = size.min(self.cap);
        let pages = pages.clamp(1, self.max_pages(size));
        let tick = self.tick;
        let mut layout = Layout {
            shelves: Vec::new(),
            vacant: Vec::new(),
            tops: vec![0; pages as usize],
            // Continuing the old counter keeps every migrated band's
            // epoch above any stale `(slot, epoch)` reference, so old
            // references can never alias the new layout (#3302).
            next_epoch: self.layout.next_epoch,
        };
        // Preflight: place every surviving cached cell into the new
        // layout, in `live_cells`'s probe order so the accepted plan
        // and these placements agree. A failure leaves `self`
        // untouched.
        let mut moves: Vec<CellMove> = Vec::new();
        let mut fits = true;
        self.each_surviving_cell(|owner, src, w, h| {
            if !fits {
                return;
            }
            let Some((x, y, slot)) = alloc_on(&mut layout, size, tick, w, h) else {
                fits = false;
                return;
            };
            moves.push(CellMove {
                owner,
                src,
                dst: (x, y),
                dst_slot: u32::try_from(slot).expect("shelf count fits u32"),
                w,
                h,
            });
        });
        if !fits {
            return None;
        }
        debug_assert!(
            self.texture.is_some() || moves.is_empty(),
            "cached cells imply committed uploads and thus atlas storage"
        );
        let old_bytes = self.gpu_bytes();
        self.size = size;
        self.pages = pages;
        if let Some(old_texture) = self.texture.take() {
            let (texture, view) = Self::allocate(device, size, pages);
            for m in &moves {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &old_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: m.src.0,
                            y: m.src.1,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: m.dst.0,
                            y: m.dst.1,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: m.w,
                        height: m.h,
                        depth_or_array_layers: 1,
                    },
                );
            }
            self.texture = Some(texture);
            self.view = view;
            self.generation = self
                .generation
                .checked_add(1)
                .expect("atlas generation overflow");
            crate::diag::grow(
                device,
                "glyph atlas",
                crate::diag::Class::Atlas,
                old_bytes,
                self.gpu_bytes(),
                moves.iter().map(|m| u64::from(m.w) * u64::from(m.h)).sum(),
                true,
            );
        }
        let old = std::mem::replace(&mut self.layout, layout);
        // Pending cells may append to migrated shelves but must never
        // rewrite migrated texels: each migrated band's upload region
        // starts at its migrated frontier.
        for shelf in &mut self.layout.shelves {
            if shelf.live {
                shelf.base = shelf.x;
            }
        }
        let dedupe_glyphs = std::mem::take(&mut self.dedupe_glyphs);
        let dedupe_paths = std::mem::take(&mut self.dedupe_paths);
        let dedupe_masks = std::mem::take(&mut self.dedupe_masks);
        // Dropped entries' bytes, paired like `evict_one`'s record with
        // whether their band served the previous commit.
        let mut dropped: Vec<(u64, bool)> = Vec::new();
        let served = |slot: u32| {
            old.shelves[usize::try_from(slot).expect("shelf index")].last_used + 1 == tick
        };
        self.map.retain(|key, cached| {
            let Some(slot) = cached.shelf else {
                return true;
            };
            if touched_in(&old, tick, slot) || dedupe_glyphs.contains(key) {
                return true;
            }
            dropped.push((
                u64::from(cached.entry.w) * u64::from(cached.entry.h) + LIVE_ENTRY_BYTES,
                served(slot),
            ));
            false
        });
        self.paths.retain(|key, emit| {
            if emit.cells.iter().all(|c| touched_in(&old, tick, c.slot))
                || dedupe_paths.contains(key)
            {
                return true;
            }
            let first = emit.cells.first().expect("a dropped path owns cells");
            dropped.push((path_entry_bytes(emit), served(first.slot)));
            false
        });
        self.masks.retain(|key, mask| {
            if touched_in(&old, tick, mask.slot) || dedupe_masks.contains(key) {
                return true;
            }
            dropped.push((
                (mask.size[0] * mask.size[1]) as u64 + LIVE_ENTRY_BYTES,
                served(mask.slot),
            ));
            false
        });
        for m in &moves {
            match m.owner {
                CellOwner::Glyph(key) => {
                    let cached = self.map.get_mut(&key).expect("surviving glyph");
                    cached.entry.x = m.dst.0 as u16;
                    cached.entry.y = m.dst.1 as u16;
                    cached.shelf = Some(m.dst_slot);
                }
                CellOwner::Path(key, i) => {
                    let cell = &mut self.paths.get_mut(&key).expect("surviving path").cells[i];
                    cell.x = m.dst.0 as u16;
                    cell.y = m.dst.1 as u16;
                    cell.slot = m.dst_slot;
                }
                CellOwner::Mask(key) => {
                    let mask = self.masks.get_mut(&key).expect("surviving mask");
                    mask.atlas = [m.dst.0 as f32, m.dst.1 as f32];
                    mask.slot = m.dst_slot;
                }
            }
        }
        // Rebuild `emit_slots`, `slot_keys`, `live` and `cpu_bytes`
        // against the new layout — the same accounting `place_*` runs.
        self.emit_slots.clear();
        self.slot_holes.clear();
        self.slot_dead.clear();
        self.slot_keys.clear();
        self.slot_keys
            .resize_with(self.layout.shelves.len(), Default::default);
        self.live.clear();
        self.cpu_bytes = self.mask_texture_bytes;
        for (key, cached) in &self.map {
            if let Some(slot) = cached.shelf {
                let hk = live_hash(key);
                self.live.insert(
                    hk,
                    LiveEntry {
                        kind: LiveKind::Glyph(*key),
                    },
                );
                self.slot_keys[usize::try_from(slot).expect("shelf index")].insert(hk);
                self.cpu_bytes +=
                    u64::from(cached.entry.w) * u64::from(cached.entry.h) + LIVE_ENTRY_BYTES;
            }
        }
        for (&key, emit) in &mut self.paths {
            // Migrated slot arena: deduplicated like `place_path`'s,
            // appended to the fresh arena — holes were cleared above so
            // this is the range `alloc_emit_slots` would return.
            let start = self.emit_slots.len();
            for cell in &emit.cells {
                self.slot_keys[usize::try_from(cell.slot).expect("shelf index")].insert(key);
                if !self.emit_slots[start..].contains(&cell.slot) {
                    self.emit_slots.push(cell.slot);
                }
            }
            emit.slots = start..self.emit_slots.len();
            self.live.insert(
                key,
                LiveEntry {
                    kind: LiveKind::Path,
                },
            );
            self.cpu_bytes += path_entry_bytes(emit);
        }
        for (&key, mask) in &self.masks {
            self.slot_keys[usize::try_from(mask.slot).expect("shelf index")].insert(key);
            self.live.insert(
                key,
                LiveEntry {
                    kind: LiveKind::Mask,
                },
            );
            self.cpu_bytes += (mask.size[0] * mask.size[1]) as u64 + LIVE_ENTRY_BYTES;
        }
        self.dedupe_glyphs = dedupe_glyphs;
        self.dedupe_paths = dedupe_paths;
        self.dedupe_masks = dedupe_masks;
        self.evicted.extend(dropped);
        Some(Relocation {
            cells: moves
                .iter()
                .map(|m| {
                    (
                        m.src,
                        CellLocation {
                            x: m.dst.0,
                            y: m.dst.1,
                        },
                    )
                })
                .collect(),
            old_shelves: old.shelves.iter().map(|s| (s.epoch, s.live)).collect(),
        })
    }

    /// Whether `slot` currently holds a live band at `epoch` — the
    /// bounds-safe stale check for references recorded before a repack
    /// shrank the shelf vector (#3302). [`Self::shelf_epoch`] stays for
    /// callers holding known-current slots.
    pub fn shelf_matches(&self, slot: u32, epoch: u64) -> bool {
        usize::try_from(slot)
            .ok()
            .and_then(|i| self.layout.shelves.get(i))
            .is_some_and(|s| s.live && s.epoch == epoch)
    }

    /// The most pages an atlas of `size` may carry: the device's 2D
    /// height limit, the atlas's `budget / 8` byte share, and the `u16`
    /// texel origins retained admissions store.
    #[expect(clippy::cast_possible_truncation, reason = "atlas sizes are small")]
    fn max_pages(&self, size: u32) -> u32 {
        let page_texels = u64::from(size) * u64::from(size);
        (self.texture_limit.min(65_536) / size)
            .min((self.page_budget.max(page_texels) / page_texels) as u32)
            .max(1)
    }

    /// Starts a commit batch: advances the commit ordinal, marks every
    /// shelf `touches` hit during lowering as live this commit so it is
    /// never a victim, and freezes each live shelf's `x` as `base` for
    /// [`Self::upload_committed`]'s region diffing.
    pub fn begin_commit(&mut self, touches: &[u32]) {
        self.slot_holes.append(&mut self.slot_dead);
        self.tick += 1;
        self.evicting = false;
        self.evicted.clear();
        for &idx in touches {
            let row = &mut self.layout.shelves[usize::try_from(idx).expect("shelf index")];
            // One mark per slot per commit.
            if row.last_used < self.tick {
                row.last_used = self.tick;
                row.hits = row.hits.saturating_add(1);
            }
        }
        for shelf in &mut self.layout.shelves {
            if shelf.live {
                shelf.base = shelf.x;
            }
        }
    }

    /// Identity an emission can compare in one load: texture generation
    /// (grow/recycle) in the high half, eviction clock in the low — any
    /// atlas change invalidating retained UVs moves it (#119).
    pub const fn live_stamp(&self) -> u64 {
        (self.generation << 32) | (self.clock & 0xFFFF_FFFF)
    }

    /// The epoch of the band occupying `slot` — compared with the
    /// `(slot, epoch)` references retained emissions hold.
    pub fn shelf_epoch(&self, slot: u32) -> u64 {
        self.layout.shelves[usize::try_from(slot).expect("shelf index")].epoch
    }

    /// The shelf whose band run owns the atlas pixel `(x, y)` — the
    /// pixel's page is `(y / size)`, so only that page's bands are
    /// scanned; the pixel must also sit inside the shelf's used run
    /// (`x < shelf.x`). Recovers a stored instance UV's band without
    /// the leaf recording the slot at emit time (#119). Dead bands
    /// hold no texels a sampler can reference, so only live ones hit.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "atlas pixels fit u32"
    )]
    pub fn shelf_at(&self, x: f32, y: f32) -> Option<u32> {
        if !x.is_finite() || !y.is_finite() || y < 0.0 {
            return None;
        }
        self.shelf_hit(y as u32, x as u32)
    }

    /// [`Self::shelf_at`] over many points, like a leaf's emission:
    /// consecutive cells usually sit on the same shelf, so `hint`
    /// carries the last hit's slot and skips the scan while the
    /// point still lands in that band (#119).
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "atlas pixels fit u32"
    )]
    pub fn shelf_at_hint(&self, x: f32, y: f32, hint: &mut Option<u32>) -> Option<u32> {
        if !x.is_finite() || !y.is_finite() || y < 0.0 {
            return None;
        }
        let y = y as u32;
        let x = x as u32;
        if let Some(slot) = *hint
            && let Some(shelf) = self.layout.shelves.get(slot as usize)
            && shelf.live
            && shelf.y <= y
            && y < shelf.y + shelf.h
            && x < shelf.x
        {
            return Some(slot);
        }
        let slot = self.shelf_hit(y, x);
        *hint = slot;
        slot
    }

    /// The live shelf covering texel `(x, y)` in `y`, as a slot index.
    /// Bands never overlap in y within a page, so the scan hits at
    /// most once; shelves from dead-band splits sit at any index, so
    /// the search is a filter rather than a binary search.
    ///
    /// Kept out of line: the commit's pin walk calls the hint check
    /// from four nested loops, and inlined there at opt-level 3 the
    /// scan's position counter is spilled to the stack, so every shelf
    /// pays a store-to-load round trip (#2852). In its own frame the
    /// counter stays in a register.
    #[inline(never)]
    fn shelf_hit(&self, y: u32, x: u32) -> Option<u32> {
        let page = y / self.size;
        self.layout
            .shelves
            .iter()
            .position(|s| s.live && s.y <= y && y < s.y + s.h && s.y / self.size == page && x < s.x)
            .and_then(|i| u32::try_from(i).ok())
    }

    /// Lets [`Self::alloc`] reclaim the coldest shelves: set by the
    /// commit when the plan's dry run needed eviction — or exhausted
    /// the layout — so in-place eviction, not a wholesale clear, makes
    /// room (#119).
    pub const fn enable_evicting(&mut self) {
        self.evicting = true;
    }

    /// Bytes this commit's evictions reclaimed, paired with whether the
    /// shelf had served the previous commit — the caller reports them
    /// through `diag::retire` and clears the list itself.
    pub fn take_evicted(&mut self) -> Vec<(u64, bool)> {
        std::mem::take(&mut self.evicted)
    }

    /// Marks `slot` used by the current commit — a hit mid-commit or a
    /// fresh allocation — so the same commit cannot evict it.
    fn touch(&mut self, slot: u32) {
        let row = &mut self.layout.shelves[usize::try_from(slot).expect("shelf index")];
        if row.last_used < self.tick {
            row.last_used = self.tick;
            row.hits = row.hits.saturating_add(1);
        }
    }

    /// Evicts the coldest shelf the current commit may touch: the least
    /// recently used of the probationary segment (`hits == 0`) if any,
    /// else the LRU of the protected segment. `false` when nothing is
    /// evictable — every shelf was touched this commit or the layout is
    /// empty.
    fn evict_one(&mut self) -> bool {
        let Some(slot) = evict_layout(&mut self.layout, self.size, self.tick) else {
            return false;
        };
        let served_recently = self.layout.shelves[slot].last_used + 1 == self.tick;
        let mut bytes = 0u64;
        for hk in std::mem::take(&mut self.slot_keys[slot]) {
            let Some(live) = self.live.remove(&hk) else {
                continue;
            };
            match live.kind {
                LiveKind::Glyph(key) => {
                    if let Some(cached) = self.map.remove(&key) {
                        bytes += u64::from(cached.entry.w) * u64::from(cached.entry.h)
                            + LIVE_ENTRY_BYTES;
                    }
                }
                LiveKind::Path => {
                    if let Some(emit) = self.paths.remove(&hk) {
                        self.slot_dead.push(emit.slots.clone());
                        bytes += path_entry_bytes(&emit);
                    }
                }
                LiveKind::Mask => {
                    if let Some(mask) = self.masks.remove(&hk) {
                        #[expect(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "cell sizes are small positive floats"
                        )]
                        {
                            bytes += (mask.size[0] * mask.size[1]) as u64 + LIVE_ENTRY_BYTES;
                        }
                    }
                }
            }
        }
        self.cpu_bytes = self.cpu_bytes.saturating_sub(bytes);
        self.clock += 1;
        self.evicted.push((bytes, served_recently));
        true
    }

    /// Places a `w` × `h` cell: a live shelf of its height class first,
    /// then a dead band, then virgin space — and, when the commit is
    /// evicting, a cold shelf reclaimed in place. Returns the texel
    /// origin and the shelf (eviction slot) index.
    pub(crate) fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32, usize)> {
        loop {
            if let Some((x, y, slot)) = alloc_on(&mut self.layout, self.size, self.tick, w, h) {
                while self.slot_keys.len() < self.layout.shelves.len() {
                    self.slot_keys.push(rustc_hash::FxHashSet::default());
                }
                return Some((x, y, slot));
            }
            if !self.evicting || !self.evict_one() {
                return None;
            }
        }
    }
}

/// Whether `slot`'s shelf in `layout` is live and was used at `tick` —
/// the touch mark `begin_commit`, cache hits and `alloc_on` leave, and
/// the survival criterion the repack and the probe share (#3302).
fn touched_in(layout: &Layout, tick: u64, slot: u32) -> bool {
    let row = &layout.shelves[usize::try_from(slot).expect("shelf index")];
    row.live && row.last_used == tick
}

/// The shelf `tick` may evict, picked the way [`Atlas::evict_one`]
/// picks: the least recently used of the probationary segment
/// (`hits == 0`) if any, else the LRU of the protected segment. Marks
/// it dead and hands its band to `vacant`. `None` when every shelf was
/// touched this commit or the layout is empty. `Atlas::plan`'s dry run
/// shares this so the verdict matches what the commit would do (#119).
fn evict_layout(layout: &mut Layout, size: u32, tick: u64) -> Option<usize> {
    let slot = layout
        .shelves
        .iter()
        .enumerate()
        .filter(|(_, s)| s.live && s.last_used < tick)
        .min_by_key(|(_, s)| (u8::from(s.hits > 0), s.last_used))
        .map(|(i, _)| i)?;
    layout.shelves[slot].live = false;
    free_band(layout, size, slot);
    Some(slot)
}

/// Returns the dead band at `slot` to `vacant` — coalesced. Evicting
/// many small shelves one by one would otherwise leave the free space
/// fragmented by height class: a tall cell can only reclaim a band at
/// least as tall, so dead bands merge with vertically adjacent dead
/// bands, and a merged band that reaches the layout's frontier hands
/// its rows back to `top` as virgin space instead (#119). The
/// invariant maintained here — no two `vacant` bands are ever
/// vertically adjacent — is what makes the merge bounded. The merged
/// band keeps `slot`'s index; merged-away slots stay dead with
/// `h == 0` and are never reused.
fn free_band(layout: &mut Layout, size: u32, slot: usize) {
    debug_assert!(!layout.shelves[slot].live);
    layout.shelves[slot].epoch = layout.next_epoch;
    layout.next_epoch += 1;
    let mut slot = slot;
    loop {
        let mut y = layout.shelves[slot].y;
        let mut h = layout.shelves[slot].h;
        // A band lives inside one page: merges never cross a page edge.
        let page = y / size;
        // Fold in every dead band vertically adjacent to this one;
        // the invariant limits it to two, but rerunning the scan is
        // cheap at this `vacant` size.
        let mut merged = true;
        while merged {
            merged = false;
            for i in 0..layout.vacant.len() {
                let other = usize::try_from(layout.vacant[i]).expect("shelf index");
                let (oy, oh) = (layout.shelves[other].y, layout.shelves[other].h);
                if (oy + oh == y || oy == y + h) && oy / size == page {
                    y = y.min(oy);
                    h += oh;
                    layout.shelves[other].h = 0;
                    layout.vacant.swap_remove(i);
                    merged = true;
                    break;
                }
            }
        }
        let top = &mut layout.tops[page as usize];
        if y + h != page * size + *top {
            layout.shelves[slot].y = y;
            layout.shelves[slot].h = h;
            layout
                .vacant
                .push(u32::try_from(slot).expect("shelf count fits u32"));
            return;
        }
        // Reaching the frontier recycles the band as virgin rows; the
        // slot joins the dead `h == 0` ghost set. The new top may
        // already abut another dead band — fold that one the same way.
        layout.shelves[slot].h = 0;
        *top = y - page * size;
        let top = layout.tops[page as usize];
        let Some(back) = layout.vacant.iter().position(|&i| {
            let s = &layout.shelves[usize::try_from(i).expect("shelf index")];
            s.y / size == page && s.y + s.h == page * size + top
        }) else {
            return;
        };
        slot = usize::try_from(layout.vacant.swap_remove(back)).expect("shelf index");
    }
}

/// A cell's shelf band height: cell height plus padding, quantised up
/// to a multiple of 8 — the rows the cell actually consumes on a shelf.
const fn band(h: u32) -> u32 {
    (h + 2 * PAD).div_ceil(8) * 8
}

/// `Atlas::alloc` against an explicit layout and atlas edge — never
/// evicts. The dry-run side of a transactional batch clones `layout`
/// and calls this instead (#169 A3); the real side reaches it through
/// `Atlas::alloc`'s eviction loop.
fn alloc_on(
    layout: &mut Layout,
    size: u32,
    tick: u64,
    w: u32,
    h: u32,
) -> Option<(u32, u32, usize)> {
    // Every band — a live shelf, a dead band, virgin rows — spans one
    // page's width, so a cell wider than the edge places nowhere. It
    // fails here, before any branch, so the plan grows the edge (#234)
    // instead of a reclaimed band taking it and the shelf upload
    // overrunning the texture's x extent (#2396).
    if w + 2 * PAD > size {
        return None;
    }
    // Shelf height classes are multiples of 8.
    let class = band(h);
    for (i, shelf) in layout.shelves.iter_mut().enumerate() {
        if shelf.live && shelf.h == class && shelf.x + w + 2 * PAD <= size {
            let x = shelf.x + PAD;
            shelf.x += w + 2 * PAD;
            // Allocations pin (`last_used`) so the same commit cannot
            // evict the shelf; only a cache hit promotes (`hits`).
            shelf.last_used = shelf.last_used.max(tick);
            return Some((x, shelf.y + PAD, i));
        }
    }
    // A dead band takes the cell when it is tall enough — the smallest
    // that fits, so large bands survive for larger classes; the
    // remainder splits off as a smaller dead band.
    let best = layout
        .vacant
        .iter()
        .enumerate()
        .map(|(pos, &i)| (pos, usize::try_from(i).expect("shelf index")))
        .filter(|&(_, i)| layout.shelves[i].h >= class)
        .min_by_key(|&(_, i)| layout.shelves[i].h)
        .map(|(pos, _)| pos);
    if let Some(pos) = best {
        let slot = usize::try_from(layout.vacant.swap_remove(pos)).expect("shelf index");
        if layout.shelves[slot].h > class {
            let rest = layout.shelves[slot].h - class;
            let y = layout.shelves[slot].y + class;
            layout.shelves.push(Shelf {
                y,
                h: rest,
                x: 0,
                base: 0,
                last_used: 0,
                hits: 0,
                live: false,
                epoch: layout.next_epoch,
            });
            layout.next_epoch += 1;
            let phantom = layout.shelves.len() - 1;
            free_band(layout, size, phantom);
        }
        let shelf = &mut layout.shelves[slot];
        shelf.h = class;
        shelf.x = w + 2 * PAD;
        shelf.base = 0;
        shelf.last_used = tick;
        shelf.hits = 0;
        shelf.live = true;
        shelf.epoch = layout.next_epoch;
        layout.next_epoch += 1;
        return Some((PAD, shelf.y + PAD, slot));
    }
    // The virgin scan fills pages front to back; no room on any page
    // fails the placement.
    for (page, top) in layout.tops.iter_mut().enumerate() {
        if *top + class > size {
            continue;
        }
        let base = u32::try_from(page).expect("pages fit u32") * size;
        let i = layout.shelves.len();
        layout.shelves.push(Shelf {
            y: base + *top,
            h: class,
            x: w + 2 * PAD,
            base: 0,
            last_used: tick,
            hits: 0,
            live: true,
            epoch: layout.next_epoch,
        });
        layout.next_epoch += 1;
        *top += class;
        return Some((PAD, base + *top - class + PAD, i));
    }
    None
}

/// The `live`/`slot_keys` hash of a glyph key: the maps index glyph
/// admissions by the same hash eviction bookkeeping does.
pub fn live_hash(key: &GlyphKey) -> u64 {
    rustc_hash::FxBuildHasher.hash_one(key)
}

/// An [`OutlinePen`] collecting a glyph outline into a `kurbo::BezPath`.
struct PathPen {
    path: kurbo::BezPath,
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

/// A glyph's coverage texels, not yet in the atlas.
struct CellRaster {
    /// Bearing: cell's left edge relative to the glyph's snapped origin.
    left: i32,
    /// Bearing: cell's top edge relative to the snapped origin.
    top: i32,
    /// Cell width.
    w: u32,
    /// Cell height.
    h: u32,
    /// `w` × `h` coverage texels.
    texels: Vec<u8>,
}

/// One path cell's raster pending its atlas origin: `(w, h, texels)` —
/// `texels` a byte range of the lowering batch's arena, `w * h` long.
pub type CellTexels = (u32, u32, Range<usize>);

/// A committed cell's texels, uploaded with its shelf's newly
/// allocated region rather than on its own (#169 A3).
pub struct CellWrite {
    /// Shelf index [`Atlas::alloc`] placed the cell on.
    pub shelf: usize,
    /// Cell texel origin.
    pub x: u32,
    /// Cell texel origin.
    pub y: u32,
    /// Cell width in texels.
    pub w: u32,
    /// Cell height in texels.
    pub h: u32,
    /// The [`CellWrites`] arena `texels` indexes.
    pub arena: u32,
    /// `w` × `h` coverage texels as a byte range of `arena`.
    pub texels: Range<usize>,
}

/// One commit's staged atlas writes and the texel arenas their byte
/// ranges index: one arena per lowering batch for path cells, one
/// owned raster per glyph or mask write.
///
/// An arena's bytes are consumed by [`Atlas::upload_committed`], which
/// copies them into the upload ring's GPU staging — the batch moves
/// into the upload and comes back emptied, so no arena survives it:
/// only the container's list capacities are retained across commits,
/// and CPU buffers never alias the ring's in-flight staging copies,
/// whose lifetime is the ring's (see `upload.rs`).
#[derive(Default)]
pub struct CellWrites {
    /// Texel arenas the staged `texels` ranges index.
    arenas: Vec<Vec<u8>>,
    /// The staged writes in commit order.
    cells: Vec<CellWrite>,
}

impl CellWrites {
    /// Moves `arena` in and returns its index for [`CellWrite::arena`].
    /// The buffer is dropped when the commit's upload consumes the
    /// batch.
    pub fn push_arena(&mut self, arena: Vec<u8>) -> u32 {
        let idx = u32::try_from(self.arenas.len()).expect("commit arenas fit u32");
        self.arenas.push(arena);
        idx
    }

    /// Stages a write.
    pub fn push(&mut self, cell: CellWrite) {
        self.cells.push(cell);
    }

    /// The staged writes in commit order.
    pub fn cells(&self) -> &[CellWrite] {
        &self.cells
    }

    /// The staged write count.
    pub const fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the commit staged no writes.
    pub const fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Clears the staged writes, dropping every arena with the commit.
    pub fn clear(&mut self) {
        self.cells.clear();
        self.arenas.clear();
    }

    /// Heap bytes the staged arena buffers alone retain — the ranges
    /// the commit's writes index, without the list capacities below.
    fn arena_heap_bytes(&self) -> u64 {
        self.arenas.iter().map(Vec::capacity).sum::<usize>() as u64
    }

    /// Heap bytes the container owns: the staged arenas plus the
    /// `cells` and `arenas` list capacities an emptied container keeps
    /// at their peak for the next commit to refill — retained for the
    /// engine's life until a trim replaces the container (#2711).
    pub fn heap_bytes(&self) -> u64 {
        (self.cells.capacity() * size_of::<CellWrite>()
            + self.arenas.capacity() * size_of::<Vec<u8>>()) as u64
            + self.arena_heap_bytes()
    }

    /// The cell's `w` × `h` texel bytes.
    pub fn texels(&self, cell: &CellWrite) -> &[u8] {
        &self.arenas[cell.arena as usize][cell.texels.clone()]
    }
}

/// The batch placement decision [`Atlas::plan`] reaches (#169 A3).
pub enum AtlasPlan {
    /// Every pending cell fits on the live shelves.
    Fits,
    /// Every pending cell fits only once bounded eviction reclaims the
    /// shelves nothing touched this commit (#119); commit evicting.
    FitsEviction,
    /// The batch's own cell set packs to this `(edge, pages)` — larger
    /// when it outgrows the atlas, smaller when it releases pages left
    /// over from a peak frame — which after repacking holds every
    /// surviving cached and pending cell (#211).
    Resize(u32, u32),
    /// Even the largest `(edge, pages)` shape cannot hold the batch's
    /// shelf footprint: the commit evicts what it can and reports the
    /// first surface that still overflows.
    Recycle,
}

impl PendingRaster {
    /// Cells this raster will place — its atlas footprint in cell
    /// count for diagnostics.
    pub fn cell_count(&self) -> usize {
        match self {
            Self::Glyph { w, .. } | Self::Mask { w, .. } => usize::from(*w > 0),
            Self::Path { cells, .. } => cells.len(),
            Self::MaskTexture { .. }
            | Self::Colr { .. }
            | Self::Bitmap { .. }
            | Self::GpuPath(_) => 0,
        }
    }
}

/// Work deferred to the render thread by a parallel lowering: every
/// atlas write and COLR cache update a surface wanted, in lowering
/// order, with the bitmaps already rasterized from immutable font data.
/// The render thread applies the batches serially, in dirty-surface
/// order, so the atlas ends in exactly the state serial lowering would
/// produce.
pub enum PendingRaster {
    /// A glyph coverage cell, keyed by `GlyphKey`. `w`/`h` are `0` for
    /// glyphs with no outline: they still must be stored so the miss is
    /// not retried every frame.
    Glyph {
        /// The cache key.
        key: GlyphKey,
        /// Bearing: the cell's left edge relative to the snapped origin.
        left: i32,
        /// Bearing: the cell's top edge relative to the snapped origin.
        top: i32,
        /// Cell width.
        w: u32,
        /// Cell height.
        h: u32,
        /// `w` × `h` coverage texels.
        texels: Vec<u8>,
    },
    /// A path emission whose cells were rasterized cell by cell; each
    /// entry of `cells` pairs one `(w, h, byte range)` with the cell in
    /// `emit` at the same index.
    Path {
        /// The content-hash key.
        key: u64,
        /// Spans and cells; each cell's `x`/`y` is set when it is
        /// stored.
        emit: PathEmit,
        /// Per-cell `(width, height, byte range)` in emission order,
        /// indexing the lowering batch's texel arena.
        cells: Vec<CellTexels>,
    },
    /// A path-clip mask; `mask.atlas` is set when it is stored.
    Mask {
        /// The content-hash key.
        key: u64,
        /// The mask, pending its atlas origin.
        mask: MaskCell,
        /// Cell width.
        w: u32,
        /// Cell height.
        h: u32,
        /// `w` × `h` coverage texels.
        texels: Vec<u8>,
    },
    /// A path-clip mask too large for the atlas, stored on its own
    /// texture and bound at group-1 binding 3.
    MaskTexture {
        /// The content-hash key.
        key: u64,
        /// The mask; `atlas` stays `[0, 0]`.
        mask: MaskCell,
        /// Texture width.
        w: u32,
        /// Texture height.
        h: u32,
        /// `w` × `h` coverage texels.
        texels: Vec<u8>,
    },
    /// A built COLR picture for one registered font.
    Colr {
        /// `Renderer::fonts` key of the font that produced it.
        font: u64,
        /// The `(glyph id, coords hash, paint hash)` cache key.
        key: (u32, u64, u64),
        /// The built picture.
        picture: cherenkov::Picture,
    },
    /// A decoded bitmap glyph pending texture insertion.
    Bitmap {
        /// Exact font, strike, and glyph identity.
        key: super::bitmap::BitmapKey,
        /// Placement in em-space.
        em: kurbo::Rect,
        /// Bitmap width.
        width: u32,
        /// Bitmap height.
        height: u32,
        /// The decoded pixels, stored like an image of them.
        upload: super::image_storage::Upload<'static>,
    },
    /// A path routed to the GPU coverage kernels (issue #2543, decision
    /// S3): the lowering recorded its curve elements, fill rule and
    /// placement — the CPU never flattened it and nothing is rasterized
    /// or stored in the atlas. The record joins the frame's coverage
    /// state at apply time; the anchor instance in the frame's instance
    /// array supplies the drawing state the vertex stage joins to each
    /// emitted geometry record (water-rs/waterui#3081). The record is
    /// shared with the retained emission that drew it.
    GpuPath(Arc<GpuDraw>),
}

/// A GPU-routed path's draw record (water-rs/waterui#3081): the
/// placement key its coverage entry lives under plus everything a miss
/// regenerates from. A retained leaf shares it with each frame's
/// pending list, so a hit re-joins — or regenerates — the entry without
/// the lowering that produced it, and a re-placement rewrites it in
/// place.
pub struct GpuDraw {
    /// The coverage entry's retention key — exact when the surface clips
    /// the coverage hull.
    pub key: crate::render::path::CoverageKey,
    /// The draw's integer shift: an unclipped entry holds placement-relative
    /// coverage, so the draw adds its own integer placement offset back; a
    /// clipped entry holds absolute coverage and shifts by zero.
    pub shift: [i32; 2],
    /// `FillRule` as the kernel's `rule` field (0 non-zero, 1 even-odd).
    pub rule: u32,
    /// Kernel transform in the generation frame: the 2x2 plus the
    /// translation's fraction, or the full raster when clipped.
    pub raster: kurbo::Affine,
    /// Generation-frame clip rect `(x0, y0, x1, y1)`: `[0, surface]`
    /// shifted by `−offset` — a no-op for unclipped entries — or the
    /// surface rect itself when clipped.
    pub clip: [f32; 4],
    /// Encoded geometry and its storage bounds in the generation frame.
    pub path: crate::render::path::GpuPath,
}

impl GpuDraw {
    /// The draw of `content` under `raster` on a `surface`-pixel target,
    /// from its geometry `device` placed under that raster. Unclipped
    /// coverage is generated placement-relative — the raster keeps its
    /// translation's fraction and the integer offset moves to the draw's
    /// shift — so every integer offset shares one entry; coverage the
    /// surface clips stays exact-offset keyed at its true position.
    pub fn new(
        content: u64,
        rule: u32,
        raster: Affine,
        device: crate::render::path::GpuPath,
        surface: (u32, u32),
    ) -> Self {
        let placed = DrawPlacement::new(content, raster, device.bound, surface);
        Self {
            key: placed.key,
            shift: placed.shift,
            rule,
            raster: placed.raster,
            clip: placed.clip,
            path: device.with_bound(placed.bound),
        }
    }

    /// Re-places the draw in place under `raster`, whose device-frame
    /// bound for this geometry is `bound` — the same record
    /// [`GpuDraw::new`] builds there, without moving it.
    pub fn place(
        &mut self,
        raster: Affine,
        bound: crate::render::path::GpuBound,
        surface: (u32, u32),
    ) {
        let placed = DrawPlacement::new(self.key.content(), raster, bound, surface);
        self.key = placed.key;
        self.shift = placed.shift;
        self.raster = placed.raster;
        self.clip = placed.clip;
        self.path.bound = placed.bound;
    }
}

/// Everything about a [`GpuDraw`] its placement decides.
struct DrawPlacement {
    key: crate::render::path::CoverageKey,
    shift: [i32; 2],
    raster: Affine,
    clip: [f32; 4],
    bound: crate::render::path::GpuBound,
}

impl DrawPlacement {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "device coordinates are f32"
    )]
    fn new(
        content: u64,
        raster: Affine,
        bound: crate::render::path::GpuBound,
        surface: (u32, u32),
    ) -> Self {
        let [xx, yx, xy, yy, tx, ty] = raster.as_coeffs();
        let offset = Vec2::new(tx.floor(), ty.floor());
        let (width, height) = (surface.0 as f32, surface.1 as f32);
        if bound.clipped {
            return Self {
                key: crate::render::path::CoverageKey::new(content, raster, offset, surface, true),
                shift: [0, 0],
                raster,
                clip: [0.0, 0.0, width, height],
                bound,
            };
        }
        let (left, top) = (offset.x as f32, offset.y as f32);
        Self {
            key: crate::render::path::CoverageKey::new(content, raster, offset, surface, false),
            shift: [offset.x as i32, offset.y as i32],
            raster: Affine::new([xx, yx, xy, yy, tx - offset.x, ty - offset.y]),
            clip: [-left, -top, width - left, height - top],
            bound: bound.shifted(-offset),
        }
    }
}

/// The glyph's atlas cell. On a cache hit the entry is returned with
/// `None`; on a miss the cell is rasterized and queued in `pending` as
/// [`PendingRaster::Glyph`], and the returned entry's `x`/`y` stay zero
/// until the render thread stores the cell — the caller records an
/// instance patch against the pending index.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors glyph_key's inputs; grouping them would only rename the bundle"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "pending lists are tiny: u32 indexes them fine"
)]
pub fn entry(
    atlas: &Atlas,
    font: &FontData,
    key: GlyphKey,
    glyph_id: u32,
    size: f32,
    subpixel: (f32, f32),
    transform: Affine,
    coords: &[i16],
    pending: &mut Vec<PendingRaster>,
) -> Result<(Entry, Option<u32>), RenderError> {
    if let Some(entry) = atlas.get(&key) {
        return Ok((entry, None));
    }
    let cell = rasterize_texels(font, glyph_id, size, subpixel, transform, coords)?;
    let (left, top, w, h, texels) = cell.map_or((0, 0, 0, 0, Vec::new()), |c| {
        (c.left, c.top, c.w, c.h, c.texels)
    });
    let idx = pending.len() as u32;
    pending.push(PendingRaster::Glyph {
        key,
        left,
        top,
        w,
        h,
        texels,
    });
    Ok((
        Entry {
            x: 0,
            y: 0,
            w: w as u16,
            h: h as u16,
            left,
            top,
        },
        Some(idx),
    ))
}

/// Rasterizes the glyph's outline into coverage texels — pure CPU work
/// that touches no atlas state. `None` for glyphs with no outline.
#[expect(
    clippy::many_single_char_names,
    reason = "matrix coefficient names follow the Affine convention"
)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "coverage math runs in f64; cells, bearings and segments are bounded by the atlas size"
)]
fn rasterize_texels(
    font: &FontData,
    glyph_id: u32,
    size: f32,
    subpixel: (f32, f32),
    transform: Affine,
    coords: &[i16],
) -> Result<Option<CellRaster>, RenderError> {
    let font_ref = skrifa::FontRef::from_index(&font.data, font.index)
        .map_err(|e| RenderError::Font(format!("{e}")))?;
    let upem = font_ref
        .head()
        .map_err(|e| RenderError::Font(format!("head: {e}")))?
        .units_per_em();
    let outlines = font_ref.outline_glyphs();
    let Some(outline) = outlines.get(skrifa::GlyphId::new(glyph_id)) else {
        return Ok(None);
    };
    let location: Vec<F2Dot14> = coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let mut pen = PathPen {
        path: kurbo::BezPath::new(),
    };
    let settings = DrawSettings::unhinted(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::new(&location),
    );
    if outline.draw(settings, &mut pen).is_err() {
        return Ok(None);
    }
    if pen.path.is_empty() {
        return Ok(None);
    }
    // Font units to device pixels: y flips, scale is size per em, then the
    // run's transform's linear part.
    let scale = f64::from(size) / f64::from(upem);
    let [a, b, c, d, ..] = transform.as_coeffs();
    let m = Affine::new([a, b, c, d, 0.0, 0.0]) * Affine::scale_non_uniform(scale, -scale);
    let (fx, fy) = subpixel;
    // Flatten to 0.05 px in device space.
    let mut segments: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut bbox = kurbo::Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut last = kurbo::Point::ORIGIN;
    let mut start = kurbo::Point::ORIGIN;
    let offset = Vec2::new(f64::from(fx), f64::from(fy));
    let mut line = |p0: kurbo::Point, p1: kurbo::Point| {
        if p0 == p1 {
            return;
        }
        let a = m * p0 + offset;
        let b = m * p1 + offset;
        bbox = bbox.union_pt(a).union_pt(b);
        segments.push((a.x as f32, a.y as f32, b.x as f32, b.y as f32));
    };
    // Flatten to 0.05 px in device space: `m`'s largest column norm is the
    // worst-case factor a font-unit error grows by.
    let [ma, mb, mc, md, ..] = m.as_coeffs();
    let lmax = ma.hypot(mb).max(mc.hypot(md)).max(1e-9);
    let tol = 0.05 / lmax;
    // Every subpath is closed: an open contour is closed implicitly.
    kurbo::flatten(&pen.path, tol, |el| match el {
        PathEl::MoveTo(p) => {
            line(last, start);
            start = p;
            last = p;
        }
        PathEl::LineTo(p) => {
            line(last, p);
            last = p;
        }
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => unreachable!("flatten emits lines"),
        PathEl::ClosePath => {
            line(last, start);
            last = start;
        }
    });
    line(last, start);
    if segments.is_empty() || bbox.width() <= 0.0 || bbox.height() <= 0.0 {
        return Ok(None);
    }
    // Overlapping contours resolve to the union's boundary edges, like
    // `path::rasterize`: `None` keeps the glyph bit-identical.
    let resolved = cherenkov::lowering::resolve_winding(&segments, cherenkov::FillRule::NonZero)
        .into_resolved();
    let segments = resolved.as_deref().unwrap_or(&segments);
    let left = bbox.x0.floor() as i32 - 1;
    let top = bbox.y0.floor() as i32 - 1;
    let right = bbox.x1.ceil() as i32 + 1;
    let bottom = bbox.y1.ceil() as i32 + 1;
    let w = (right - left) as u32;
    let h = (bottom - top) as u32;
    // Rasterize in cell space.
    let mut raster = Raster::new(w as usize, h as usize);
    let ox = left as f32;
    let oy = top as f32;
    for &(x0, y0, x1, y1) in segments {
        raster.draw_line(x0 - ox, y0 - oy, x1 - ox, y1 - oy);
    }
    let coverage = raster.coverage();
    let texels: Vec<u8> = coverage.iter().copied().map(raster::texel).collect();
    Ok(Some(CellRaster {
        left,
        top,
        w,
        h,
        texels,
    }))
}

/// One glyph's unhinted outline in font units, or `None` when the font
/// has no outline for `id`.
pub fn outline(
    outlines: &skrifa::outline::OutlineGlyphCollection<'_>,
    coords: &[F2Dot14],
    id: u32,
) -> Result<Option<kurbo::BezPath>, RenderError> {
    let Some(glyph) = outlines.get(skrifa::GlyphId::new(id)) else {
        return Ok(None);
    };
    let mut pen = PathPen {
        path: kurbo::BezPath::new(),
    };
    glyph
        .draw(
            DrawSettings::unhinted(
                skrifa::instance::Size::unscaled(),
                skrifa::instance::LocationRef::new(coords),
            ),
            &mut pen,
        )
        .map_err(|e| RenderError::Font(format!("glyph {id}: {e}")))?;
    Ok(Some(pen.path))
}

/// A per-glyph transform must be finite and invertible.
pub fn checked_transform(glyph: &cherenkov::Glyph) -> Result<Affine, RenderError> {
    let t = glyph.transform.unwrap_or(Affine::IDENTITY);
    if !t.is_finite() || !t.inverse().is_finite() {
        return Err(RenderError::Render(
            "glyph transform must be finite and invertible".into(),
        ));
    }
    Ok(t)
}

/// How a glyph is placed: pure translations fold into the position and
/// keep the atlas path; anything else is realized as outline coverage.
pub enum GlyphPlacement {
    /// The glyph with a pure translation folded into `x`/`y`.
    Translate(cherenkov::Glyph),
    /// `translate(x, y) * transform`, applied before the font scale.
    Outline(Affine),
}

/// Validates `glyph.transform` and classifies its placement.
#[expect(clippy::float_cmp, reason = "exact identity coefficients")]
#[expect(
    clippy::many_single_char_names,
    reason = "a/b/c/d/e/f are the conventional affine coefficient names"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "glyph positions are f32 by design"
)]
pub fn classify(glyph: &cherenkov::Glyph) -> Result<GlyphPlacement, RenderError> {
    if glyph.transform.is_none() {
        return Ok(GlyphPlacement::Translate(*glyph));
    }
    let t = checked_transform(glyph)?;
    let [a, b, c, d, e, f] = t.as_coeffs();
    if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 {
        let mut glyph = *glyph;
        glyph.x += e as f32;
        glyph.y += f as f32;
        glyph.transform = None;
        return Ok(GlyphPlacement::Translate(glyph));
    }
    Ok(GlyphPlacement::Outline(
        Affine::translate((f64::from(glyph.x), f64::from(glyph.y))) * t,
    ))
}

/// Unhinted outlines in run coordinates for semantic glyph strokes.
/// Strokes use the font's outline, including on COLR fonts; palette paint
/// graphs apply only to filled glyphs. Missing outlines are explicit errors.
pub fn stroke_outlines(
    font: &FontData,
    run: &cherenkov::GlyphRun,
) -> Result<Vec<kurbo::BezPath>, RenderError> {
    let font_ref = skrifa::FontRef::from_index(&font.data, font.index)
        .map_err(|e| RenderError::Font(e.to_string()))?;
    let upem = font_ref
        .head()
        .map_err(|e| RenderError::Font(e.to_string()))?
        .units_per_em();
    if upem == 0 {
        return Err(RenderError::Font("zero units_per_em".into()));
    }
    let scale = f64::from(run.size) / f64::from(upem);
    let coords: Vec<F2Dot14> = run.coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let outlines = font_ref.outline_glyphs();
    let mut paths = Vec::with_capacity(run.glyphs.len());
    for glyph in run.glyphs.iter() {
        let path = outline(&outlines, &coords, glyph.id)?.ok_or_else(|| {
            RenderError::Font(format!("glyph {} has no stroke outline", glyph.id))
        })?;
        let placement = Affine::translate((f64::from(glyph.x), f64::from(glyph.y)))
            * checked_transform(glyph)?
            * Affine::scale_non_uniform(scale, -scale);
        paths.push(placement * path);
    }
    Ok(paths)
}

/// Conservative outline bounds in the run's local coordinates, before rasterization.
pub fn run_bounds(font: &FontData, run: &cherenkov::GlyphRun) -> Result<kurbo::Rect, RenderError> {
    let scale = f64::from(run.size) / f64::from(font.units_per_em);
    if run.coords.iter().all(|coord| *coord == 0) {
        // The head table encloses every glyph at the default location.
        // Culling needs only a conservative box, not individual outlines.
        let Some(first) = run.glyphs.first() else {
            return Ok(kurbo::Rect::ZERO);
        };
        let mut origins = kurbo::Rect::new(
            f64::from(first.x),
            f64::from(first.y),
            f64::from(first.x),
            f64::from(first.y),
        );
        for glyph in &run.glyphs[1..] {
            origins = origins.union_pt((f64::from(glyph.x), f64::from(glyph.y)));
        }
        return Ok(kurbo::Rect::new(
            font.default_bounds.x0.mul_add(scale, origins.x0),
            font.default_bounds.y1.mul_add(-scale, origins.y0),
            font.default_bounds.x1.mul_add(scale, origins.x1),
            font.default_bounds.y0.mul_add(-scale, origins.y1),
        ));
    }
    // A non-default variable-font location can extend outside the head box.
    let font_ref = std::cell::OnceCell::new();
    let coords = std::cell::OnceCell::new();
    let metrics = std::cell::OnceCell::new();
    let mut instances = font.bounds.borrow_mut();
    let index = instances
        .iter()
        .position(|instance| instance.coords == run.coords)
        .unwrap_or_else(|| {
            instances.push(InstanceBounds {
                coords: run.coords.clone(),
                glyphs: Vec::new(),
            });
            instances.len() - 1
        });
    let cached = &mut instances[index].glyphs;
    let mut bounds = None::<kurbo::Rect>;
    for glyph in run.glyphs.iter() {
        let b = match cached.binary_search_by_key(&glyph.id, |entry| entry.0) {
            Ok(index) => cached[index].1,
            Err(index) => {
                let parsed = font_ref
                    .get_or_init(|| skrifa::FontRef::from_index(&font.data, font.index))
                    .as_ref()
                    .map_err(|e| RenderError::Font(format!("{e}")))?;
                let metrics = metrics.get_or_init(|| {
                    let coords: &Vec<F2Dot14> = coords.get_or_init(|| {
                        run.coords.iter().map(|c| F2Dot14::from_bits(*c)).collect()
                    });
                    parsed.glyph_metrics(
                        skrifa::instance::Size::unscaled(),
                        skrifa::instance::LocationRef::new(coords),
                    )
                });
                let bounds = metrics.bounds(skrifa::GlyphId::new(glyph.id)).map(|b| {
                    kurbo::Rect::new(
                        f64::from(b.x_min),
                        f64::from(b.y_min),
                        f64::from(b.x_max),
                        f64::from(b.y_max),
                    )
                });
                cached.insert(index, (glyph.id, bounds));
                bounds
            }
        };
        let Some(b) = b else {
            continue;
        };
        if b.is_zero_area() {
            continue;
        }
        let x = f64::from(glyph.x);
        let y = f64::from(glyph.y);
        let b = kurbo::Rect::new(
            b.x0.mul_add(scale, x),
            b.y1.mul_add(-scale, y),
            b.x1.mul_add(scale, x),
            b.y0.mul_add(-scale, y),
        );
        bounds = Some(bounds.map_or(b, |bounds| bounds.union(b)));
    }
    Ok(bounds.unwrap_or(kurbo::Rect::ZERO))
}

/// The cache key for a glyph at a quantized device position.
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
pub fn glyph_key(
    run: &cherenkov::GlyphRun,
    glyph: u32,
    subpixel: (f32, f32),
    transform: Affine,
) -> GlyphKey {
    let mut hasher = DefaultHasher::new();
    run.coords.hash(&mut hasher);
    let [a, b, c, d, ..] = transform.as_coeffs();
    GlyphKey {
        font: run.font.raw(),
        glyph,
        size_bits: (run.size * 64.0).round() as u32,
        subpixel: u64::from(subpixel.0.to_bits()) | (u64::from(subpixel.1.to_bits()) << 32),
        matrix: [
            (a as f32).to_bits(),
            (b as f32).to_bits(),
            (c as f32).to_bits(),
            (d as f32).to_bits(),
        ],
        coords_hash: hasher.finish(),
    }
}

/// The atlas's map lookup.
impl Atlas {
    pub fn get(&self, key: &GlyphKey) -> Option<Entry> {
        self.map.get(key).map(|cached| cached.entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_run_key_keeps_exact_identity_at_each_glyph_position() {
        let mut run = cherenkov::GlyphRun {
            font: cherenkov::FontId::new(91),
            size: 19.25,
            coords: Vec::new().into(),
            glyphs: Vec::new().into(),
            style: cherenkov::GlyphStyle::Fill,
        };
        for coords in [vec![], vec![0], vec![i16::MIN, 123, i16::MAX]] {
            run.coords = coords.into();
            for transform in [
                Affine::IDENTITY,
                Affine::new([1.5, -0.0, 0.25, 2.0, 7.0, -8.0]),
            ] {
                let base = glyph_key(&run, 0, (0.0, 0.0), transform);
                for glyph in [0, 1, 65_535, u32::MAX] {
                    for position in [(0.0, 0.0), (0.25, 0.5), (0.75, 0.25)] {
                        assert_eq!(
                            base.at(glyph, position),
                            glyph_key(&run, glyph, position, transform)
                        );
                        let key = base.at(glyph, position);
                        let mut actual = DefaultHasher::new();
                        key.hash(&mut actual);
                        let mut expected = DefaultHasher::new();
                        key.font.hash(&mut expected);
                        key.glyph.hash(&mut expected);
                        key.size_bits.hash(&mut expected);
                        key.subpixel.hash(&mut expected);
                        key.matrix.hash(&mut expected);
                        key.coords_hash.hash(&mut expected);
                        assert_eq!(actual.finish(), expected.finish());
                    }
                }
            }
        }
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

    /// Two parallel lowerings that both miss the same glyphs produce the
    /// same atlas as serial lowering: pending rasters commit in
    /// dirty-surface order and a duplicated key resolves to the first
    /// surface's cell.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "the serial reference and the parallel run share the helpers but need both sequences"
    )]
    fn pending_apply_matches_serial_atlas() {
        /// Lower `keys` against `atlas`, collecting the pending rasters.
        fn lower(atlas: &Atlas, font: &FontData, keys: &[GlyphKey]) -> Vec<PendingRaster> {
            let mut pending = Vec::new();
            for &k in keys {
                entry(
                    atlas,
                    font,
                    k,
                    k.glyph,
                    16.0,
                    (0.0, 0.0),
                    Affine::IDENTITY,
                    &[],
                    &mut pending,
                )
                .expect("entry");
            }
            pending
        }
        /// Commit one pending list, the way `apply_raster` does.
        fn apply(
            atlas: &mut Atlas,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            pending: Vec<PendingRaster>,
        ) {
            for raster in pending {
                let PendingRaster::Glyph {
                    key,
                    left,
                    top,
                    w,
                    h,
                    texels,
                } = raster
                else {
                    unreachable!("glyph pending only")
                };
                assert!(
                    atlas
                        .store_glyph(device, queue, key, left, top, w, h, &texels)
                        .is_some()
                );
            }
        }
        let Some((device, queue)) = device_and_queue() else {
            return;
        };
        let parsed =
            skrifa::FontRef::from_index(include_bytes!("../../../scenes/fonts/NotoSans.ttf"), 0)
                .expect("test font");
        let head = parsed.head().expect("test font head");
        let font = FontData {
            data: include_bytes!("../../../scenes/fonts/NotoSans.ttf")
                .as_slice()
                .into(),
            index: 0,
            units_per_em: head.units_per_em(),
            default_bounds: kurbo::Rect::new(
                f64::from(head.x_min()),
                f64::from(head.y_min()),
                f64::from(head.x_max()),
                f64::from(head.y_max()),
            ),
            has_colr: false,
            has_bitmap: false,
            bitmap: None,
            colr: std::cell::RefCell::new(FxHashMap::default()),
            bounds: std::cell::RefCell::default(),
        };
        let key = |glyph: u32| GlyphKey {
            font: 7,
            glyph,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        // Surface A draws glyphs {1, 2, 3}; surface B overlaps on {2, 3, 4}.
        let surfaces = [[key(1), key(2), key(3)], [key(2), key(3), key(4)]];
        // Serial reference: each surface's pending commits before the next
        // lowers, so its lookups hit the earlier surface's cells.
        let mut serial = Atlas::new(&device, u64::MAX);
        for keys in &surfaces {
            let pending = lower(&serial, &font, keys);
            apply(&mut serial, &device, &queue, pending);
        }
        // Parallel: both lowerings read the same empty atlas, so every
        // miss becomes a pending raster — duplicates included — then the
        // render thread commits them in dirty-surface order.
        let mut parallel = Atlas::new(&device, u64::MAX);
        let batches: Vec<_> = surfaces
            .iter()
            .map(|keys| lower(&parallel, &font, keys))
            .collect();
        for pending in batches {
            apply(&mut parallel, &device, &queue, pending);
        }
        for keys in &surfaces {
            for &k in keys {
                assert_eq!(
                    serial.get(&k),
                    parallel.get(&k),
                    "glyph {}'s cell differs",
                    k.glyph
                );
            }
        }
        // A pending mask dedupes the same way.
        let mask = MaskCell {
            device: [0.0, 0.0],
            atlas: [0.0, 0.0],
            size: [2.0, 2.0],
            rect: [0.0, 0.0, 2.0, 2.0],
            slot: 0,
        };
        assert!(
            parallel
                .store_mask(&device, &queue, 0xdead, mask, 2, 2, &[255u8; 4])
                .is_some()
        );
        let stored = parallel.mask_origin(0xdead).expect("stored");
        assert!(
            parallel
                .store_mask(&device, &queue, 0xdead, mask, 2, 2, &[0u8; 4])
                .is_some(),
            "duplicate mask resolves to the stored cell"
        );
        assert_eq!(parallel.mask_origin(0xdead), Some(stored));
    }

    /// A glyph with no outline is cached without a cell, so it names no
    /// shelf (#2327). Parallel lowerings read an atlas that does not see
    /// their own admissions, so one batch can carry the same outline-less
    /// glyph twice; the second apply is a hit, and a hit pins its shelf.
    /// On an atlas with no shelves yet — the first commit, or the first
    /// after a resize cleared the layout — that pin must not index one.
    #[test]
    fn an_outline_less_glyph_names_no_shelf() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let mut atlas = Atlas::new(&device, u64::MAX);
        let key = |glyph: u32| GlyphKey {
            font: 7,
            glyph,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        let (space, ink) = (key(3), key(4));
        let other = GlyphKey { font: 8, ..ink };
        let mut writes = CellWrites::default();
        // One batch on an empty layout: the outline-less glyph twice
        // before any cell opens a shelf.
        atlas.begin_commit(&[]);
        assert_eq!(
            atlas.place_glyph(space, 0, 0, 0, 0, Vec::new(), &mut writes),
            Some(((0, 0), None))
        );
        assert_eq!(
            atlas.place_glyph(space, 0, 0, 0, 0, Vec::new(), &mut writes),
            Some(((0, 0), None)),
            "the duplicate resolves to the cached outline-less glyph"
        );
        assert!(writes.is_empty(), "an outline-less glyph writes no texels");
        assert!(atlas.layout.shelves.is_empty());
        // A later commit opens shelf 0; hitting the outline-less glyph
        // again must not pin it, so an evicting commit may still reclaim
        // it.
        assert!(
            atlas
                .place_glyph(ink, 0, 0, 4, 4, vec![0; 16], &mut writes)
                .is_some()
        );
        assert!(
            atlas
                .place_glyph(other, 0, 0, 4, 4, vec![0; 16], &mut writes)
                .is_some()
        );
        atlas.begin_commit(&[]);
        let pinned = atlas.layout.shelves[0].last_used;
        assert!(
            atlas
                .place_glyph(space, 0, 0, 0, 0, Vec::new(), &mut writes)
                .is_some()
        );
        assert_eq!(
            atlas.layout.shelves[0].last_used, pinned,
            "an outline-less hit pins no shelf"
        );
        // Dropping the font drops the outline-less glyph with it, and
        // releases only the bytes its cells were charged: the other
        // font's cell keeps its shelf key and its share.
        atlas.remove_font(7);
        assert!(atlas.get(&space).is_none() && atlas.get(&ink).is_none());
        assert_eq!(
            atlas.slot_keys[0].iter().copied().collect::<Vec<_>>(),
            [live_hash(&other)]
        );
        assert_eq!(atlas.cpu_bytes(), 16 + LIVE_ENTRY_BYTES);
        // The same after a reset: a fresh layout holds only the
        // outline-less glyph, and dropping its font touches no shelf.
        atlas.clear();
        atlas.begin_commit(&[]);
        assert!(
            atlas
                .place_glyph(space, 0, 0, 0, 0, Vec::new(), &mut writes)
                .is_some()
        );
        atlas.remove_font(7);
        assert!(atlas.get(&space).is_none());
    }

    /// Masks over `MASK_TEXTURE_TEXELS` leave the atlas for a dedicated
    /// texture.
    #[test]
    fn mask_in_atlas_bounds() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let atlas = Atlas::new(&device, u64::MAX);
        assert!(atlas.mask_in_atlas(200, 200));
        assert!(!atlas.mask_in_atlas(300, 300));
        assert!(atlas.mask_texture_fits(4096, 4096));
    }

    /// An evicting commit reclaims the coldest untouched shelf and drops
    /// its entries; shelves the lowering's hits sit on survive (#119).
    #[test]
    fn evicting_commit_reclaims_cold_shelves() {
        let Some((device, queue)) = device_and_queue() else {
            return;
        };
        let mut atlas = Atlas::new(&device, u64::MAX);
        atlas.size = 64;
        let key = |glyph: u32| GlyphKey {
            font: 7,
            glyph,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        let (key_a, key_b, key_c, key_e) = (key(1), key(2), key(3), key(5));
        // A class-8 shelf, a packed class-16 shelf, a class-24 shelf and
        // a second class-16 shelf take `top` to the atlas edge.
        assert!(
            atlas
                .store_glyph(&device, &queue, key_a, 0, 0, 4, 4, &[0; 16])
                .is_some()
        );
        assert!(
            atlas
                .store_glyph(&device, &queue, key_b, 0, 0, 60, 10, &[0; 600])
                .is_some()
        );
        assert!(
            atlas
                .store_glyph(&device, &queue, key_c, 0, 0, 4, 20, &[0; 80])
                .is_some()
        );
        assert!(
            atlas
                .store_glyph(&device, &queue, key_e, 0, 0, 60, 10, &[0; 600])
                .is_some()
        );
        assert_eq!(
            atlas.layout.tops.iter().copied().sum::<u32>(),
            64,
            "the layout is full"
        );
        let shelf = |key: &GlyphKey| atlas.map[key].shelf.expect("an outlined glyph has a shelf");
        let [slot_a, slot_b, slot_c, slot_e] =
            [shelf(&key_a), shelf(&key_b), shelf(&key_c), shelf(&key_e)];
        let epoch_a = atlas.shelf_epoch(slot_a);
        let epoch_b = atlas.shelf_epoch(slot_b);
        // Commit 2: the hits pin A, C and E; B's shelf is untouched. A
        // class-16 cell cannot fit live or virgin space, so eviction
        // must reclaim B's band rather than any pinned shelf.
        atlas.begin_commit(&[slot_a, slot_c, slot_e]);
        atlas.enable_evicting();
        let mut writes = CellWrites::default();
        let key_d = key(4);
        assert!(
            atlas
                .place_glyph(key_d, 0, 0, 60, 10, vec![0; 600], &mut writes)
                .is_some(),
            "eviction must reclaim B's class-16 band"
        );
        assert!(
            atlas.get(&key_b).is_none(),
            "the cold shelf's entry is gone"
        );
        assert_ne!(
            atlas.shelf_epoch(slot_b),
            epoch_b,
            "a dead band's epoch moves so stale references fail"
        );
        assert!(
            atlas.get(&key_a).is_some()
                && atlas.get(&key_c).is_some()
                && atlas.get(&key_e).is_some(),
            "touched shelves keep their entries"
        );
        assert_eq!(
            atlas.shelf_epoch(slot_a),
            epoch_a,
            "a surviving band keeps its epoch"
        );
    }

    /// A retained emission's stored UVs name only the shelf each quad's
    /// own fragment samples: a `KIND_REGION` interval widened across
    /// path cells spans shelves, so the UV scan pins the first band and
    /// leaves the rest evictable. The `(slot, epoch)` refs folded at
    /// apply already record every band the emission samples, so
    /// `replay_pins` pins the still-live ones too — otherwise an
    /// evicting commit reclaims a band the frame still draws (#2909).
    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "atlas texel origins are small integers — exact in f32"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "the atlas layout, a real lowering and the commit sequence each need their full setup"
    )]
    fn replay_pins_hold_every_band_a_region_emission_samples() {
        use cherenkov::Draw as _;
        use cherenkov::testing::LayerOp;

        let (mut renderer, _info) = match crate::render::init(crate::GpuConfig::default()) {
            Ok(pair) => pair,
            Err(e) => panic!("the replay-pin commit needs a GPU adapter: {e}"),
        };
        let device = renderer.device.clone();
        let queue = renderer.queue.clone();
        renderer.atlas.size = 64;
        let key = |glyph: u32| GlyphKey {
            font: 7,
            glyph,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        // The layout of `evicting_commit_reclaims_cold_shelves`: a
        // class-8 shelf, a packed class-16 shelf, a class-24 shelf and
        // a second class-16 shelf take `top` to the atlas edge.
        let (key_a, key_b, key_c, key_d) = (key(1), key(2), key(3), key(4));
        let origin_a = renderer
            .atlas
            .store_glyph(&device, &queue, key_a, 0, 0, 4, 4, &[0; 16])
            .expect("the first cell places");
        assert!(
            renderer
                .atlas
                .store_glyph(&device, &queue, key_b, 0, 0, 60, 10, &[0; 600])
                .is_some()
        );
        assert!(
            renderer
                .atlas
                .store_glyph(&device, &queue, key_c, 0, 0, 4, 20, &[0; 80])
                .is_some()
        );
        assert!(
            renderer
                .atlas
                .store_glyph(&device, &queue, key_d, 0, 0, 60, 10, &[0; 600])
                .is_some()
        );
        assert_eq!(
            renderer.atlas.layout.tops.iter().copied().sum::<u32>(),
            64,
            "the layout is full"
        );
        let slot_a = renderer.atlas.map[&key_a].shelf.expect("a shelved cell");
        let slot_b = renderer.atlas.map[&key_b].shelf.expect("a shelved cell");
        let epoch_a = renderer.atlas.shelf_epoch(slot_a);
        let epoch_b = renderer.atlas.shelf_epoch(slot_b);

        // One layer's fill lowered for real, then edited into the
        // retained state `replay_pins` consumes on the next commit: a
        // region emission whose stored quad samples band A while its
        // recorded `(slot, epoch)` refs span bands A and B.
        let layer = cherenkov::LayerId::new(1);
        let mut tree = cherenkov::SurfaceTree::new();
        tree.apply(LayerOp::Create(layer));
        tree.apply(LayerOp::Push {
            parent: tree.root(),
            child: layer,
        });
        let mut caches: FxHashMap<_, crate::render::lower::ContentData> = std::iter::once((
            layer,
            crate::render::lower::ContentData::new(cherenkov::Picture::record(|c| {
                c.fill(
                    kurbo::Rect::new(0.0, 0.0, 8.0, 8.0),
                    cherenkov::WorkingColor::WHITE,
                );
            })),
        ))
        .collect();
        let fonts = FxHashMap::default();
        let images = FxHashMap::default();
        let bitmaps = FxHashMap::default();
        let glyphs = crate::render::lower::GlyphContext {
            atlas: &renderer.atlas,
            live_stamp: renderer.atlas.live_stamp(),
            fonts: &fonts,
            images: &images,
            bitmaps: &bitmaps,
            content: &FxHashMap::default(),
        };
        let mut frame = crate::render::lower::Frame::default();
        let mut anchor_scratch = crate::render::lower::AnchorScratch::default();
        let mut raster_scratch = crate::render::path::EmitScratch::new();
        let mut lowering = crate::render::lower::Lowering::new(
            &mut frame,
            (64, 64),
            &mut anchor_scratch,
            &mut raster_scratch,
            Some(crate::render::gpu_cov::CoverageLimit::new(&device.limits())),
        );
        lowering.prepare(&mut caches, &glyphs).expect("prepared");
        let plan = crate::render::planes::Plan {
            planes: Vec::new(),
            rejected: Vec::new(),
            unplaced: Vec::new(),
            trailing: false,
        };
        lowering
            .run(
                &tree,
                &mut caches,
                cherenkov::WorkingColor::BLACK,
                &glyphs,
                &FxHashMap::default(),
                FxHashMap::default(),
                &plan,
            )
            .expect("lowered");
        {
            let content = caches.get_mut(&layer).expect("the layer lowered");
            let (_, emissions) = content.retained.prepared();
            let emission = emissions
                .iter_mut()
                .find_map(|entry| entry.data.as_mut())
                .expect("the fill realized an emission");
            let inst = &mut content.storage.instances[emission.instances.start];
            inst.kind = crate::render::instance::KIND_REGION;
            inst.uv = [origin_a.0 as f32 + 0.5, origin_a.1 as f32 + 0.5];
            let first = content.storage.refs.len();
            content
                .storage
                .refs
                .extend([(slot_a, epoch_a), (slot_b, epoch_b)]);
            emission.refs = crate::render::lower::Emission::pack_refs(first, 2);
            emission.clear_pending_cells();
        }

        let (target, view) = crate::render::create_target(
            &device,
            "pin test surface",
            (64, 64),
            crate::render::TARGET_USAGES,
            crate::render::WORKING_FORMAT,
        );
        let mut surf = crate::render::SurfaceState::new(
            None,
            None,
            None,
            cherenkov::DEFAULT_REFRESH,
            (target, view),
            cherenkov::testing::unhosted_waker(),
            false,
        );
        surf.layers = caches;
        let mut pending = vec![surf];
        let results = vec![Ok(crate::render::lower::Lowered::default())];
        let mut touches = Vec::new();
        renderer.replay_pins(&mut pending, &results, &mut touches);
        assert!(
            touches.contains(&slot_b),
            "every still-live band the emission samples is pinned"
        );

        // The evicting commit reclaims the coldest unpinned band; with
        // B pinned that is C, without the pin it is B itself.
        renderer.atlas.begin_commit(&touches);
        renderer.atlas.enable_evicting();
        let mut writes = CellWrites::default();
        let key_e = key(5);
        assert!(
            renderer
                .atlas
                .place_glyph(key_e, 0, 0, 60, 10, vec![0; 600], &mut writes)
                .is_some(),
            "reclaiming a cold band makes room"
        );
        assert!(
            renderer.atlas.shelf_matches(slot_b, epoch_b),
            "the band the emission still samples survives eviction"
        );
        assert!(
            renderer.atlas.get(&key_b).is_some(),
            "its entries stay live"
        );
    }

    /// A batch whose cells exceed one `cap` page appends atlas pages
    /// instead of reporting exhaustion (#211): the fresh-layout grow
    /// probe places every cell on blank pages, so the admitted cells
    /// land past `size` in the taller texture and the entries address
    /// them there.
    #[test]
    fn an_over_cap_batch_appends_atlas_pages() {
        let Some((device, queue)) = device_and_queue() else {
            return;
        };
        // cap 4096 with `budget / 8` holding two pages.
        let mut atlas = Atlas::new(&device, 256 * 1024 * 1024);
        if atlas.max_pages(4096) < 2 {
            return;
        }
        let key = |glyph: u32| GlyphKey {
            font: 7,
            glyph,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        // 512² cells: seven per shelf, seven shelves per page — 49 on
        // one `cap` page, so the batch of 70 fits only on two.
        let mut rasters = Vec::new();
        for glyph in 0..70u32 {
            rasters.push(PendingRaster::Glyph {
                key: key(glyph),
                left: 0,
                top: 0,
                w: 512,
                h: 512,
                texels: vec![0x80; 512 * 512],
            });
        }
        let refs: Vec<&PendingRaster> = rasters.iter().collect();
        let AtlasPlan::Resize(size, pages) = atlas.plan(&refs) else {
            panic!("expected a page grow");
        };
        assert_eq!((size, pages), (4096, 2));
        atlas.resize_to(&device, size, pages);
        assert_eq!(atlas.pages(), 2);
        for raster in &rasters {
            let PendingRaster::Glyph {
                key,
                left,
                top,
                w,
                h,
                texels,
            } = raster
            else {
                unreachable!("glyph pending only")
            };
            assert!(
                atlas
                    .store_glyph(&device, &queue, *key, *left, *top, *w, *h, texels)
                    .is_some(),
                "every cell of a planned batch must store"
            );
        }
        let top_y = rasters
            .iter()
            .filter_map(|r| match r {
                PendingRaster::Glyph { key, .. } => atlas.get(key).map(|e| u32::from(e.y)),
                _ => None,
            })
            .max();
        assert!(
            top_y.unwrap_or(0) >= 4096,
            "cells past the first page address y >= 4096"
        );
    }

    /// Once a paged atlas's live batch packs onto fewer pages, `plan`
    /// returns the smaller target so the peak texture is released
    /// (#211): the cells a repacked commit places — hits and misses
    /// alike — probe a fresh layout, while a batch still needing every
    /// page keeps them all.
    #[test]
    fn a_smaller_batch_releases_atlas_pages() {
        let Some((device, queue)) = device_and_queue() else {
            return;
        };
        let mut atlas = Atlas::new(&device, 256 * 1024 * 1024);
        if atlas.max_pages(4096) < 2 {
            return;
        }
        let key = |glyph: u32| GlyphKey {
            font: 7,
            glyph,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        let texels = vec![0x80; 512 * 512];
        let cell = |glyph: u32| PendingRaster::Glyph {
            key: key(glyph),
            left: 0,
            top: 0,
            w: 512,
            h: 512,
            texels: texels.clone(),
        };
        let peak: Vec<PendingRaster> = (0..70u32).map(cell).collect();
        let refs: Vec<&PendingRaster> = peak.iter().collect();
        let AtlasPlan::Resize(size, pages) = atlas.plan(&refs) else {
            panic!("expected a page grow");
        };
        atlas.resize_to(&device, size, pages);
        assert_eq!(atlas.pages(), 2);
        let bytes_peak = atlas.gpu_bytes();
        let old_origin = atlas
            .store_glyph(&device, &queue, key(100), 0, 0, 512, 512, &texels)
            .expect("cached glyph");
        // The pending key must preserve this hit even without a touched shelf.
        atlas.begin_commit(&[]);

        // A batch of 40 cells still needs them stored — but packs on
        // one page, so the second page must be released.
        let quiet: Vec<PendingRaster> = (100..140u32).map(cell).collect();
        let refs: Vec<&PendingRaster> = quiet.iter().collect();
        let AtlasPlan::Resize(size, pages) = atlas.plan(&refs) else {
            panic!("a smaller batch must shrink the atlas");
        };
        assert_eq!((size, pages), (4096, 1));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let relocation = atlas
            .relocate(&device, &mut encoder, size, pages)
            .expect("shrink fits");
        assert_eq!(relocation.moved(), 1);
        let moved = relocation
            .cell(old_origin.0, old_origin.1)
            .expect("cached glyph migrated");
        let retained = atlas.get(&key(100)).expect("pending hit retained");
        assert_eq!(
            (u32::from(retained.x), u32::from(retained.y)),
            (moved.x, moved.y)
        );
        queue.submit([encoder.finish()]);
        assert_eq!(atlas.pages(), 1);
        assert!(atlas.gpu_bytes() < bytes_peak);
        for glyph in 100..140 {
            assert!(
                atlas
                    .store_glyph(&device, &queue, key(glyph), 0, 0, 512, 512, &texels)
                    .is_some(),
                "the shrunken atlas still holds the whole batch"
            );
        }
        let max_y = (100..140)
            .map(|glyph| u32::from(atlas.get(&key(glyph)).expect("stored glyph").y))
            .max();
        assert!(max_y.unwrap_or(0) < 4096, "no cell lands past page one");

        // A batch whose stored-plus-pending cells still need both pages
        // keeps them: 40 cached + 50 new = 90, which two pages hold.
        let peak: Vec<PendingRaster> = (200..250u32).map(cell).collect();
        let refs: Vec<&PendingRaster> = peak.iter().collect();
        let AtlasPlan::Resize(size, pages) = atlas.plan(&refs) else {
            panic!("expected a regrow");
        };
        atlas.resize_to(&device, size, pages);
        assert_eq!(atlas.pages(), 2);
        let again: Vec<PendingRaster> = (300..340u32).map(cell).collect();
        let refs: Vec<&PendingRaster> = again.iter().collect();
        let AtlasPlan::Resize(size, pages) = atlas.plan(&refs) else {
            panic!("a one-page batch must release the second page");
        };
        assert_eq!((size, pages), (4096, 1));
    }

    /// A batch whose strip cells are wider than the live edge grows the
    /// atlas even when the whole set packs at no probed shape: `plan`
    /// returns the shape the cells' real shelf footprint needs rather
    /// than recycling in place on an edge the cells can never fit
    /// (#234). Before the fix this returned `Recycle`, and the evicting
    /// commit failed the first cell wider than the 1024 edge.
    #[test]
    fn an_unplaceable_edge_still_grows() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let mut atlas = Atlas::new(&device, 256 * 1024 * 1024);
        // The batch needs an adapter that pages at 4096 and reaches the
        // 8192 edge — a smaller `max_texture_dimension_2d` cannot carry
        // the regression.
        if atlas.max_pages(4096) < 2 {
            return;
        }
        // 4000 strip cells of 1600×4 — every one wider than the 1024
        // live edge, and ~51M shelf texels of footprint together: past
        // every shape up to 4096×2 pages, inside the 8192² page.
        let count = 4000usize;
        let (cw, ch) = (1600u32, 4u32);
        let raster = PendingRaster::Path {
            key: 1,
            emit: PathEmit {
                cells: (0..count)
                    .map(|_| PathCell {
                        rect: [0.0, 0.0, 1600.0, 4.0],
                        x: 0,
                        y: 0,
                        slot: 0,
                        interior: 0,
                    })
                    .collect(),
                ..PathEmit::default()
            },
            cells: {
                let n = (cw * ch) as usize;
                (0..count).map(|i| (cw, ch, i * n..(i + 1) * n)).collect()
            },
        };
        let refs = [&raster];
        let AtlasPlan::Resize(size, pages) = atlas.plan(&refs) else {
            panic!("cells wider than the live edge must grow the atlas");
        };
        assert!(size >= 2048, "the grown edge must take a 1600-wide cell");
        atlas.resize_to(&device, size, pages);
        atlas.begin_commit(&[]);
        let mut writes = CellWrites::default();
        let PendingRaster::Path { key, emit, cells } = raster else {
            unreachable!("path pending only")
        };
        let arena = writes.push_arena(vec![0x80; (cw * ch) as usize * count]);
        assert!(
            atlas
                .place_path(key, emit, cells, arena, &mut writes)
                .is_some(),
            "every cell of a batch that grew for its edge must place"
        );
    }

    /// A batch that genuinely exceeds the largest shape the device
    /// allows still recycles: the fail-fast `AtlasExhausted` answer for
    /// a frame bigger than `cap × max_pages` is kept (#234).
    #[test]
    fn an_over_capacity_batch_still_recycles() {
        let Some((device, _queue)) = device_and_queue() else {
            return;
        };
        let mut atlas = Atlas::new(&device, 256 * 1024 * 1024);
        if atlas.max_pages(4096) < 2 {
            return;
        }
        // 6000 strip cells of 1600×4: ~77M shelf texels, past the
        // 8192² page the cap allows on this adapter.
        let count = 6000usize;
        let (cw, ch) = (1600u32, 4u32);
        let raster = PendingRaster::Path {
            key: 1,
            emit: PathEmit {
                cells: (0..count)
                    .map(|_| PathCell {
                        rect: [0.0, 0.0, 1600.0, 4.0],
                        x: 0,
                        y: 0,
                        slot: 0,
                        interior: 0,
                    })
                    .collect(),
                ..PathEmit::default()
            },
            cells: vec![(cw, ch, 0..(cw * ch) as usize); count],
        };
        let refs = [&raster];
        assert!(
            matches!(atlas.plan(&refs), AtlasPlan::Recycle),
            "a batch exceeding cap × max_pages must still recycle"
        );
    }

    #[test]
    fn clear_drops_every_arena_with_the_commit() {
        let mut writes = CellWrites::default();
        for _ in 0..8 {
            for _ in 0..64 {
                writes.push_arena(vec![0xAB; 256]);
            }
            writes.clear();
            // `clear` empties the staged arenas; the list capacities
            // stay for reuse, so `heap_bytes` still counts them.
            assert_eq!(writes.arena_heap_bytes(), 0);
        }
    }

    /// The upload consumes its batch: the arenas it copied into GPU
    /// staging are dropped with the call, so the container the renderer
    /// keeps across the idle span that follows — the emptied shell the
    /// next commit refills — retains no source-arena bytes (#2671).
    #[test]
    fn upload_releases_the_source_arenas() {
        let Some((device, queue)) = device_and_queue() else {
            return;
        };
        let mut atlas = Atlas::new(&device, u64::MAX);
        let key = GlyphKey {
            font: 7,
            glyph: 3,
            size_bits: (16.0f32 * 64.0).to_bits(),
            subpixel: 0,
            matrix: [
                1.0f32.to_bits(),
                0.0f32.to_bits(),
                0.0f32.to_bits(),
                1.0f32.to_bits(),
            ],
            coords_hash: 0,
        };
        let mut writes = CellWrites::default();
        atlas.begin_commit(&[]);
        assert!(
            atlas
                .place_glyph(key, 0, 0, 4, 4, vec![0xAB; 16], &mut writes)
                .is_some()
        );
        assert!(writes.heap_bytes() > 0);
        let writes = atlas.upload_committed(&device, &queue, writes);
        assert_eq!(
            writes.arena_heap_bytes(),
            0,
            "no source-arena capacity outlives the upload"
        );
        assert!(writes.is_empty(), "the returned shell stages nothing");
        // The emptied shell keeps its `cells`/`arenas` list capacities
        // for the next commit — `heap_bytes` reports them (#2711).
        assert!(
            writes.heap_bytes() > 0,
            "the reusable list capacity is retained by design"
        );

        // An arena staged without a placed cell ends with the upload too.
        let mut writes = writes;
        writes.push_arena(vec![0xCD; 64]);
        let writes = atlas.upload_committed(&device, &queue, writes);
        assert_eq!(
            writes.arena_heap_bytes(),
            0,
            "a cell-less arena is not retained"
        );
    }
}
