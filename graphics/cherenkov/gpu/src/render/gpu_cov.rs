//! The GPU coverage compute route of issue #2543, decision S2: one set
//! of dispatches in the frame's own encoder, ahead of its raster
//! passes, in the same submission, without readback or a fence.
//!
//! The lowering leaves each routed path's curve elements, fill rule,
//! transform and an anchor instance in a [`PendingRaster::GpuPath`]
//! record. `apply_pending` joins them into this frame context, which
//! allocates one block in the coverage arena per path and accumulates
//! the per-path dispatch records and the decision-R2 bound they size
//! from. The kernels write only geometry: each block carries a count
//! word, the emitted 24-byte geometry records and the packed R8 texels.
//! The cell-buffer vertex stage joins each record to the draw's anchor
//! instance (water-rs/waterui#3081); the draw loops read each draw's
//! indirect arguments from `draws`, whose count words arrive through
//! 4-byte buffer copies.

use cherenkov::RenderError;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::render::bindings;
use crate::render::glyph::GpuDraw;
use crate::render::instance::Instance;
use crate::render::path::stroke::{DashTable, GpuStrokeRecord};
use crate::render::path::{CoverageKey, GpuBound, GpuElement, GpuPath};
use crate::render::stroke_abi::{
    LIST_WORDS, RECORD_WORDS, SCAN_BLOCK, SLOT_WORDS, SLOTS, STAGE_WORDS,
};

/// The divisor taking the engine's GPU budget to the retained-but-undrawn
/// coverage share (water-rs/waterui#3081).
pub const BUDGET_SHARE: u64 = 8;
/// u32 words of one `GpuPath` record on the device.
const PATH_WORDS: usize = 48;
/// u32 words of `Params`.
const PARAMS_WORDS: usize = 28;
/// `Params` blocks the buffer holds, one per 256 bytes (the dynamic offset
/// stride): the radix passes (at most eight), then the two scan operations.
const PARAMS_BLOCKS: u64 = 10;
/// The first of the two scan operations' `Params` blocks.
const SCAN_PARAMS_BLOCK: u32 = 8;
/// Bytes between `Params` blocks.
const PARAMS_STRIDE: u32 = 256;
/// u32 words of indirect arguments per GPU draw: `draw_indirect` (4) +
/// `draw_indexed_indirect` (5) + `draw_indirect` (4) = 13.
pub const ARGS_WORDS: u32 = 13;
/// u32 words of dispatch-domain arguments in `args`.
const DISPATCH_ARGS_WORDS: u64 = 15;
/// Byte offset of the dash fragment dispatch in `args`.
const ARGS_DASH: u64 = 48;
/// u32 words of per-record scratch the dash passes keep (`dash_words`).
const DASH_RECORD_WORDS: u32 = 8;
/// u32 words of one block of the dash scans (`scan_words`).
const SCAN_BLOCK_WORDS: u32 = 4;
/// One stroke record is `RECORD_WORDS` words, the layout the kernels read.
const _: () = assert!(std::mem::size_of::<GpuStrokeRecord>() == RECORD_WORDS as usize * 4);
/// A scan block is 64 lanes of four records, the shape `stroke.wgsl` scans.
const _: () = assert!(SCAN_BLOCK == 64 * 4);
/// Byte offset of a draw's opaque-phase `draw_indirect` record.
pub const ARGS_OPAQUE: u32 = 0;
/// Byte offset of its partial-phase `draw_indexed_indirect` record.
pub const ARGS_PARTIAL: u32 = 16;
/// Byte offset of its normal-phase `draw_indirect` record.
pub const ARGS_NORMAL: u32 = 36;
/// The kernel workgroup size `coverage.wgsl` declares.
const WORKGROUP: u32 = 64;
const CLASSIFY_WIDTH: u32 = super::path::GPU_CLASSIFY_WIDTH;
/// Decision R2's hard per-batch cap on the coverage buffers' total
/// size. A path whose own bound passes it is routed to the CPU sweep
/// ([`CoverageLimit::admits`]); a frame whose paths together pass it is
/// refused at [`GpuCoverage::draw`], never retried.
const GPU_COV_MAX_BYTES: u64 = 256 << 20;
/// The bytes a frame binds beside its paths' rows: the `Params` blocks,
/// the dispatch arguments and the work header.
const FRAME_FIXED_BYTES: u64 = 18 * 1024 + 128 + 48;

/// u32 words of one geometry record in the arena: the device bounds
/// vec4 plus `(texel offset, interior)`; an all-ones offset denotes a span.
const GEOMETRY_WORDS: u32 = 6;
/// u32 words of one block's header: the emitted instance count and pad.
const BLOCK_HEADER_WORDS: u32 = 4;

/// One GPU draw of the frame: the coverage entry it replays and the
/// draw's placement shift. Block addresses resolve at `write_draw` and
/// prologue time, after any compaction has settled (water-rs/waterui#3081).
#[derive(Clone, Copy)]
struct Draw {
    /// The entry's retention key.
    key: CoverageKey,
    /// The integer translation the anchor's `shape.half` carries into
    /// the vertex-stage join: the draw's own placement offset for an
    /// unclipped entry, zero for a clipped one.
    shift: [i32; 2],
}

/// A retained coverage entry: one arena block per placement key.
struct Entry {
    /// First word of the block — the count word's address.
    base: u32,
    /// First texel word of the block, arena-absolute.
    texel_base: u32,
    /// Whole block words — the accounting unit.
    words: u32,
    /// Geometry slots the block reserves.
    geometry_slots: u32,
    /// This commit's `paths` record index while the entry awaits
    /// generation — the compaction's record fix-up needs it.
    record: u32,
    /// The generating frame's submit landed. Entries that never made
    /// it into a submission die at the next `begin_frame`, so no frame
    /// can replay a block whose kernels never ran.
    generated: bool,
    /// At least one ordinary draw needs this coverage beyond picture generation.
    retained: bool,
    /// While a compaction's copies are unrecorded, the block base in
    /// `compaction`'s source buffer where this entry's valid content
    /// lives.
    src: Option<u32>,
    /// Commit epoch of last use — the undrawn-eviction order.
    last_used: u64,
}

/// A compaction's pending copies: the buffer live content sits in and
/// each generated entry's `(src, dst, len)` byte range into the fresh
/// arena (water-rs/waterui#3081 §3.3).
struct Compaction {
    source: wgpu::Buffer,
    copies: Vec<(u64, u64, u64)>,
}

/// The retained coverage arena: one storage buffer whose per-entry
/// blocks carry a count header, the emitted geometry records and the
/// packed R8 texels. Blocks are appended through a bump frontier;
/// holes from evicted or rolled-back entries are reclaimed by
/// compaction into a fresh buffer under the 2× rule (§3.3).
struct CoverageArena {
    buffer: Option<wgpu::Buffer>,
    /// Words `buffer` holds.
    words: u32,
    /// Bump frontier, always 8-word aligned.
    tail: u32,
    /// Layout changed; allocate its final capacity at `ensure`.
    pending: bool,
    /// Bumped on every buffer replacement; `bind0` is stale whenever it
    /// differs from the generation `bind0` was built against.
    generation: u64,
}

impl CoverageArena {
    const fn new() -> Self {
        Self {
            buffer: None,
            words: 0,
            tail: 0,
            pending: false,
            generation: 0,
        }
    }

    /// The arena's buffer, once the first frame with GPU draws sized it.
    const fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// Words the frame's allocations ask for — the buffer requirement.
    const fn required_words(&self) -> u32 {
        self.tail
    }

    fn trim(&mut self) {
        self.buffer = None;
        self.words = 0;
        self.tail = 0;
        self.pending = false;
        self.generation += 1;
    }
}

/// Pipelines for the sparse boundary coverage stages.
struct Pipes {
    prepare_dispatch: wgpu::ComputePipeline,
    prepare_cells: wgpu::ComputePipeline,
    radix_count: wgpu::ComputePipeline,
    radix_scan_blocks: wgpu::ComputePipeline,
    radix_prefix: wgpu::ComputePipeline,
    radix_scatter: wgpu::ComputePipeline,
    flatten: wgpu::ComputePipeline,
    bin: wgpu::ComputePipeline,
    fragment_bins: wgpu::ComputePipeline,
    link_groups: wgpu::ComputePipeline,
    prefix_tiles: wgpu::ComputePipeline,
    prefix: wgpu::ComputePipeline,
    integrate_edges: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    classify: wgpu::ComputePipeline,
    fill_cells: wgpu::ComputePipeline,
    emit_counts: wgpu::ComputePipeline,
    strokes: Option<StrokePipes>,
    dashes: Option<DashPipes>,
}

struct StrokePipes {
    stroke_vertices: wgpu::ComputePipeline,
    stroke_emit: wgpu::ComputePipeline,
    bin_strokes: wgpu::ComputePipeline,
}

struct DashPipes {
    dash_length: wgpu::ComputePipeline,
    dash_scan_reduce: wgpu::ComputePipeline,
    dash_scan_blocks: wgpu::ComputePipeline,
    dash_scan_apply: wgpu::ComputePipeline,
    dash_fragments: wgpu::ComputePipeline,
    dash_args: wgpu::ComputePipeline,
    dash_emit: wgpu::ComputePipeline,
    bin_dashes: wgpu::ComputePipeline,
}

/// The frame's coverage scratch buffers, sized exactly by the R2 bound.
struct Buffers {
    params: wgpu::Buffer,
    elements: wgpu::Buffer,
    paths: wgpu::Buffer,
    segments: wgpu::Buffer,
    coverage: wgpu::Buffer,
    /// The stroke records, then the dash tables.
    strokes: wgpu::Buffer,
    args: wgpu::Buffer,
    work: wgpu::Buffer,
}

/// The retained coverage arena and one commit's dispatch context.
/// Entries keyed by `path::placement` survive across frames — a draw
/// that finds its key replays the block with a per-draw shift, a miss
/// runs the kernels for it alone (water-rs/waterui#3081).
pub struct GpuCoverage {
    /// `GpuPath` record words for the frame's misses, in apply order.
    paths: Vec<[u32; PATH_WORDS]>,
    batch: Batch,
    batches: Vec<Batch>,
    uploads: Vec<wgpu::Buffer>,
    /// Retained coverage entries by placement key (water-rs/waterui#3081).
    entries: FxHashMap<CoverageKey, Entry>,
    /// Commit ordinal — `Entry::last_used`'s clock.
    epoch: u64,
    /// Coverage generations this commit — `stats.paths_rasterized`.
    generated: u32,
    /// Block words summed over live entries — the compaction numerator.
    live_words: u64,
    /// Bytes held by entries no draw touched this commit.
    undrawn_bytes: u64,
    /// The undrawn byte budget — the engine's GPU-budget share.
    budget: u64,
    /// The device's binding limit.
    limit: CoverageLimit,
    /// An unrecorded compaction's copy plan. Held until a submit lands
    /// it — a failed or retried commit re-records it rather than
    /// pointing entries at content that never moved.
    compaction: Option<Compaction>,
    /// Compute input elements, all paths concatenated.
    elements: Vec<GpuElement>,
    element_sources: Vec<Arc<[GpuElement]>>,
    /// Compute stroker input: the stroke records of all paths concatenated,
    /// undashed paths first (dashed records form one contiguous range).
    strokes: Vec<GpuStrokeRecord>,
    /// Each path's stroke records, in apply order (empty for a fill).
    stroke_sources: Vec<Arc<[GpuStrokeRecord]>>,
    /// Each path's dash table, in apply order.
    dash_sources: Vec<Option<Arc<DashTable>>>,
    /// The dash tables' words behind the stroke records.
    dashes: Vec<u32>,
    /// `(path, strip)` items uploaded after atomic counters and row windings.
    work: Vec<[u32; 2]>,
    /// One GPU draw per applied `GpuPath`, in apply order.
    draws: Vec<Draw>,
    /// CPU words of the frame's `draws` buffer: `ARGS_WORDS` per draw,
    /// the three count words supplied by the prologue's copies.
    draw_args: Vec<u32>,
    /// The per-draw indirect records: `INDIRECT | COPY_DST`.
    draws_buffer: Option<wgpu::Buffer>,
    /// The coverage arena: geometry records and texels, one block per entry.
    arena: CoverageArena,
    /// The prologue went into some surface's encoder already.
    encoded: bool,
    // Device objects, created on the first dispatch that needs them.
    layout: Option<wgpu::BindGroupLayout>,
    args_layout: Option<wgpu::BindGroupLayout>,
    pipes: Option<Pipes>,
    buffers: Option<Buffers>,
    /// Last generation working set, reserved before optional picture admission.
    generation_bytes: u64,
}

/// A contiguous range of misses sharing one temporary buffer allocation.
#[derive(Clone, Copy, Default)]
struct Batch {
    start: usize,
    end: usize,
    element_total: u32,
    stroke_total: u32,
    dash_total: u32,
    stage_total: u32,
    list_total: u32,
    dash_table_words: u32,
    seg_total: u32,
    bin_total: u32,
    totals: FrameTotals,
    row_total: u32,
    bound: BufferBounds,
}

/// The R2 bound of a batch's draws, split by the buffer each term sizes.
/// `record_prologue` and the output checks compare each bound buffer, not
/// the sum, with the device binding limit; the sum is decision R2's cap.
#[derive(Clone, Copy, Default)]
struct BufferBounds {
    elements: u64,
    paths: u64,
    segments: u64,
    /// The coverage scratch: bins, fragments, sort, groups, tokens, tiles.
    scratch: u64,
    /// The work-item buffer: row windings and strip/tile items.
    work: u64,
    /// The coverage arena blocks the frame allocates.
    arena: u64,
    /// The stroke records and dash cumulative lengths.
    strokes: u64,
    /// Records outside any binding-limited buffer.
    unbound: u64,
}

impl BufferBounds {
    const fn add(&mut self, other: &Self) {
        self.elements += other.elements;
        self.paths += other.paths;
        self.segments += other.segments;
        self.scratch += other.scratch;
        self.work += other.work;
        self.arena += other.arena;
        self.strokes += other.strokes;
        self.unbound += other.unbound;
    }

    const fn total(&self) -> u64 {
        self.elements
            + self.paths
            + self.segments
            + self.scratch
            + self.work
            + self.arena
            + self.strokes
            + self.unbound
    }

    fn largest(&self) -> u64 {
        [
            self.elements,
            self.paths,
            self.segments,
            self.scratch,
            self.work,
            self.arena,
            self.strokes,
        ]
        .into_iter()
        .max()
        .expect("seven buffers")
    }
}

impl GpuCoverage {
    pub fn new(device: &wgpu::Device, budget: u64) -> Self {
        Self::with_limits(&device.limits(), budget)
    }

    fn with_limits(limits: &wgpu::Limits, budget: u64) -> Self {
        Self {
            paths: Vec::new(),
            batch: Batch::default(),
            batches: Vec::new(),
            uploads: Vec::new(),
            entries: FxHashMap::default(),
            epoch: 0,
            generated: 0,
            live_words: 0,
            undrawn_bytes: 0,
            budget,
            limit: CoverageLimit::new(limits),
            compaction: None,
            elements: Vec::new(),
            element_sources: Vec::new(),
            strokes: Vec::new(),
            stroke_sources: Vec::new(),
            dash_sources: Vec::new(),
            dashes: Vec::new(),
            work: Vec::new(),
            draws: Vec::new(),
            draw_args: Vec::new(),
            draws_buffer: None,
            arena: CoverageArena::new(),
            encoded: false,
            layout: None,
            args_layout: None,
            pipes: None,
            buffers: None,
            generation_bytes: 0,
        }
    }

    /// A frame's apply starts over: the pending records of a planned
    /// frame are committed from scratch (`pending.clear()` precedes
    /// the lowers that produce them, `mod.rs` commit flow). Entries the
    /// previous attempt allocated but never submitted die with it — a
    /// failed or retried commit can never leave a block whose kernels
    /// did not run for a later frame to replay (water-rs/waterui#3081).
    /// The holes they leave are the compaction rule's input.
    pub fn begin_frame(&mut self) {
        self.epoch += 1;
        self.paths.clear();
        self.batch = Batch::default();
        self.batches.clear();
        self.uploads.clear();
        self.elements.clear();
        self.element_sources.clear();
        self.strokes.clear();
        self.stroke_sources.clear();
        self.dash_sources.clear();
        self.dashes.clear();
        self.work.clear();
        self.draws.clear();
        self.draw_args.clear();
        self.generated = 0;
        self.entries.retain(|_, e| e.generated);
        self.live_words = self.entries.values().map(|e| u64::from(e.words)).sum();
        // Every surviving entry predates the bumped epoch: all undrawn.
        self.undrawn_bytes = self.live_words * 4;
        self.encoded = false;
    }

    /// Whether the frame carries GPU draws — the prologue and the
    /// arena sizing gate on it.
    pub const fn has_draws(&self) -> bool {
        !self.draws.is_empty()
    }

    /// Whether the frame's draws still need the coverage prologue
    /// recorded on the first encoded surface.
    pub const fn needs_prologue(&self) -> bool {
        self.has_draws() && !self.encoded
    }

    /// GPU buffer bytes resident — the `Engine::memory()` contribution
    /// of scope item 8. While a compaction is pending its source arena
    /// is still held next to the new one until the submit lands.
    pub fn bytes(&self) -> u64 {
        self.uploads.iter().map(wgpu::Buffer::size).sum::<u64>()
            + self.generation_resident_bytes()
            + self.arena.buffer.as_ref().map_or(0, wgpu::Buffer::size)
            + self
                .compaction
                .as_ref()
                .filter(|compaction| self.arena.buffer.as_ref() != Some(&compaction.source))
                .map_or(0, |compaction| compaction.source.size())
            + self.draws_buffer.as_ref().map_or(0, wgpu::Buffer::size)
    }

    /// Resident storage plus the allocations the accepted frame will need.
    /// Admission uses the same layout as ensure, before creating buffers.
    pub fn planned_bytes(&self) -> u64 {
        let mut bytes = self.bytes();
        if !self.has_draws() {
            return bytes;
        }
        if self.arena.pending {
            bytes += u64::from(self.arena.words) * 4;
        }
        let draws = (self.draws.len() as u64).max(1) * u64::from(ARGS_WORDS) * 4;
        bytes += draws.saturating_sub(self.draws_buffer.as_ref().map_or(0, wgpu::Buffer::size));
        if self.has_misses() {
            let sizes = allocation_sizes(self.generation_sizes());
            let resident = self.buffers.as_ref().map_or([0; 8], |b| {
                [
                    b.coverage.size(),
                    b.elements.size(),
                    b.strokes.size(),
                    b.paths.size(),
                    b.segments.size(),
                    b.args.size(),
                    b.work.size(),
                    b.params.size(),
                ]
            });
            let required = sizes
                .into_iter()
                .chain([PARAMS_BLOCKS * u64::from(PARAMS_STRIDE)]);
            bytes += required
                .zip(resident)
                .map(|(need, have)| need.saturating_sub(have))
                .sum::<u64>();
        }
        bytes
    }

    fn generation_sizes(&self) -> [u64; 7] {
        let mut sizes = [0; 7];
        for &batch in self.batches.iter().chain(std::iter::once(&self.batch)) {
            for (peak, size) in sizes.iter_mut().zip(BatchPlan::new(batch).sizes) {
                *peak = (*peak).max(size);
            }
        }
        sizes
    }

    fn generation_resident_bytes(&self) -> u64 {
        self.buffers.as_ref().map_or(0, |b| {
            b.params.size()
                + b.elements.size()
                + b.paths.size()
                + b.segments.size()
                + b.coverage.size()
                + b.strokes.size()
                + b.args.size()
                + b.work.size()
        })
    }

    /// Released generation buffers will be needed again when picture misses
    /// rasterize. Do not offer their working set to optional image retention.
    pub fn generation_reservation(&self) -> u64 {
        self.generation_bytes
            .saturating_sub(self.generation_resident_bytes())
    }

    /// Retained host input/draw-list allocation capacities.
    pub fn cpu_bytes(&self) -> u64 {
        (self.entries.capacity() * std::mem::size_of::<(CoverageKey, Entry)>()
            + self.uploads.capacity() * std::mem::size_of::<wgpu::Buffer>()
            + self.batches.capacity() * std::mem::size_of::<Batch>()
            + self.paths.capacity() * std::mem::size_of::<[u32; PATH_WORDS]>()
            + self.elements.capacity() * std::mem::size_of::<GpuElement>()
            + self.element_sources.capacity() * std::mem::size_of::<Arc<[GpuElement]>>()
            + self.strokes.capacity() * std::mem::size_of::<GpuStrokeRecord>()
            + self.stroke_sources.capacity() * std::mem::size_of::<Arc<[GpuStrokeRecord]>>()
            + self.dash_sources.capacity() * std::mem::size_of::<Option<Arc<DashTable>>>()
            + self.dashes.capacity() * std::mem::size_of::<u32>()
            + self.work.capacity() * std::mem::size_of::<[u32; 2]>()
            + self.draws.capacity() * std::mem::size_of::<Draw>()
            + self.draw_args.capacity() * std::mem::size_of::<u32>()) as u64
    }

    /// The coverage arena's buffer, once `ensure` has sized it —
    /// `ENGINE_GROUP0` binding 4.
    pub const fn arena(&self) -> Option<&wgpu::Buffer> {
        self.arena.buffer()
    }

    /// Bumped whenever the arena's buffer is replaced or dropped.
    pub const fn arena_generation(&self) -> u64 {
        self.arena.generation
    }

    /// The per-draw indirect-argument buffer — valid after `ensure`.
    pub const fn draws_buffer(&self) -> &wgpu::Buffer {
        self.draws_buffer.as_ref().expect("draws buffer")
    }

    /// Byte offset of `draw`'s three indirect argument records —
    /// `ARGS_OPAQUE`/`ARGS_PARTIAL`/`ARGS_NORMAL` further in.
    pub fn draw_base(draw: u32) -> u64 {
        u64::from(draw) * u64::from(ARGS_WORDS) * 4
    }

    /// Coverage generations this commit — the frame's
    /// `stats.paths_rasterized` contribution (water-rs/waterui#3081).
    pub const fn generated(&self) -> u32 {
        self.generated
    }

    /// Whether the frame's draws include misses — a compute pass and
    /// its timestamp pair exist only then.
    pub const fn has_misses(&self) -> bool {
        !self.paths.is_empty()
    }

    /// Mark `key`'s entry live for this commit before any miss may
    /// evict it: the frame's drawn entries are never evicted (§3.3).
    pub fn touch(&mut self, key: CoverageKey) {
        if let Some(entry) = self.entries.get_mut(&key)
            && entry.last_used != self.epoch
        {
            self.undrawn_bytes -= u64::from(entry.words) * 4;
            entry.last_used = self.epoch;
        }
    }

    /// The frame's submit landed: entries it generated are replayable
    /// and a compaction's copies have executed (water-rs/waterui#3081).
    pub fn submitted(&mut self) {
        // Submitted commands retain these buffers until the GPU finishes.
        // Do not keep generation scratch waiting for another frame: a static
        // application may never submit a retained-only frame to release it.
        self.buffers = None;
        if !self.encoded {
            return;
        }
        for entry in self.entries.values_mut() {
            entry.generated = true;
            entry.src = None;
        }
        self.compaction = None;
        self.uploads.clear();
    }

    /// Join one applied `GpuPath` draw into the frame, as the path's
    /// draw index. A resident entry replays its block untouched; a miss
    /// allocates a block, evicts undrawn entries LRU-first over the
    /// budget, and pushes the generation record the kernels run on.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "device coordinates are f32"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "hit arm plus the miss's bound and record"
    )]
    pub fn draw(&mut self, draw: &GpuDraw, retain: bool) -> Result<u32, RenderError> {
        let &GpuDraw {
            key,
            shift,
            rule,
            raster,
            clip,
            path: ref prepared,
        } = draw;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.retained |= retain;
            if entry.last_used != self.epoch {
                self.undrawn_bytes -= u64::from(entry.words) * 4;
                entry.last_used = self.epoch;
            }
            self.draws.push(Draw { key, shift });
            return Ok(u32::try_from(self.draws.len() - 1).expect("draw count fits u32"));
        }
        let GpuPath {
            elements,
            strokes,
            stroke,
            dash,
            bound,
            ..
        } = prepared;
        let append = |batch: &Batch| {
            path_rows(prepared, bound, batch.totals).and_then(|(rows, totals)| {
                let mut next = batch.bound;
                next.add(&rows);
                if next.total() + FRAME_FIXED_BYTES > GPU_COV_MAX_BYTES {
                    return Err(RenderError::Unsupported(
                        "GPU coverage bound exceeds the per-batch cap",
                    ));
                }
                if next.largest() > self.limit.bytes {
                    return Err(RenderError::Unsupported(
                        "GPU coverage bound exceeds the device buffer limit",
                    ));
                }
                Ok((next, totals))
            })
        };
        let (next, totals) = match append(&self.batch) {
            Ok(fits) => fits,
            Err(_) if self.batch.start != self.batch.end => {
                let empty = Batch {
                    start: self.paths.len(),
                    end: self.paths.len(),
                    ..Batch::default()
                };
                let fits = append(&empty)?;
                self.batches.push(self.batch);
                self.batch = empty;
                fits
            }
            Err(error) => return Err(error),
        };
        self.batch.bound = next;
        let record = u32::try_from(self.paths.len()).expect("path count fits u32");
        let elem_base = self.batch.element_total;
        let geometry_slots = u32::try_from(bound.instance_cap).expect("instance cap fits u32");
        let texels =
            u32::try_from(bound.texels.div_ceil(4) + bound.cell_cap).expect("texel words fit u32");
        let (base, texel_base, block_words) = self.alloc_block(geometry_slots, texels)?;
        let grid_base = self.batch.totals.columns;
        let [xx, yx, xy, yy, tx, ty] = raster.as_coeffs();
        let mut words = [0u32; PATH_WORDS];
        // atom[0..3]: bbox ordered-bits +inf/-inf seeds (the kernel's
        // atomicMin/Max operates on the monotone float->ordered map);
        // atom[4..7]: counters at 0.
        words[0] = 0xff80_0000;
        words[1] = 0xff80_0000;
        words[2] = 0x007f_ffff;
        words[3] = 0x007f_ffff;
        words[8] = elem_base;
        words[9] = u32::try_from(elements.len()).expect("elements fit u32");
        words[10] = self.batch.seg_total;
        words[11] = u32::try_from(bound.segments).expect("segments fit u32");
        words[12] = grid_base;
        words[13] = texel_base;
        words[14] = base + BLOCK_HEADER_WORDS;
        words[15] = rule;
        words[16] = u32::try_from(bound.grid_w).expect("grid fits u32");
        words[17] = u32::try_from(bound.grid_h).expect("grid fits u32");
        words[18] = self.batch.row_total;
        // affine = (a, c, b, d); translate = (e, f)
        words[20] = (xx as f32).to_bits();
        words[21] = (xy as f32).to_bits();
        words[22] = (yx as f32).to_bits();
        words[23] = (yy as f32).to_bits();
        words[24] = (tx as f32).to_bits();
        words[25] = (ty as f32).to_bits();
        words[26] = clip[2].to_bits();
        words[27] = clip[3].to_bits();
        words[34] = clip[0].to_bits();
        words[35] = clip[1].to_bits();
        // The compute stroker's path parameters. `stroke_base` is written at
        // dispatch, where the record order is known; a fill leaves all zero.
        if !strokes.is_empty() {
            words[37] = stroke.half_width.to_bits();
            words[38] = stroke.miter_limit.to_bits();
            words[39] = stroke.start_cap
                | stroke.end_cap << 2
                | u32::from(dash.as_deref().is_some_and(|dash| dash.merge_start)) << 4
                | stroke.k_cap << 8;
            words[40] = u32::MAX;
            words[44] = self.batch.stage_total;
        }
        if let Some(dash) = dash.as_deref() {
            words[40] = self.batch.dash_table_words;
            words[41] = u32::try_from(dash.cum.len() - 1).expect("dash entries fit u32");
            [words[42], words[43]] = dash.phase;
        }

        self.paths.push(words);
        self.batch.end = self.paths.len();
        self.batch.element_total += u32::try_from(elements.len()).expect("elements fit u32");
        self.element_sources.push(Arc::clone(elements));
        let records = u32::try_from(strokes.len()).expect("stroke records fit u32");
        // The R2 cap above bounds the records far below `u32::MAX`.
        self.batch.stroke_total += records;
        self.stroke_sources.push(Arc::clone(strokes));
        if let Some(dash) = dash {
            self.batch.dash_total += records;
            self.batch.stage_total += u32::try_from(dash.stage_total).expect("staging fits u32");
            self.batch.list_total += u32::try_from(dash.fragments).expect("fragments fit u32");
            self.batch.dash_table_words +=
                u32::try_from(dash.cum.len() * 2).expect("dash table fits u32");
        }
        self.dash_sources.push(dash.clone());
        self.entries.insert(
            key,
            Entry {
                base,
                texel_base,
                words: block_words,
                geometry_slots,
                record,
                generated: false,
                retained: retain,
                src: None,
                last_used: self.epoch,
            },
        );
        self.generated += 1;
        self.draws.push(Draw { key, shift });
        self.batch.seg_total += u32::try_from(bound.segments).expect("segments");
        self.batch.bin_total += u32::try_from(bound.bins).expect("bins");
        self.batch.totals = totals;
        self.batch.row_total += u32::try_from(bound.grid_h).expect("rows");

        Ok(u32::try_from(self.draws.len() - 1).expect("draw count fits u32"))
    }

    /// Settle the retained set once per commit, after the frame's
    /// entries were touched and before any miss allocates (§3.3).
    /// Retained-but-undrawn entries pay for themselves: they are evicted
    /// LRU-first until they fit the budget again. Entries this frame
    /// drew — `last_used` at the current epoch — are never touched. When
    /// the frame draws coverage and the arena's frontier then exceeds
    /// twice the live words, the live blocks are rebuilt into a fresh
    /// buffer, whether or not any draw misses. A frame without GPU
    /// draws records no encoder work to carry the copies, so its
    /// compaction waits for the next frame that has some.
    pub fn settle(&mut self, draws_coverage: bool) {
        self.evict_undrawn();
        if draws_coverage && u64::from(self.arena.tail) > 2 * self.live_words {
            self.compact(0)
                .expect("the live blocks fit the arena they came from");
        }
    }

    fn evict_undrawn(&mut self) {
        if self.undrawn_bytes <= self.budget {
            return;
        }
        let mut undrawn: Vec<CoverageKey> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_used < self.epoch)
            .map(|(&key, _)| key)
            .collect();
        undrawn.sort_by_key(|&key| self.entries[&key].last_used);
        for key in undrawn {
            if self.undrawn_bytes <= self.budget {
                break;
            }
            let entry = self.entries.remove(&key).expect("undrawn entry");
            self.undrawn_bytes -= u64::from(entry.words) * 4;
            self.live_words -= u64::from(entry.words);
        }
    }

    /// Allocate a block of `geometry_slots` geometry records and
    /// `texels` texel words, compacting or growing the arena when the
    /// bump frontier cannot serve it.
    fn alloc_block(
        &mut self,
        geometry_slots: u32,
        texels: u32,
    ) -> Result<(u32, u32, u32), RenderError> {
        let block_words = (BLOCK_HEADER_WORDS + geometry_slots * GEOMETRY_WORDS + texels)
            .next_multiple_of(GEOMETRY_WORDS);
        // A block that does not fit at the tail rebuilds the live blocks
        // into a fresh buffer with room for it (§3.3).
        if self.arena.tail + block_words > self.arena.words {
            self.compact(block_words)?;
        }
        let base = self.arena.tail;
        self.arena.tail += block_words;
        self.live_words += u64::from(block_words);
        Ok((
            base,
            base + BLOCK_HEADER_WORDS + geometry_slots * GEOMETRY_WORDS,
            block_words,
        ))
    }

    /// Plan every live entry in a fresh arena buffer sized to the
    /// live words plus this frame's immediate need, with a quarter of
    /// that as headroom (§3.3). Generated entries
    /// schedule one copy each into the coverage prologue; ungenerated
    /// ones redirect their dispatch record to the new block.
    fn compact(&mut self, extra: u32) -> Result<(), RenderError> {
        let mut order: Vec<CoverageKey> = self.entries.keys().copied().collect();
        order.sort_by_key(|key| self.entries[key].base);
        let mut tail = 0u32;
        let mut moves = Vec::with_capacity(order.len());
        for key in order {
            let entry = &self.entries[&key];
            moves.push((key, tail));
            tail += entry.words;
        }
        if u64::from(tail) + u64::from(extra) > self.limit.words() {
            return Err(RenderError::Unsupported(
                "GPU coverage output exceeds device buffer limit",
            ));
        }
        // The live set is bounded by the drawn set plus the undrawn
        // budget, so in steady state it churns rather than grows: the
        // headroom only absorbs the holes misses leave between repacks.
        // A quarter of the need amortizes each repack over `live / 4`
        // newly generated words, where doubling would hold the live set's
        // size again in empty capacity. Clamped to what one binding can
        // address; the live set plus the immediate need already fit.
        let need = u64::from(tail) + u64::from(extra);
        let size = (need + u64::from(extra).max(need / 4))
            .min(self.limit.words())
            .max(16)
            * 4;
        // Generated content's source: an unrecorded compaction's buffer
        // still holds it, otherwise the live arena does.
        let source = self
            .compaction
            .as_ref()
            .map(|compaction| compaction.source.clone())
            .or_else(|| self.arena.buffer.clone());
        let mut copies = Vec::new();
        for (key, new_base) in moves {
            let entry = self.entries.get_mut(&key).expect("compaction entry");
            if entry.generated {
                let src = entry.src.unwrap_or(entry.base);
                copies.push((
                    u64::from(src) * 4,
                    u64::from(new_base) * 4,
                    u64::from(entry.words) * 4,
                ));
                entry.src = Some(src);
            } else {
                let record = &mut self.paths[entry.record as usize];
                record[14] = new_base + BLOCK_HEADER_WORDS;
                record[13] = new_base + BLOCK_HEADER_WORDS + entry.geometry_slots * GEOMETRY_WORDS;
            }
            entry.texel_base =
                new_base + BLOCK_HEADER_WORDS + entry.geometry_slots * GEOMETRY_WORDS;
            entry.base = new_base;
        }
        self.compaction = source
            .filter(|_| !copies.is_empty())
            .map(|source| Compaction { source, copies });
        self.arena.words = u32::try_from(size / 4).expect("arena words fit u32");
        self.arena.tail = tail;
        self.arena.pending = true;
        Ok(())
    }

    /// Write `index`'s indirect records and stamp the anchor's per-draw
    /// slots once its global instance index is known: the draw's
    /// placement shift goes in `shape.half` and the block's texel base
    /// word in `uv.x`, which the cell-buffer vertex stage joins into
    /// every emitted instance. The count words arrive through the
    /// prologue's copies.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the shift is a pixel-exact integer translation"
    )]
    pub fn write_draw(&mut self, index: u32, anchor_global: u32, anchor: &mut Instance) {
        let draw = self.draws[index as usize];
        let entry = &self.entries[&draw.key];
        anchor.shape.half = [draw.shift[0] as f32, draw.shift[1] as f32];
        anchor.uv[0] = f32::from_bits(entry.texel_base);
        anchor.uv[1] = 0.0;
        let first_vertex = (anchor_global + 1) * 8;
        let first_instance = entry.base / GEOMETRY_WORDS;
        let base = usize::try_from(index).expect("draw index") * ARGS_WORDS as usize;
        if self.draw_args.len() < base + ARGS_WORDS as usize {
            self.draw_args.resize(base + ARGS_WORDS as usize, 0);
        }
        self.draw_args[base..base + ARGS_WORDS as usize].copy_from_slice(&[
            // opaque: draw_indirect(8, n, first_vertex, first_instance)
            8,
            0,
            first_vertex,
            first_instance,
            // partial: draw_indexed_indirect(6, n, 0, base_vertex, first_instance)
            6,
            0,
            0,
            first_vertex,
            first_instance,
            // normal: draw_indirect(6, n, first_vertex, first_instance)
            6,
            0,
            first_vertex,
            first_instance,
        ]);
    }

    /// Reject device limits before allocating the generated coverage.
    pub fn validate_output_sizes(&self, limits: &wgpu::Limits) -> Result<(), RenderError> {
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        if u64::from(self.arena.required_words()) * 4 > limit {
            return Err(RenderError::Unsupported(
                "GPU coverage output exceeds device buffer limit",
            ));
        }
        Ok(())
    }

    /// Allocate the planned arena and size draw/compute buffers before optional
    /// images are admitted. Multiple commit-time repacks allocate only once.
    /// Its generation tells the renderer whether `bind0` is stale.
    pub fn ensure(&mut self, device: &wgpu::Device) -> Result<(), RenderError> {
        self.materialize_arena(device);
        self.draws_buffer = Some(sized_buffer(
            device,
            self.draws_buffer.as_ref(),
            "gpu draws",
            (self.draws.len() as u64).max(1) * u64::from(ARGS_WORDS) * 4,
            wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | if cfg!(test) {
                    wgpu::BufferUsages::COPY_SRC
                } else {
                    wgpu::BufferUsages::empty()
                },
        ));
        if self.has_misses() {
            self.size_buffers(device, self.generation_sizes())?;
        }
        Ok(())
    }

    fn materialize_arena(&mut self, device: &wgpu::Device) {
        if self.arena.pending {
            self.arena.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("coverage arena"),
                size: u64::from(self.arena.words) * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.arena.pending = false;
            self.arena.generation += 1;
        }
    }

    /// All surfaces have encoded their draws. Picture-only coverage can leave
    /// now: submitted commands own the old arena until its readers complete.
    pub fn finish_pictures(&mut self) {
        let before = self.entries.len();
        self.entries.retain(|_, entry| entry.retained);
        if self.entries.len() == before {
            return;
        }
        self.live_words = self
            .entries
            .values()
            .map(|entry| u64::from(entry.words))
            .sum();
        self.undrawn_bytes = self
            .entries
            .values()
            .filter(|entry| entry.last_used < self.epoch)
            .map(|entry| u64::from(entry.words) * 4)
            .sum();
        if self.entries.is_empty() {
            self.compaction = None;
            self.arena.trim();
        }
        // Mixed arenas reuse their existing storage. The next commit's
        // ordinary 2x compaction rule reclaims these holes without allocating
        // and copying a fresh arena after every picture generation frame.
    }

    pub fn trim(&mut self) {
        self.generation_bytes = 0;
        self.entries.clear();
        self.compaction = None;
        self.live_words = 0;
        self.undrawn_bytes = 0;
        self.buffers = None;
        self.uploads.clear();
        self.draws_buffer = None;
        self.arena.trim();
    }

    /// Record the frame's coverage prologue on the encoder — a pending
    /// compaction's copies, the miss dispatch that fills each new
    /// block, then one 4-byte copy per phase record feeding the count
    /// words `emit_counts` wrote. A frame whose draws are all hits
    /// records no compute at all. Idempotent per frame.
    pub fn record_prologue(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        delivery: crate::render::shaders::ShaderDelivery,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        if self.encoded || !self.has_draws() {
            return;
        }
        self.encoded = true;
        let arena = self.arena.buffer().expect("coverage arena sized").clone();
        if let Some(compaction) = &self.compaction {
            for &(src, dst, len) in &compaction.copies {
                encoder.copy_buffer_to_buffer(&compaction.source, src, &arena, dst, len);
            }
        }
        if self.has_misses() {
            let timestamp_writes = timestamp_writes.map(|writes| {
                (
                    writes.query_set,
                    writes.beginning_of_pass_write_index,
                    writes.end_of_pass_write_index,
                )
            });
            for index in 0..=self.batches.len() {
                let batch = self.batches.get(index).copied().unwrap_or(self.batch);
                let timestamps = timestamp_writes.map(|(query_set, beginning, end)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: if index == 0 { beginning } else { None },
                        end_of_pass_write_index: if index == self.batches.len() {
                            end
                        } else {
                            None
                        },
                    }
                });
                self.dispatch(device, batch, queue, encoder, delivery, timestamps);
            }
        }
        let draws_buffer = self.draws_buffer.clone().expect("draws buffer");
        queue.write_buffer(&draws_buffer, 0, bytemuck::cast_slice(&self.draw_args));
        // Each phase's instance count is the block's count word,
        // finalized only once `emit_counts` ran.
        for (index, draw) in self.draws.iter().enumerate() {
            let count_word = u64::from(self.entries[&draw.key].base) * 4;
            let record = index as u64 * u64::from(ARGS_WORDS) * 4;
            for offset in [4, u64::from(ARGS_PARTIAL) + 4, u64::from(ARGS_NORMAL) + 4] {
                encoder.copy_buffer_to_buffer(
                    &arena,
                    count_word,
                    &draws_buffer,
                    record + offset,
                    4,
                );
            }
        }
    }

    fn size_buffers(&mut self, device: &wgpu::Device, sizes: [u64; 7]) -> Result<(), RenderError> {
        if sizes.into_iter().any(|size| size > self.limit.bytes) {
            return Err(RenderError::Unsupported(
                "GPU coverage input exceeds device buffer limit",
            ));
        }
        let sizes = allocation_sizes(sizes);
        let previous = self.buffers.as_ref();
        // Tests read the buffers back; the runtime never does.
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | if cfg!(test) {
                wgpu::BufferUsages::COPY_SRC
            } else {
                wgpu::BufferUsages::empty()
            };
        self.buffers = Some(Buffers {
            params: sized_buffer(
                device,
                previous.map(|b| &b.params),
                "cov params",
                PARAMS_BLOCKS * u64::from(PARAMS_STRIDE),
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            elements: sized_buffer(
                device,
                previous.map(|b| &b.elements),
                "cov elements",
                sizes[1],
                storage,
            ),
            paths: sized_buffer(
                device,
                previous.map(|b| &b.paths),
                "cov paths",
                sizes[3],
                storage,
            ),
            segments: sized_buffer(
                device,
                previous.map(|b| &b.segments),
                "cov segments",
                sizes[4],
                storage,
            ),
            coverage: sized_buffer(
                device,
                previous.map(|b| &b.coverage),
                "cov scratch",
                sizes[0],
                storage,
            ),
            strokes: sized_buffer(
                device,
                previous.map(|b| &b.strokes),
                "cov strokes",
                sizes[2],
                storage,
            ),
            args: sized_buffer(
                device,
                previous.map(|b| &b.args),
                "cov args",
                DISPATCH_ARGS_WORDS * 4,
                storage | wgpu::BufferUsages::INDIRECT,
            ),
            work: sized_buffer(
                device,
                previous.map(|b| &b.work),
                "cov work",
                sizes[6],
                storage,
            ),
        });
        self.generation_bytes = self.generation_resident_bytes();

        Ok(())
    }

    /// Record the dispatch set on the encoder — coverage clearing,
    /// then the sparse coverage kernels, batching every routed path of the frame
    /// (decision S2).
    #[expect(
        clippy::too_many_lines,
        reason = "one ordered compute dispatch sequence and its output resources"
    )]
    fn dispatch(
        &mut self,
        device: &wgpu::Device,
        batch: Batch,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        delivery: crate::render::shaders::ShaderDelivery,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        self.ensure_pipes(device, delivery, batch);
        self.work.clear();
        self.elements.clear();
        self.strokes.clear();
        self.dashes.clear();

        for (index, path) in self.paths[batch.start..batch.end].iter().enumerate() {
            for strip in 0..path[17].div_ceil(
                u32::try_from(crate::render::path::STRIP_H).expect("coverage capacity fits u32"),
            ) {
                self.work.push([
                    u32::try_from(index).expect("coverage capacity fits u32"),
                    strip,
                ]);
            }
        }
        let strip_total = u32::try_from(self.work.len()).expect("coverage capacity fits u32");
        for (index, path) in self.paths[batch.start..batch.end].iter_mut().enumerate() {
            path[28] = u32::try_from(self.work.len()).expect("work count fits u32") - strip_total;
            let tiles = path[16].div_ceil(CLASSIFY_WIDTH) * path[17].div_ceil(4);
            for tile in 0..tiles {
                self.work
                    .push([u32::try_from(index).expect("path index fits u32"), tile]);
            }
        }
        let tile_total = u32::try_from(self.work.len()).expect("work count fits u32") - strip_total;
        let BatchPlan {
            params,
            work_header_bytes,
            radix_pass_count,
            ..
        } = BatchPlan::new(batch);
        let stroke_total = params[18];
        let dash_total = params[19];
        let dash_blocks = params[21];
        let buffers = self.buffers.as_ref().expect("coverage buffers sized");

        let mut upload = BatchUpload::new(queue, !self.batches.is_empty());
        for radix_pass in 0..radix_pass_count.max(1) {
            let mut block = params;
            block[17] = radix_pass;
            upload.write_buffer(
                &buffers.params,
                u64::from(radix_pass * PARAMS_STRIDE),
                bytemuck::cast_slice(&block),
            );
        }
        // The two scan operations of the dash passes read their operation
        // from a block of their own (design section 7.4.2).
        for op in 0..2 {
            let mut block = params;
            block[22] = op;
            upload.write_buffer(
                &buffers.params,
                u64::from((SCAN_PARAMS_BLOCK + op) * PARAMS_STRIDE),
                bytemuck::cast_slice(&block),
            );
        }
        for (index, source) in self.element_sources[batch.start..batch.end]
            .iter()
            .enumerate()
        {
            let path_i = u32::try_from(index).expect("path index");
            self.elements
                .extend(source.iter().copied().map(|mut element| {
                    element.head[2] = path_i;
                    element
                }));
        }
        upload.write_buffer(&buffers.elements, 0, bytemuck::cast_slice(&self.elements));
        // Undashed paths' records first, dashed ones after (7.4); the dash
        // tables follow in apply order, where `draw` assigned their offsets.
        for dashed in [false, true] {
            for (index, source) in self.stroke_sources[batch.start..batch.end]
                .iter()
                .enumerate()
            {
                if self.dash_sources[batch.start + index].is_some() != dashed {
                    continue;
                }
                let path_i = u32::try_from(index).expect("path index");
                if !source.is_empty() {
                    self.paths[batch.start + index][36] =
                        u32::try_from(self.strokes.len()).expect("stroke records fit u32");
                }
                self.strokes
                    .extend(source.iter().copied().map(|mut record| {
                        record.head[2] = path_i;
                        record
                    }));
            }
        }
        for dash in self.dash_sources[batch.start..batch.end].iter().flatten() {
            for &[integer, fraction] in &*dash.cum {
                self.dashes.extend([integer, fraction]);
            }
        }
        upload.write_buffer(&buffers.strokes, 0, bytemuck::cast_slice(&self.strokes));
        upload.write_buffer(
            &buffers.strokes,
            self.strokes.len() as u64 * std::mem::size_of::<GpuStrokeRecord>() as u64,
            bytemuck::cast_slice(&self.dashes),
        );
        upload.write_buffer(
            &buffers.work,
            work_header_bytes,
            bytemuck::cast_slice(&self.work),
        );
        upload.write_buffer(
            &buffers.paths,
            0,
            bytemuck::cast_slice(self.paths[batch.start..batch.end].as_flattened()),
        );
        if let Some(source) = upload.finish(device, encoder) {
            self.uploads.push(source);
        }
        encoder.clear_buffer(&buffers.work, 0, Some(work_header_bytes));

        let arena = self.arena.buffer().expect("coverage arena sized");
        let layout = self.layout.as_ref().expect("coverage layout");
        let entries = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffers.params,
                    offset: 0,
                    size: wgpu::BufferSize::new(PARAMS_WORDS as u64 * 4),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.elements.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.paths.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.segments.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.coverage.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: arena.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: buffers.work.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: buffers.strokes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: buffers.args.as_entire_binding(),
            },
        ];
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu coverage"),
            layout,
            entries: &entries
                .iter()
                .filter(|entry| entry.binding != 8)
                .cloned()
                .collect::<Vec<_>>(),
        });
        let args_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu coverage argument writes"),
            layout: self.args_layout.as_ref().expect("coverage argument layout"),
            entries: &entries,
        });
        let pipes = self.pipes.as_ref().expect("coverage pipelines");
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpu coverage"),
                timestamp_writes,
            });
            let dispatch = |pass: &mut wgpu::ComputePass<'_>, n: u32| {
                let groups = n.div_ceil(WORKGROUP);
                pass.dispatch_workgroups(groups.min(256), groups.div_ceil(256), 1);
            };
            pass.set_bind_group(0, &bind, &[0]);
            pass.set_pipeline(&pipes.flatten);
            dispatch(&mut pass, params[0]);
            if stroke_total > 0 {
                let pipes = pipes.strokes.as_ref().expect("stroke pipelines prepared");
                pass.set_pipeline(&pipes.stroke_vertices);
                dispatch(&mut pass, stroke_total);
                pass.set_pipeline(&pipes.stroke_emit);
                dispatch(&mut pass, stroke_total);
            }
            if dash_total > 0 {
                let pipes = pipes.dashes.as_ref().expect("dash pipelines prepared");
                // Lengths, their per-subpath prefix, the dashes of every
                // record, then the fragment and chord offsets that compact
                // them (7.4); the scans read their operation from the
                // `Params` block at the offset they are bound with.
                let scan = |pass: &mut wgpu::ComputePass<'_>, op: u32| {
                    pass.set_bind_group(0, &bind, &[(SCAN_PARAMS_BLOCK + op) * PARAMS_STRIDE]);
                    pass.set_pipeline(&pipes.dash_scan_reduce);
                    dispatch(pass, dash_blocks * WORKGROUP);
                    pass.set_pipeline(&pipes.dash_scan_blocks);
                    pass.dispatch_workgroups(1, 1, 1);
                    pass.set_pipeline(&pipes.dash_scan_apply);
                    dispatch(pass, dash_blocks * WORKGROUP);
                    pass.set_bind_group(0, &bind, &[0]);
                };
                pass.set_pipeline(&pipes.dash_length);
                dispatch(&mut pass, dash_total);
                scan(&mut pass, 0);
                pass.set_pipeline(&pipes.dash_fragments);
                dispatch(&mut pass, dash_total);
                scan(&mut pass, 1);
                pass.set_bind_group(0, &args_bind, &[0]);
                pass.set_pipeline(&pipes.dash_args);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_bind_group(0, &bind, &[0]);
                pass.set_pipeline(&pipes.dash_emit);
                pass.dispatch_workgroups_indirect(&buffers.args, ARGS_DASH);
            }
            pass.set_pipeline(&pipes.bin);
            dispatch(&mut pass, params[0]);
            if stroke_total > 0 {
                let pipes = pipes.strokes.as_ref().expect("stroke pipelines prepared");
                pass.set_pipeline(&pipes.bin_strokes);
                dispatch(&mut pass, stroke_total);
            }
            if dash_total > 0 {
                let pipes = pipes.dashes.as_ref().expect("dash pipelines prepared");
                pass.set_pipeline(&pipes.bin_dashes);
                pass.dispatch_workgroups_indirect(&buffers.args, ARGS_DASH);
            }
            pass.set_bind_group(0, &args_bind, &[0]);
            pass.set_pipeline(&pipes.prepare_dispatch);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &bind, &[0]);
            pass.set_pipeline(&pipes.fragment_bins);
            pass.dispatch_workgroups_indirect(&buffers.args, 0);
            pass.set_bind_group(0, &args_bind, &[0]);
            pass.set_pipeline(&pipes.prepare_dispatch);
            pass.dispatch_workgroups(1, 1, 1);
            for radix_pass in 0..radix_pass_count {
                pass.set_bind_group(0, &bind, &[radix_pass * PARAMS_STRIDE]);
                pass.set_pipeline(&pipes.radix_count);
                pass.dispatch_workgroups_indirect(&buffers.args, 12);
                pass.set_pipeline(&pipes.radix_scan_blocks);
                pass.dispatch_workgroups_indirect(&buffers.args, 24);
                pass.set_pipeline(&pipes.radix_prefix);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&pipes.radix_scatter);
                pass.dispatch_workgroups_indirect(&buffers.args, 12);
            }
            pass.set_bind_group(0, &bind, &[0]);
            pass.set_pipeline(&pipes.link_groups);
            pass.dispatch_workgroups_indirect(&buffers.args, 12);
            pass.set_pipeline(&pipes.prefix_tiles);
            dispatch(&mut pass, tile_total);
            pass.set_pipeline(&pipes.prefix);
            dispatch(&mut pass, strip_total);
            pass.set_pipeline(&pipes.integrate_edges);
            pass.dispatch_workgroups_indirect(&buffers.args, 12);
            pass.set_pipeline(&pipes.integrate);
            pass.dispatch_workgroups_indirect(&buffers.args, 12);
            pass.set_pipeline(&pipes.classify);
            dispatch(&mut pass, tile_total);
            pass.set_bind_group(0, &args_bind, &[0]);
            pass.set_pipeline(&pipes.prepare_cells);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_bind_group(0, &bind, &[0]);
            pass.set_pipeline(&pipes.fill_cells);
            pass.dispatch_workgroups_indirect(&buffers.args, 36);
            pass.set_pipeline(&pipes.emit_counts);
            dispatch(
                &mut pass,
                u32::try_from(batch.end - batch.start).expect("coverage capacity fits u32"),
            );
        }
    }

    /// Build only the kernel families this batch dispatches.
    fn ensure_pipes(
        &mut self,
        device: &wgpu::Device,
        delivery: crate::render::shaders::ShaderDelivery,
        batch: Batch,
    ) {
        if self.pipes.as_ref().is_some_and(|pipes| {
            (batch.stroke_total == 0 || pipes.strokes.is_some())
                && (batch.dash_total == 0 || pipes.dashes.is_some())
        }) {
            return;
        }
        let layout = self.layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gpu coverage"),
                entries: &super::layout_entries(bindings::COVERAGE_MAIN_GROUP0),
            })
        });
        let args_layout = self.args_layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gpu coverage argument writes"),
                entries: &super::layout_entries(bindings::COVERAGE_GROUP0),
            })
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu coverage"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let args_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu coverage argument writes"),
            bind_group_layouts: &[Some(args_layout)],
            immediate_size: 0,
        });
        let programs = delivery.coverage_programs(device);
        let make = |entry| {
            let program = programs
                .iter()
                .find(|program| program.entry == entry)
                .expect("coverage kernel has a shader program");
            let layout = match program.layout {
                super::programs::ComputeLayout::CoverageMain => &pipe_layout,
                super::programs::ComputeLayout::CoverageArguments => &args_pipe_layout,
            };
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module: &program.module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let pipes = self.pipes.get_or_insert_with(|| Pipes {
            prepare_dispatch: make("prepare_dispatch"),
            prepare_cells: make("prepare_cells"),
            radix_count: make("radix_count"),
            radix_scan_blocks: make("radix_scan_blocks"),
            radix_prefix: make("radix_prefix"),
            radix_scatter: make("radix_scatter"),
            flatten: make("flatten"),
            bin: make("bin"),
            fragment_bins: make("fragment_bins"),
            link_groups: make("link_groups"),
            prefix_tiles: make("prefix_tiles"),
            prefix: make("prefix"),
            integrate_edges: make("integrate_edges"),
            integrate: make("integrate"),
            classify: make("classify"),
            fill_cells: make("fill_cells"),
            emit_counts: make("emit_counts"),
            strokes: None,
            dashes: None,
        });
        if batch.stroke_total > 0 {
            pipes.strokes.get_or_insert_with(|| StrokePipes {
                stroke_vertices: make("stroke_vertices"),
                stroke_emit: make("stroke_emit"),
                bin_strokes: make("bin_strokes"),
            });
        }
        if batch.dash_total > 0 {
            pipes.dashes.get_or_insert_with(|| DashPipes {
                dash_length: make("dash_length"),
                dash_scan_reduce: make("dash_scan_reduce"),
                dash_scan_blocks: make("dash_scan_blocks"),
                dash_scan_apply: make("dash_scan_apply"),
                dash_fragments: make("dash_fragments"),
                dash_args: make("dash_args"),
                dash_emit: make("dash_emit"),
                bin_dashes: make("bin_dashes"),
            });
        }
    }
}

/// Queue writes suffice for one batch. Multiple batches must upload between
/// their compute passes rather than all writing the shared destinations before submit.
struct BatchUpload<'a> {
    queue: &'a wgpu::Queue,
    ordered: bool,
    bytes: Vec<u8>,
    copies: Vec<(wgpu::Buffer, u64, u64, u64)>,
}

impl<'a> BatchUpload<'a> {
    const fn new(queue: &'a wgpu::Queue, ordered: bool) -> Self {
        Self {
            queue,
            ordered,
            bytes: Vec::new(),
            copies: Vec::new(),
        }
    }

    fn write_buffer(&mut self, target: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if !self.ordered {
            self.queue.write_buffer(target, offset, bytes);
            return;
        }
        let source_offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(bytes);
        self.copies
            .push((target.clone(), offset, source_offset, bytes.len() as u64));
    }

    fn finish(
        self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<wgpu::Buffer> {
        if !self.ordered {
            return None;
        }
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("coverage batch upload"),
            contents: &self.bytes,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        for (target, offset, source_offset, size) in self.copies {
            encoder.copy_buffer_to_buffer(&source, source_offset, &target, offset, size);
        }
        Some(source)
    }
}

struct BatchPlan {
    params: [u32; PARAMS_WORDS],
    sizes: [u64; 7],
    work_header_bytes: u64,
    radix_pass_count: u32,
}

impl BatchPlan {
    fn new(batch: Batch) -> Self {
        let FrameTotals {
            strips: strip_total,
            tiles: tile_total,
            ..
        } = batch.totals;
        // The compute stroker's scratch precedes the bins: the vertex slots,
        // then the dashed passes' per-record words, scan blocks, staging and
        // compacted list. It is empty in a frame without strokes, which
        // leaves every later region where it was (design section 8). Dashed
        // records are the contiguous tail of the record order (7.4).
        let scratch = batch.stroke_scratch();
        let StrokeScratch {
            stroke_total,
            dash_total,
            dash_first,
            dash_blocks,
            dash_words_base,
            scan_words_base,
            staging_base,
            list_base,
            bin_base,
        } = scratch;
        let fragment_base = bin_base + batch.bin_total * 3;
        let sorted = fragment_base + batch.totals.fragments * 8;
        let groups = sorted + batch.totals.fragments * 2;
        let sort_scratch = groups + group_words(batch.totals.fragments, batch.totals.columns);
        let radix_pass_count = radix_passes(batch.totals.columns);
        // Start on the opposite side for odd pass counts so the result always
        // precedes the group arena, leaving sort_scratch free for tokens.
        let (sort_a, sort_b) = if radix_pass_count.is_multiple_of(2) {
            (sorted, sort_scratch)
        } else {
            (sort_scratch, sorted)
        };
        let histogram = sort_scratch + batch.totals.fragments * 2;
        // Classification starts after radix sorting has released sort_scratch and
        // its histogram. The sorted keys stay live through texel generation.
        let tokens = sort_scratch;
        let params: [u32; PARAMS_WORDS] = [
            batch.element_total,
            batch.row_total,
            strip_total,
            u32::try_from(batch.end - batch.start).expect("coverage capacity fits u32"),
            tile_total,
            CLASSIFY_WIDTH,
            batch.totals.columns,
            bin_base,
            sort_scratch,
            fragment_base,
            batch.totals.fragments,
            sort_a,
            sort_b,
            histogram,
            groups,
            tokens,
            sorted,
            0,
            stroke_total,
            dash_total,
            dash_first,
            dash_blocks,
            0,
            dash_words_base,
            scan_words_base,
            staging_base,
            list_base,
            0,
        ];
        let cov_words = u64::from(groups) + batch.totals.scratch_words();
        let scratch_bytes = cov_words * 4;
        let work_header_bytes = u64::from(5 + batch.row_total) * 4;
        let strokes_bytes = (u64::from(stroke_total) * u64::from(RECORD_WORDS)
            + u64::from(batch.dash_table_words))
            * 4;
        let work_bytes = work_header_bytes + u64::from(strip_total + tile_total) * 8;
        let sizes = [
            scratch_bytes,
            u64::from(batch.element_total) * 48,
            strokes_bytes,
            (batch.end - batch.start) as u64 * PATH_WORDS as u64 * 4,
            u64::from(batch.seg_total) * 16,
            DISPATCH_ARGS_WORDS * 4,
            work_bytes,
        ];
        Self {
            params,
            sizes,
            work_header_bytes,
            radix_pass_count,
        }
    }
}

/// Where the compute stroker's scratch regions sit at the start of the
/// `coverage` buffer, in words: the vertex slots, the dashed passes'
/// per-record words and scan blocks, the fragment staging and the compacted
/// list, then the bins (design section 8). A frame without strokes has no
/// region, so every later region stays where it was.
#[derive(Clone, Copy, Debug)]
struct StrokeScratch {
    stroke_total: u32,
    dash_total: u32,
    /// Absolute index of the first dashed record; dashed records are the
    /// contiguous tail of the record order (7.4).
    dash_first: u32,
    dash_blocks: u32,
    dash_words_base: u32,
    scan_words_base: u32,
    staging_base: u32,
    list_base: u32,
    bin_base: u32,
}

impl Batch {
    const fn stroke_scratch(&self) -> StrokeScratch {
        let (stroke_total, dash_total) = (self.stroke_total, self.dash_total);
        let dash_blocks = dash_total.div_ceil(SCAN_BLOCK);
        let dash_words_base = stroke_total * SLOTS * SLOT_WORDS;
        let scan_words_base = dash_words_base + dash_total * DASH_RECORD_WORDS;
        let staging_base = scan_words_base + dash_blocks * SCAN_BLOCK_WORDS;
        let list_base = staging_base + self.stage_total * STAGE_WORDS;
        StrokeScratch {
            stroke_total,
            dash_total,
            dash_first: stroke_total - dash_total,
            dash_blocks,
            dash_words_base,
            scan_words_base,
            staging_base,
            list_base,
            bin_base: list_base + self.list_total * LIST_WORDS,
        }
    }
}

/// The device's coverage binding limit,
/// `max_storage_buffer_binding_size.min(max_buffer_size)`: every buffer
/// the coverage kernels bind must fit it.
#[derive(Clone, Copy, Debug)]
pub struct CoverageLimit {
    bytes: u64,
}

impl CoverageLimit {
    pub fn new(limits: &wgpu::Limits) -> Self {
        Self {
            bytes: limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size),
        }
    }

    const fn words(self) -> u64 {
        self.bytes / 4
    }

    /// The capacity term of the automatic route's predicate (decisions R1
    /// and R2, as water-rs/waterui#2544 amended them): whether the rows the
    /// single draw of `path`, placed at `bound`, would add to an otherwise
    /// empty batch each fit the binding limit and, with the batch's fixed
    /// bytes, the per-batch cap. Its inputs are the path's content, scale
    /// and placement, the surface size and the device limit, so it is as
    /// deterministic as the rest of the predicate. A path it refuses takes
    /// the CPU sweep. The rows are the ones [`GpuCoverage::draw`] adds, from
    /// the same [`path_rows`].
    pub fn admits(self, path: &GpuPath, bound: &GpuBound) -> bool {
        path_rows(path, bound, FrameTotals::default()).is_ok_and(|(rows, _)| {
            rows.largest() <= self.bytes && rows.total() + FRAME_FIXED_BYTES <= GPU_COV_MAX_BYTES
        })
    }
}

/// The running totals of a batch's misses that one more path's rows
/// depend on: the sort groups and tokens are sized from the frame's
/// fragments and strip columns together.
#[derive(Clone, Copy, Default)]
struct FrameTotals {
    fragments: u32,
    columns: u32,
    strips: u32,
    tiles: u32,
}

impl FrameTotals {
    /// Sort groups remain live; radix sort, classification tokens and tile
    /// records reuse the following arena at different points in the prologue.
    fn scratch_words(self) -> u64 {
        let radix = u64::from(self.fragments) * 2
            + u64::from(self.fragments.div_ceil(WORKGROUP).div_ceil(256)) * 256 * 16;
        let tokens = (u64::from(self.fragments.min(self.columns)) * 2 + u64::from(self.strips)) * 3;
        u64::from(group_words(self.fragments, self.columns))
            + radix.max(tokens).max(u64::from(self.tiles) * 4)
    }
}

/// The rows the draw of `path` at `bound` adds to decision R2's bound of
/// a frame whose earlier misses reached `before`, by buffer, and the
/// totals after it. [`GpuCoverage::draw`] and [`CoverageLimit::admits`]
/// both size a path from here, so the route predicate and the frame's
/// check cannot disagree on what one path costs.
fn path_rows(
    path: &GpuPath,
    bound: &GpuBound,
    before: FrameTotals,
) -> Result<(BufferBounds, FrameTotals), RenderError> {
    let rows = bound.grid_h;
    let strips = bound.grid_h.div_ceil(crate::render::path::STRIP_H);
    let to64 = |n: usize| u64::try_from(n).expect("bound fits u64");
    let key_range = || RenderError::Unsupported("GPU coverage key range exceeds u32");
    let tiles = strips * bound.grid_w.div_ceil(CLASSIFY_WIDTH as usize);
    // The sort groups take up to 16 words per fragment at u32 word
    // offsets, so a frame past this many fragments cannot be laid out.
    let after = FrameTotals {
        fragments: u32::try_from(bound.fragments)
            .ok()
            .and_then(|fragments| before.fragments.checked_add(fragments))
            .filter(|&fragments| fragments <= u32::MAX / 16)
            .ok_or(RenderError::Unsupported(
                "GPU coverage fragment bound exceeds u32 addressing",
            ))?,
        columns: u32::try_from(bound.grid_w * strips)
            .ok()
            .and_then(|columns| before.columns.checked_add(columns))
            .ok_or_else(key_range)?,
        strips: before.strips + u32::try_from(strips).map_err(|_| key_range())?,
        tiles: before.tiles + u32::try_from(tiles).map_err(|_| key_range())?,
    };
    let mut bounds = BufferBounds {
        elements: to64(path.elements.len()) * (std::mem::size_of::<GpuElement>() as u64),
        segments: to64(bound.segments) * 16,
        scratch: to64(bound.bins) * 12
            + to64(bound.fragments) * 40
            + (after.scratch_words() - before.scratch_words()) * 4,
        work: to64(rows) * 4 + (to64(strips) + to64(tiles)) * 8,
        paths: (PATH_WORDS as u64) * 4,
        arena: to64(bound.texels.div_ceil(4) + bound.cell_cap) * 4
            + to64(bound.instance_cap) * u64::from(GEOMETRY_WORDS) * 4,
        unbound: 16 + u64::from(ARGS_WORDS) * 4,
        ..BufferBounds::default()
    };
    bounds.add(&stroke_bounds(path.strokes.len(), path.dash.as_deref()));
    Ok((bounds, after))
}

/// The bytes a stroke path's records bound (design section 8), by buffer:
/// the records, and for a dashed path its cumulative-length table, in the
/// strokes buffer; the vertex scratch, and for a dashed path the per-record
/// dash words, the staging and list entries and the scan blocks, in the
/// coverage scratch.
fn stroke_bounds(records: usize, dash: Option<&DashTable>) -> BufferBounds {
    let to64 = |n: usize| u64::try_from(n).expect("bound fits u64");
    let mut bounds = BufferBounds {
        strokes: to64(records) * std::mem::size_of::<GpuStrokeRecord>() as u64,
        scratch: to64(records) * u64::from(SLOTS * SLOT_WORDS) * 4,
        ..BufferBounds::default()
    };
    if let Some(dash) = dash {
        bounds.strokes += to64(dash.cum.len()) * 8;
        bounds.scratch += to64(records) * u64::from(DASH_RECORD_WORDS) * 4
            + to64(dash.stage_total) * u64::from(STAGE_WORDS) * 4
            + to64(dash.fragments) * u64::from(LIST_WORDS) * 4
            + to64(records.div_ceil(SCAN_BLOCK as usize)) * u64::from(SCAN_BLOCK_WORDS) * 4;
    }
    bounds
}

/// Boundary records use the smaller of sorted-fragment indices and logical
/// strip columns. Radix masks and prefixes borrow the same arena first.
const fn group_words(fragments: u32, columns: u32) -> u32 {
    if fragments == 0 {
        return 0;
    }
    let records = if fragments < columns {
        fragments
    } else {
        columns
    };
    let radix = fragments + 32 + fragments.div_ceil(16384) * 16 + 16;
    let coverage = records * 16;
    if coverage > radix { coverage } else { radix }
}

/// Four-bit radix passes that order every key below `key_end` and keep the
/// all-ones empty-slot sentinel after them: the sentinel's low `4 * passes`
/// bits are all ones, which exceeds every key once `key_end <= 2^(4 * passes)`.
/// The initial buffer depends on parity so the sorted keys finish in the
/// fixed output arena without an extra pass.
const fn radix_passes(key_end: u32) -> u32 {
    key_end.bit_width().div_ceil(4)
}

fn allocation_sizes(mut sizes: [u64; 7]) -> [u64; 7] {
    sizes[1] = sizes[1].max(48);
    sizes[2] = sizes[2].next_multiple_of(16).max(16);
    sizes[4] = sizes[4].max(16);
    sizes
}

fn sized_buffer(
    device: &wgpu::Device,
    previous: Option<&wgpu::Buffer>,
    label: &'static str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    if let Some(buffer) = previous.filter(|buffer| buffer.size() >= size) {
        return buffer.clone();
    }
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::stroke_abi::{CLOSE, FRAG_START_CAP};
    use kurbo::{Affine, PathEl, Point};

    #[test]
    fn coverage_pipelines_follow_later_stroke_and_dash_use() {
        let Some((device, _, delivery)) = delivery_device() else {
            return;
        };
        let mut coverage = GpuCoverage::new(&device, u64::MAX);
        let mut batch = Batch::default();
        coverage.ensure_pipes(&device, delivery, batch);
        let pipes = coverage.pipes.as_ref().unwrap();
        let fill = pipes.flatten.clone();
        assert!(pipes.strokes.is_none());
        assert!(pipes.dashes.is_none());

        batch.stroke_total = 1;
        coverage.ensure_pipes(&device, delivery, batch);
        let pipes = coverage.pipes.as_ref().unwrap();
        let stroke = pipes.strokes.as_ref().unwrap().stroke_emit.clone();
        assert_eq!(pipes.flatten, fill);
        assert!(pipes.dashes.is_none());

        batch.dash_total = 1;
        coverage.ensure_pipes(&device, delivery, batch);
        let pipes = coverage.pipes.as_ref().unwrap();
        assert_eq!(pipes.flatten, fill);
        assert_eq!(pipes.strokes.as_ref().unwrap().stroke_emit, stroke);
        assert!(pipes.dashes.is_some());
    }

    #[test]
    fn radix_passes_order_keys_below_the_sentinel() {
        assert_eq!(radix_passes(255), 2);
        assert_eq!(radix_passes(256), 3);
        assert_eq!(radix_passes(1 << 16), 5);
        assert_eq!(radix_passes(1 << 24), 7);
        assert_eq!(radix_passes(u32::MAX), 8);
    }

    #[test]
    fn dense_path_scratch_bound_covers_radix_sort() {
        let Some(device) = device() else {
            return;
        };
        let mut elements = Vec::new();
        for _ in 0..256 {
            elements.extend([
                PathEl::MoveTo(Point::new(0.0, 0.0)),
                PathEl::LineTo(Point::new(64.0, 64.0)),
                PathEl::LineTo(Point::new(0.0, 64.0)),
                PathEl::ClosePath,
            ]);
        }
        let path = crate::render::path::prepare_gpu(&elements, Affine::IDENTITY, (64.0, 64.0));
        let mut frame = GpuCoverage::new(&device, u64::MAX);
        frame.begin_frame();
        frame
            .draw(&GpuDraw::new(1, 0, Affine::IDENTITY, path, (64, 64)), true)
            .expect("dense path fits");
        let plan = BatchPlan::new(frame.batch);
        assert!(
            plan.sizes[0] <= frame.batch.bound.scratch,
            "allocation {} exceeds admitted scratch {}",
            plan.sizes[0],
            frame.batch.bound.scratch
        );
    }

    #[test]
    fn sparse_capacity_tracks_perimeter_and_rejects_over_budget() {
        fn rectangle(size: u32) -> crate::render::path::GpuPath {
            let edge = f64::from(size);
            crate::render::path::prepare_gpu(
                &[
                    PathEl::MoveTo(Point::new(0.0, 0.0)),
                    PathEl::LineTo(Point::new(edge, 0.0)),
                    PathEl::LineTo(Point::new(edge, edge)),
                    PathEl::LineTo(Point::new(0.0, edge)),
                    PathEl::ClosePath,
                ],
                Affine::IDENTITY,
                (edge, edge),
            )
        }
        let small = rectangle(1024).bound;
        let large_path = rectangle(4096);
        let large = large_path.bound;
        assert!(large.fragments <= 4 * small.fragments);
        assert!(large.texels <= 4 * small.texels);
        assert!(large.cell_cap <= 4 * small.cell_cap);
        let Some(device) = device() else {
            return;
        };
        let mut frame = GpuCoverage::new(&device, u64::MAX);
        frame.begin_frame();
        frame
            .draw(
                &GpuDraw::new(1, 0, Affine::IDENTITY, large_path, (4096, 4096)),
                true,
            )
            .expect("sparse 4096-square capacity");
        assert!(frame.batch.bound.total() < GPU_COV_MAX_BYTES);
        let huge_path = rectangle(1_000_000);
        let before = frame.paths.len();
        assert!(
            frame
                .draw(
                    &GpuDraw::new(2, 0, Affine::IDENTITY, huge_path, (1_000_000, 1_000_000),),
                    true
                )
                .is_err()
        );
        assert_eq!(frame.paths.len(), before, "reject before appending a path");
    }

    #[test]
    fn draw_refuses_a_bound_over_the_device_binding_limit() {
        let rectangle = |size: u32| {
            let edge = f64::from(size);
            crate::render::path::prepare_gpu(
                &[
                    PathEl::MoveTo(Point::new(0.0, 0.0)),
                    PathEl::LineTo(Point::new(edge, 0.0)),
                    PathEl::LineTo(Point::new(edge, edge)),
                    PathEl::LineTo(Point::new(0.0, edge)),
                    PathEl::ClosePath,
                ],
                Affine::IDENTITY,
                (edge, edge),
            )
        };
        let Some(device) = device() else {
            return;
        };
        let mut probe = GpuCoverage::new(&device, u64::MAX);
        probe.begin_frame();
        probe
            .draw(
                &GpuDraw::new(1, 0, Affine::IDENTITY, rectangle(4096), (4096, 4096)),
                true,
            )
            .expect("fits the device's own limits");
        // A device whose binding limit sits below the largest single
        // buffer's bound of this draw, far under the 256 MiB cap.
        let small = wgpu::Limits {
            max_storage_buffer_binding_size: probe.batch.bound.largest() / 2,
            ..wgpu::Limits::default()
        };
        let mut frame = GpuCoverage::with_limits(&small, u64::MAX);
        frame.begin_frame();
        assert!(
            frame
                .draw(
                    &GpuDraw::new(1, 0, Affine::IDENTITY, rectangle(4096), (4096, 4096),),
                    true
                )
                .is_err(),
            "draw must refuse what record_prologue would reject"
        );
        assert_eq!(frame.paths.len(), 0, "reject before appending a path");
        // A limit above every single buffer's bound but below their sum:
        // the frame fits each binding, so it is accepted.
        assert!(probe.batch.bound.largest() < probe.batch.bound.total());
        let between = wgpu::Limits {
            max_storage_buffer_binding_size: probe.batch.bound.largest(),
            ..wgpu::Limits::default()
        };
        let mut frame = GpuCoverage::with_limits(&between, u64::MAX);
        frame.begin_frame();
        frame
            .draw(
                &GpuDraw::new(1, 0, Affine::IDENTITY, rectangle(4096), (4096, 4096)),
                true,
            )
            .expect("every buffer fits the binding limit");
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one scene setup, GPU readback and commit lifecycle check"
    )]
    fn road_paths_share_bounded_scratch() {
        let Some((device, queue, delivery)) = delivery_device() else {
            return;
        };
        let scene = cherenkov_scene::Scene::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scenes/perf/road-network-batches"),
        )
        .expect("generated road scene");
        let cherenkov_scene::Item::Layer(layer) = &scene.root.items[0] else {
            panic!("road layer")
        };
        let draws: Vec<_> = layer
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let cherenkov_scene::Item::Draw(cherenkov_scene::Draw::Stroke {
                    shape,
                    stroke,
                    ..
                }) = item
                else {
                    panic!("road stroke")
                };
                let shape = cherenkov::ShapeData::Path {
                    elements: shape.to_path().elements().into(),
                    rule: cherenkov::FillRule::NonZero,
                };
                let path = crate::render::path::stroke::prepare(
                    &shape,
                    &stroke.into(),
                    Affine::IDENTITY,
                    crate::render::path::FLATTEN,
                    (1024.0, 768.0),
                )
                .expect("road encodes");
                GpuDraw::new(index as u64 + 1, 0, Affine::IDENTITY, path, (1024, 768))
            })
            .collect();
        let [first, second] = &draws[..] else {
            panic!("two road layers")
        };
        let mut frame = GpuCoverage::new(&device, u64::MAX);
        let output_bytes: u64 = draws
            .iter()
            .map(|draw| {
                assert!(frame.limit.admits(&draw.path, &draw.path.bound));
                path_rows(&draw.path, &draw.path.bound, FrameTotals::default())
                    .expect("path rows")
                    .0
                    .arena
            })
            .sum();
        assert!(
            output_bytes + 64 < frame.limit.bytes,
            "both retained outputs fit"
        );
        frame.begin_frame();
        frame.draw(first, false).expect("first road fits");
        frame
            .draw(second, true)
            .expect("individually admitted roads share bounded scratch");
        assert_eq!(frame.batches.len(), 1, "two dispatch batches");
        frame.begin_frame();
        frame
            .draw(first, false)
            .expect("first road after abandoned commit");
        frame
            .draw(second, true)
            .expect("second road after abandoned commit");
        assert_eq!(frame.generated(), 2, "unsubmitted results are regenerated");
        assert!(
            frame.arena().is_none(),
            "planning allocates no intermediate arenas"
        );
        let planned_bytes = frame.planned_bytes();
        frame.ensure(&device).expect("coverage buffers fit");
        assert_eq!(
            frame.bytes(),
            planned_bytes,
            "admission budgets actual buffer allocations"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        frame.record_prologue(&device, &queue, &mut encoder, delivery, None);
        let arena = frame.arena().expect("retained arena");
        let first_entry = &frame.entries[&first.key];
        let second_entry = &frame.entries[&second.key];
        let copies = [
            (arena, u64::from(first_entry.base) * 4, 4),
            (arena, u64::from(second_entry.base) * 4, 4),
            (frame.draws_buffer(), 0, u64::from(ARGS_WORDS) * 8),
        ];
        let words = read_back(&device, &queue, &copies, encoder);
        for (index, entry) in [first_entry, second_entry].into_iter().enumerate() {
            let count = words[index][0];
            assert!(
                count > 0 && count <= entry.geometry_slots,
                "batch writes its own output block"
            );
            for offset in [
                1,
                ARGS_PARTIAL as usize / 4 + 1,
                ARGS_NORMAL as usize / 4 + 1,
            ] {
                assert_eq!(words[2][index * ARGS_WORDS as usize + offset], count);
            }
        }
        let generation_bytes = frame.generation_resident_bytes();
        assert!(generation_bytes > 0);
        assert_eq!(
            frame.generation_reservation(),
            0,
            "resident scratch is already counted"
        );
        frame.submitted();
        assert_eq!(frame.generation_reservation(), generation_bytes);
        assert!(
            frame.buffers.is_none(),
            "idle frames release generation scratch"
        );
        assert_eq!(frame.uploads.len(), 0);
        frame.begin_frame();
        frame.draw(first, false).expect("first cache hit");
        frame.draw(second, true).expect("second cache hit");
        assert!(!frame.has_misses(), "both batches remain cached");
        let arena_generation = frame.arena_generation();
        let resident = frame.bytes();
        frame.compact(0).expect("first planned repack");
        frame.compact(0).expect("second planned repack");
        assert_eq!(frame.arena_generation(), arena_generation);
        assert_eq!(frame.bytes(), resident, "copy source is counted once");
        frame.ensure(&device).expect("coverage buffers fit");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        frame.record_prologue(&device, &queue, &mut encoder, delivery, None);
        let replay = read_back(
            &device,
            &queue,
            &[(frame.draws_buffer(), 0, u64::from(ARGS_WORDS) * 8)],
            encoder,
        );
        assert_eq!(replay[0], words[2], "cached coverage preserves draw counts");
        frame.submitted();
        assert!(
            frame.buffers.is_none(),
            "replay releases generation scratch"
        );
        frame.finish_pictures();
        assert!(!frame.entries.contains_key(&first.key));
        frame.begin_frame();
        frame
            .draw(second, false)
            .expect("ordinary entry shared with a picture");
        assert!(!frame.has_misses());
        frame.settle(true);
        frame.ensure(&device).expect("ordinary coverage fits");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        frame.record_prologue(&device, &queue, &mut encoder, delivery, None);
        let survivor = &frame.entries[&second.key];
        let surviving = read_back(
            &device,
            &queue,
            &[(
                frame.arena().expect("ordinary arena"),
                u64::from(survivor.base) * 4,
                4,
            )],
            encoder,
        );
        assert_eq!(
            surviving[0], words[1],
            "ordinary coverage survives retirement"
        );
        frame.submitted();
        frame.finish_pictures();
        assert!(
            frame.entries.contains_key(&second.key),
            "ordinary sharing remains retained"
        );
        frame.trim();
        assert_eq!(frame.generation_reservation(), 0);
    }

    fn device() -> Option<wgpu::Device> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .next()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .ok()
            .map(|(device, _)| device)
    }

    /// A device that takes the build's shader delivery, with default limits:
    /// the compute stroker's layout must fit the baseline storage-buffer
    /// count.
    fn delivery_device() -> Option<(
        wgpu::Device,
        wgpu::Queue,
        crate::render::shaders::ShaderDelivery,
    )> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .next()?;
        let backend = adapter.get_info().backend;
        let required_features = match backend {
            wgpu::Backend::Vulkan | wgpu::Backend::Metal | wgpu::Backend::Dx12 => {
                wgpu::Features::PASSTHROUGH_SHADERS
            }
            _ => wgpu::Features::empty(),
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            ..wgpu::DeviceDescriptor::default()
        }))
        .ok()?;
        let delivery = crate::render::shaders::delivery(backend, &device)
            .expect("the device takes passthrough");
        Some((device, queue, delivery))
    }

    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        copies: &[(&wgpu::Buffer, u64, u64)],
        encoder: wgpu::CommandEncoder,
    ) -> Vec<Vec<u32>> {
        let mut encoder = encoder;
        let targets: Vec<wgpu::Buffer> = copies
            .iter()
            .map(|&(_, _, size)| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("stroke readback"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();
        for (&(source, offset, size), target) in copies.iter().zip(&targets) {
            encoder.copy_buffer_to_buffer(source, offset, target, 0, size);
        }
        queue.submit([encoder.finish()]);
        for target in &targets {
            target.map_async(wgpu::MapMode::Read, .., |result| {
                result.expect("map readback");
            });
        }
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(120)),
            })
            .expect("the stroke kernels finish");
        targets
            .iter()
            .map(|target| {
                let words =
                    bytemuck::cast_slice(&target.get_mapped_range(..).expect("mapped range"))
                        .to_vec();
                target.unmap();
                words
            })
            .collect()
    }

    fn unordered(value: u32) -> f32 {
        f32::from_bits(if value & 0x8000_0000 == 0 {
            !value
        } else {
            value ^ 0x8000_0000
        })
    }

    /// Strokes of every shape the encoder has to survive, as
    /// `(centerline, stroke)`.
    #[expect(
        clippy::too_many_lines,
        reason = "a table of the shapes and styles the encoder has to survive"
    )]
    fn stroke_cases() -> Vec<(kurbo::BezPath, kurbo::Stroke)> {
        use kurbo::{BezPath, Cap, Join, Stroke};
        let mut cases = Vec::new();
        let zigzag = |scale: f64| {
            let mut path = BezPath::new();
            path.move_to((60.0, 80.0));
            path.line_to((140.0 * scale, 90.0));
            path.line_to((90.0, 200.0 * scale));
            path.line_to((200.0, 160.0));
            path.line_to((200.0, 160.0));
            path.line_to((120.0, 150.0));
            path
        };
        let curvy = || {
            let mut path = BezPath::new();
            path.move_to((50.0, 300.0));
            path.curve_to((150.0, 180.0), (90.0, 420.0), (260.0, 300.0));
            path.quad_to((300.0, 200.0), (340.0, 330.0));
            path
        };
        let cusp = || {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.curve_to((220.0, 220.0), (100.0, 220.0), (220.0, 100.0));
            path
        };
        let looped = || {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.curve_to((260.0, 260.0), (40.0, 260.0), (200.0, 100.0));
            path
        };
        let degenerate = || {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.curve_to((100.0, 100.0), (200.0, 140.0), (240.0, 100.0));
            path.curve_to((300.0, 100.0), (180.0, 100.0), (260.0, 100.0));
            path
        };
        let closed = |side: f64| {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.line_to((100.0 + side, 100.0));
            path.line_to((100.0 + side, side.mul_add(0.5, 100.0)));
            path.line_to((100.0, side.mul_add(0.5, 100.0)));
            path.close_path();
            path
        };
        let triangle = || {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.line_to((300.0, 120.0));
            path.line_to((120.0, 130.0));
            path.close_path();
            path
        };
        let reversal = || {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.line_to((250.0, 100.0));
            path.line_to((150.0, 100.0));
            path
        };
        let joins = [Join::Bevel, Join::Miter, Join::Round];
        let caps = [Cap::Butt, Cap::Square, Cap::Round];
        for (n, &join) in joins.iter().enumerate() {
            for (m, &cap) in caps.iter().enumerate() {
                let width = [0.4, 7.0, 31.0][(n + m) % 3];
                let mut stroke = Stroke::new(width)
                    .with_join(join)
                    .with_miter_limit([1.0, 4.0, 10.0][n]);
                stroke.start_cap = cap;
                stroke.end_cap = caps[(m + 1) % 3];
                for path in [
                    zigzag(1.0),
                    zigzag(0.5),
                    curvy(),
                    cusp(),
                    looped(),
                    degenerate(),
                    closed(120.0),
                    triangle(),
                    reversal(),
                ] {
                    cases.push((path, stroke.clone()));
                }
            }
        }
        let patterns: [(&[f64], f64); 7] = [
            (&[20.0, 10.0], 0.0),
            (&[15.0], 4.0),
            (&[0.0, 3.0, 6.0, 2.0], 0.0),
            (&[8.0, 0.0, 4.0, 4.0], 1.0),
            (&[12.0, 5.0, 3.0], 7.0),
            (&[0.5, 0.5], 0.25),
            (&[40.0, 25.0], 33.0),
        ];
        for (n, &(pattern, offset)) in patterns.iter().enumerate() {
            for (m, &cap) in caps.iter().enumerate() {
                let stroke = Stroke::new([1.5, 6.0, 20.0][m])
                    .with_join(joins[(n + m) % 3])
                    .with_caps(cap)
                    .with_dashes(offset, pattern.iter().copied());
                for path in [
                    zigzag(1.0),
                    curvy(),
                    cusp(),
                    looped(),
                    degenerate(),
                    closed(120.0),
                ] {
                    cases.push((path, stroke.clone()));
                }
            }
        }
        // Ties on an integer rectangle, a merged and an unmerged closed
        // subpath. Offset 20 makes the dashes of [20, 10] end at s = 30k,
        // on the rectangle's vertices.
        for offset in [0.0, 5.0, 20.0, 25.0] {
            let mut path = BezPath::new();
            path.move_to((100.0, 100.0));
            path.line_to((220.0, 100.0));
            path.line_to((220.0, 160.0));
            path.line_to((100.0, 160.0));
            path.close_path();
            cases.push((path, Stroke::new(4.0).with_dashes(offset, [20.0, 10.0])));
        }
        cases
    }

    /// Mitered polylines whose every join turns by the same angle: small
    /// turns, where the miter row is the inner side's `2·s·hw`, and turns
    /// just inside and just past the miter limit, where the outer side's
    /// `hw·tan ψ` meets `ml` (design section 6.5). Each runs in a frame of
    /// its own so no other path's slack hides its join rows.
    fn miter_cases() -> Vec<(kurbo::BezPath, kurbo::Stroke)> {
        use kurbo::{BezPath, Join, Stroke, Vec2};
        let polyline = |turn: f64, alternate: bool| {
            let mut path = BezPath::new();
            // Inside the 1024-square surface at either test scale.
            let mut at = kurbo::Point::new(150.0, 420.0);
            let mut heading = 0.0_f64;
            path.move_to(at);
            for k in 0..9 {
                at += Vec2::from_angle(heading) * 50.0;
                path.line_to(at);
                heading += if alternate && k % 2 == 1 { -turn } else { turn };
            }
            path
        };
        let mut cases = Vec::new();
        for (turn, alternate) in [(0.02, false), (0.1, true), (0.6, true)] {
            for width in [3.0, 31.0] {
                cases.push((
                    polyline(turn, alternate),
                    Stroke::new(width)
                        .with_join(Join::Miter)
                        .with_miter_limit(4.0),
                ));
            }
        }
        for limit in [1.5_f64, 4.0, 10.0] {
            // A miter passes while `tan ψ < sqrt(ml² - 1)`, `ψ` half the turn.
            let edge = 2.0 * limit.mul_add(limit, -1.0).sqrt().atan();
            for turn in [edge - 1e-4, edge + 1e-4] {
                for alternate in [false, true] {
                    cases.push((
                        polyline(turn, alternate),
                        Stroke::new(31.0)
                            .with_join(Join::Miter)
                            .with_miter_limit(limit),
                    ));
                }
            }
        }
        cases
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one frame's setup, readback and assertions"
    )]
    fn stroke_segments_close_and_fit_their_bound() {
        use crate::render::path::FLATTEN;
        use cherenkov::ShapeData;
        use std::collections::HashMap;

        // The used-versus-allocated numbers below are the over-allocation
        // evidence design section 6.6 asks for; they must be visible on a
        // plain `--no-capture` run.
        let _tracing = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_test_writer()
                .with_max_level(tracing::level_filters::LevelFilter::DEBUG)
                .finish(),
        );
        let Some((device, queue, delivery)) = delivery_device() else {
            return;
        };
        let started = std::time::Instant::now();
        let cases = stroke_cases();
        // More than 16384 dashed records, so the frame's `dash_blocks`
        // exceeds 64 and `dash_scan_blocks` carries across chunks. The
        // bevel join, butt caps and a frame of its own keep the bound under
        // the 128 MiB binding limit: kurbo's default round caps on each of
        // its ~13,000 dashes would otherwise dominate the bound.
        // Round caps on dashes are covered by the cases above.
        let mut long = kurbo::BezPath::new();
        long.move_to((0.0, 0.0));
        for k in 1..=17_000u32 {
            long.line_to((f64::from(k % 2) * 6.0, f64::from((k / 2) % 256) * 0.5));
        }
        let singles = [(
            long,
            kurbo::Stroke::new(2.0)
                .with_join(kurbo::Join::Bevel)
                .with_caps(kurbo::Cap::Butt)
                .with_dashes(0.0, [5.0, 3.0]),
        )];
        // The default device's binding limit bounds one frame, so the cases
        // go through in several.
        let miters = miter_cases();
        let mut index = 0usize;
        for chunk in cases
            .chunks(24)
            .chain(singles.chunks(1))
            .chain(miters.chunks(1))
        {
            let mut frame = GpuCoverage::new(&device, u64::MAX);
            frame.begin_frame();
            let mut kept = Vec::new();
            for (path, stroke) in chunk {
                let raster = Affine::translate((8.0, 8.0))
                    * Affine::scale(if index.is_multiple_of(2) { 1.0 } else { 1.5 });
                index += 1;
                let shape = ShapeData::Path {
                    elements: path.elements().into(),
                    rule: cherenkov::FillRule::NonZero,
                };
                let gpu = crate::render::path::stroke::prepare(
                    &shape,
                    stroke,
                    raster,
                    FLATTEN / raster.spectral_norm().max(1e-12),
                    (1024.0, 1024.0),
                )
                .expect("the stroke encodes");
                if gpu.strokes.is_empty() {
                    continue;
                }
                let content = u64::try_from(index).expect("case index") + 1;
                // The draw is placement-relative, so its bound is the one
                // the kernels' bbox lies in.
                let draw = GpuDraw::new(content, 0, raster, gpu, (1024, 1024));
                frame
                    .draw(&draw, true)
                    .unwrap_or_else(|error| panic!("case {index} draws: {error:?}"));
                kept.push((index, draw.path));
            }
            // A NaN tail past `seg_total` catches a chord written beyond
            // the last path's segments: `dispatch` reuses a large enough
            // buffer as is, so the tail sits inside the bound buffer.
            // The other buffers are stubs `sized_buffer` replaces.
            let guard = u64::from(frame.batch.seg_total).max(1) * 16 + 256 * 16;
            let stub = |label| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::UNIFORM
                        | wgpu::BufferUsages::INDIRECT,
                    mapped_at_creation: false,
                })
            };
            frame.buffers = Some(Buffers {
                segments: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cov segments"),
                    size: guard,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                params: stub("cov params"),
                elements: stub("cov elements"),
                paths: stub("cov paths"),
                coverage: stub("cov scratch"),
                strokes: stub("cov strokes"),
                args: stub("cov args"),
                work: stub("cov work"),
            });
            frame.ensure(&device).expect("coverage buffers fit");
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            frame.record_prologue(&device, &queue, &mut encoder, delivery, None);
            let buffers = frame.buffers.as_ref().expect("buffers sized");
            // Every slot starts as NaN, so an unwritten chord cannot pass as
            // geometry.
            let nan = vec![0x7fc0_0000u32; usize::try_from(buffers.segments.size() / 4).unwrap()];
            queue.write_buffer(&buffers.segments, 0, bytemuck::cast_slice(&nan));
            let scratch = frame.batch.stroke_scratch();
            let path_bytes = frame.paths.len() as u64 * PATH_WORDS as u64 * 4;
            let region_bytes = u64::from(scratch.bin_base) * 4;
            let copies = [
                (&buffers.segments, 0, buffers.segments.size()),
                (&buffers.coverage, 0, region_bytes.max(4)),
                (&buffers.work, 0, 20),
                (&buffers.paths, 0, path_bytes),
            ];
            let words = read_back(&device, &queue, &copies, encoder);
            let [segments, region, work, paths] = &words[..] else {
                unreachable!("four readbacks")
            };
            let (bins_used, fragments_used) = (work[0], work[1]);
            let bins_bound: usize = kept.iter().map(|(_, gpu)| gpu.bound.bins).sum();
            let fragments_bound: usize = kept.iter().map(|(_, gpu)| gpu.bound.fragments).sum();
            assert!(
                (bins_used as usize) <= bins_bound,
                "bins {bins_used} > {bins_bound}"
            );
            assert!(
                (fragments_used as usize) <= fragments_bound,
                "fragments {fragments_used} > {fragments_bound}"
            );
            tracing::debug!(
                bins_used,
                bins_bound,
                fragments_used,
                fragments_bound,
                "stroke bins"
            );
            let mut chord_used_total = 0usize;
            let mut chord_bound_total = 0usize;
            let mut staged_used = 0usize;
            let mut staged_bound = 0usize;
            for (path_index, (case, gpu)) in kept.iter().enumerate() {
                let record = &paths[path_index * PATH_WORDS..(path_index + 1) * PATH_WORDS];
                let seg_base = record[10] as usize;
                let seg_bound = record[11] as usize;
                assert_eq!(seg_bound, gpu.bound.segments);
                let dashed = gpu.dash.as_deref();
                let first = record[36] as usize;
                let count = gpu.strokes.len();
                // Chords the kernels wrote for this path.
                let written = if let Some(dash) = dashed {
                    let mut chords = 0usize;
                    let mut fragments = 0usize;
                    let mut dash_count = 0usize;
                    staged_bound += dash.stage_total;
                    for rel in 0..count {
                        let j = first + rel - scratch.dash_first as usize;
                        let w = scratch.dash_words_base as usize + j * DASH_RECORD_WORDS as usize;
                        let staged = region[w + 4] as usize;
                        let stage_cap = frame.strokes[first + rel].head[1] as usize;
                        assert!(
                            staged <= stage_cap,
                            "case {case}: record {rel} staged {staged} > {stage_cap}"
                        );
                        fragments += staged;
                        chords += region[w + 5] as usize;
                        let head = frame.strokes[first + rel].head;
                        let stage_first = (record[44] + head[3]) as usize;
                        for k in 0..staged {
                            let at = scratch.staging_base as usize
                                + (stage_first + k) * STAGE_WORDS as usize;
                            if region[at + 2] & FRAG_START_CAP != 0 || head[0] & CLOSE != 0 {
                                dash_count += 1;
                            }
                        }
                    }
                    assert!(
                        fragments <= dash.fragments,
                        "case {case}: fragments {fragments} > {}",
                        dash.fragments
                    );
                    assert!(
                        chords <= seg_bound,
                        "case {case}: chords {chords} > {seg_bound}"
                    );
                    assert!(
                        dash_count <= dash.dashes,
                        "case {case}: dashes {dash_count} > {}",
                        dash.dashes
                    );
                    staged_used += fragments;
                    tracing::debug!(
                        case,
                        fragments,
                        fragment_bound = dash.fragments,
                        staged_bound = dash.stage_total,
                        dashes = dash_count,
                        dash_bound = dash.dashes,
                        chords,
                        chord_bound = seg_bound,
                        "dashed stroke"
                    );
                    chords
                } else {
                    tracing::debug!(case, chords = seg_bound, "undashed stroke");
                    seg_bound
                };
                chord_used_total += written;
                chord_bound_total += seg_bound;
                let mut balance: HashMap<(u32, u32), i64> = HashMap::new();
                for slot in seg_base..seg_base + seg_bound {
                    let chord = &segments[slot * 4..slot * 4 + 4];
                    if slot < seg_base + written {
                        assert!(
                            chord.iter().all(|&bits| f32::from_bits(bits).is_finite()),
                            "case {case}: chord slot {} is not finite",
                            slot - seg_base
                        );
                        *balance.entry((chord[0], chord[1])).or_default() += 1;
                        *balance.entry((chord[2], chord[3])).or_default() -= 1;
                    } else {
                        assert!(
                            chord.iter().all(|&bits| f32::from_bits(bits).is_nan()),
                            "case {case}: slot {} past the written chords was touched",
                            slot - seg_base
                        );
                    }
                }
                assert!(
                    balance.values().all(|&n| n == 0),
                    "case {case}: chords do not close"
                );
                let [x0, y0, x1, y1] = [0, 1, 2, 3].map(|a| unordered(record[a]));
                let rect = gpu.bound.rect;
                assert!(
                    x0 >= rect[0] && y0 >= rect[1] && x1 <= rect[2] && y1 <= rect[3],
                    "case {case}: bbox ({x0}, {y0}, {x1}, {y1}) outside {rect:?}"
                );
            }
            // The NaN tail past `seg_total` must be untouched.
            assert!(
                segments[frame.batch.seg_total as usize * 4..]
                    .iter()
                    .all(|&word| f32::from_bits(word).is_nan()),
                "a chord wrote past the last path's segments"
            );
            tracing::debug!(
                chord_used_total,
                chord_bound_total,
                staged_used,
                staged_bound,
                cases = kept.len(),
                elapsed = ?started.elapsed(),
                "stroke chords"
            );
        }
    }
}
