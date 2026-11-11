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

use cherenkov::{Instant, RenderError};
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;
use wgpu::util::DeviceExt;

use crate::render::bindings;
use crate::render::glyph::GpuDraw;
use crate::render::instance::Instance;
use crate::render::path::stroke::{DashTable, GpuStrokeRecord};
use crate::render::path::{
    CoverageKey, GpuBound, GpuElement, GpuPath, InputCounts, InputUnit, STRIP_H,
};
use crate::render::stroke_abi::{
    LIST_WORDS, RECORD_WORDS, SCAN_BLOCK, SLOT_WORDS, SLOTS, STAGE_WORDS,
};
use kurbo::Affine;

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
/// ([`CoverageLimit::admits`]); paths that pass it together go into
/// separate batches, and a forced path that passes it alone is generated
/// in row slices ([`GpuCoverage::draw`]).
const GPU_COV_MAX_BYTES: u64 = 256 << 20;
/// The bytes a frame binds beside its paths' rows: the `Params` blocks,
/// the dispatch arguments and the work header.
const FRAME_FIXED_BYTES: u64 = 18 * 1024 + 128 + 48;

/// u32 words of one geometry record in the arena: the device bounds
/// vec4 plus `(texel offset, interior)`; an all-ones offset denotes a span.
const GEOMETRY_WORDS: u32 = 6;
/// u32 words of one block's header: the emitted instance count, then the
/// first instance and texel words a following row slice starts from, and
/// pad (water-rs/waterui#4108).
const BLOCK_HEADER_WORDS: u32 = 4;
/// The `GpuPath` record word marking a record that continues an entry an
/// earlier batch began; the host reads it, the kernels do not.
const CONTINUES_WORD: usize = 45;
/// Byte offset of a record's `atom[4..7]` — instance count, first
/// instance, texel words — that a continuing record seeds from its
/// block's header.
const SEEDED_COUNTERS: u64 = 16;
/// Capacity of the staging buffer an in-place compaction moves live
/// blocks through, since a buffer copy cannot read and write one buffer
/// (water-rs/waterui#4108). It bounds a compaction's copy phases to the
/// moved bytes over this size, and it is the smallest suballocated
/// block the Vulkan allocation policy reserves
/// ([`crate::interop::memory_hints`]), so the bounce never takes a
/// dedicated device allocation of its own.
const BOUNCE_BYTES: u64 = 4 << 20;

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
    /// This commit's first `paths` record while the entry awaits
    /// generation — the compaction's record fix-up needs it.
    record: u32,
    /// The consecutive records generating the entry, one per row slice.
    slices: u32,
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
/// compaction under the 2× rule (§3.3) — in place while the buffer
/// holds the live set and its headroom, into a larger buffer only when
/// it does not (water-rs/waterui#4108).
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
    /// The staging buffer in-place compactions move blocks through,
    /// created by the first one.
    bounce: Option<wgpu::Buffer>,
}

impl CoverageArena {
    const fn new() -> Self {
        Self {
            buffer: None,
            words: 0,
            tail: 0,
            pending: false,
            generation: 0,
            bounce: None,
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
        self.bounce = None;
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
    cache: super::PipelineCache,
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
    /// The largest buffer the backend's allocator suballocates
    /// ([`crate::interop::suballocation_limit`]), `None` where it takes
    /// no hint. Bounds the ordered upload's staging chunks
    /// ([`Self::staging_chunk`]).
    suballocation: Option<u64>,
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
    /// Each record's range of `input_runs`: the runs of its source's
    /// elements, or of its stroke records, that it reads
    /// (water-rs/waterui#4131).
    record_runs: Vec<Range<usize>>,
    /// The records' input runs, `[first, end)` into their sources.
    input_runs: Vec<[u32; 2]>,
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
    /// The last generating frame's working set as a fresh allocation would
    /// size it, reserved before optional picture admission while released
    /// ([`Self::generation_reservation`]).
    generation_bytes: u64,
    /// The time of the commit in progress, [`GpuCoverage::begin_frame`]'s.
    frame_time: Option<Instant>,
    /// The time of the last commit that needed the generation scratch; the
    /// idle interval is measured from it ([`GpuCoverage::idle_deadline`]).
    scratch_used: Option<Instant>,
}

/// One generation record of a miss: the bound of the rows it generates,
/// the generation-frame clip narrowed to them, and the inputs that reach
/// them ([`GpuCoverage::slices`]). `bound.segments` is the inputs'.
struct Slice {
    bound: GpuBound,
    clip: [f32; 4],
    inputs: InputCounts,
    /// Runs `[first, end)` of the path's elements, or of its stroke
    /// records, each a whole number of [`InputUnit`]s.
    runs: Vec<[u32; 2]>,
}

impl Slice {
    /// The whole of `path` at `bound`, clipped to `clip`.
    fn whole(path: &GpuPath, bound: GpuBound, clip: [f32; 4]) -> Self {
        let inputs = path.inputs();
        let count = u32::try_from(inputs.elements.max(inputs.records)).expect("inputs fit u32");
        Self {
            bound,
            clip,
            inputs,
            runs: (count > 0).then_some([0, count]).into_iter().collect(),
        }
    }
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

    /// The largest generation scratch buffer. The arena is retained
    /// output, not scratch, and is bound separately.
    fn largest_scratch(&self) -> u64 {
        [
            self.elements,
            self.paths,
            self.segments,
            self.scratch,
            self.work,
            self.strokes,
        ]
        .into_iter()
        .max()
        .expect("six buffers")
    }

    /// The largest bound buffer, the retained arena included.
    fn largest(&self) -> u64 {
        self.largest_scratch().max(self.arena)
    }

    /// Whether a batch of these rows can be dispatched: every buffer,
    /// the arena blocks included, within the binding limit and the whole
    /// within the per-batch cap.
    fn fits(&self, limit: CoverageLimit) -> bool {
        self.largest() <= limit.bytes && self.total() + FRAME_FIXED_BYTES <= GPU_COV_MAX_BYTES
    }
}

impl GpuCoverage {
    /// `suballocation` is the backend's
    /// [`crate::interop::suballocation_limit`].
    pub fn new(
        device: &wgpu::Device,
        suballocation: Option<u64>,
        budget: u64,
        cache: super::PipelineCache,
    ) -> Self {
        Self {
            cache,
            ..Self::with_limits(&device.limits(), suballocation, budget)
        }
    }

    /// The size bound of one ordered-upload staging chunk: the allocator's
    /// suballocation block where the backend has one, so a chunk
    /// suballocates from the blocks the allocator retains instead of
    /// taking a dedicated driver allocation every frame
    /// (water-rs/waterui#4174), and never above the binding limit.
    fn staging_chunk(&self) -> u64 {
        self.suballocation
            .map_or(self.limit.bytes, |block| block.min(self.limit.bytes))
    }

    fn with_limits(limits: &wgpu::Limits, suballocation: Option<u64>, budget: u64) -> Self {
        Self {
            cache: super::PipelineCache::default(),
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
            suballocation,
            compaction: None,
            elements: Vec::new(),
            element_sources: Vec::new(),
            strokes: Vec::new(),
            stroke_sources: Vec::new(),
            dash_sources: Vec::new(),
            record_runs: Vec::new(),
            input_runs: Vec::new(),
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
            frame_time: None,
            scratch_used: None,
        }
    }

    /// A frame's apply starts over: the pending records of a planned
    /// frame are committed from scratch (`pending.clear()` precedes
    /// the lowers that produce them, `mod.rs` commit flow). Entries the
    /// previous attempt allocated but never submitted die with it — a
    /// failed or retried commit can never leave a block whose kernels
    /// did not run for a later frame to replay (water-rs/waterui#3081).
    /// The holes they leave are the compaction rule's input.
    pub fn begin_frame(&mut self, now: Instant) {
        self.frame_time = Some(now);
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
        self.record_runs.clear();
        self.input_runs.clear();
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
    /// of scope item 8. While a compaction into a larger buffer is
    /// pending its source arena is still held next to the new one until
    /// the submit lands; an in-place one holds only the bounce buffer.
    pub fn bytes(&self) -> u64 {
        self.uploads.iter().map(wgpu::Buffer::size).sum::<u64>()
            + self.generation_resident_bytes()
            + self.arena.buffer.as_ref().map_or(0, wgpu::Buffer::size)
            + self.arena.bounce.as_ref().map_or(0, wgpu::Buffer::size)
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
        if let Some(copies) = self.in_place_copies() {
            bytes += bounce_size(copies)
                .saturating_sub(self.arena.bounce.as_ref().map_or(0, wgpu::Buffer::size));
        }
        let draws = (self.draws.len() as u64).max(1) * u64::from(ARGS_WORDS) * 4;
        bytes += draws.saturating_sub(self.draws_buffer.as_ref().map_or(0, wgpu::Buffer::size));
        if self.has_misses() {
            // `size_buffers` replaces a retained buffer by the one
            // `grown_size` names, so the frame adds the difference.
            let resident = self.generation_resident_bytes();
            bytes = bytes - resident + self.planned_generation_bytes();
        }
        bytes
    }

    /// The generation scratch `size_buffers` will hold for this frame's
    /// misses: each retained buffer grown by [`grown_size`], the fixed
    /// parameter and argument buffers at their sizes.
    fn planned_generation_bytes(&self) -> u64 {
        let resident = self.buffers.as_ref().map(|b| {
            [
                b.coverage.size(),
                b.elements.size(),
                b.strokes.size(),
                b.paths.size(),
                b.segments.size(),
                b.args.size(),
                b.work.size(),
            ]
        });
        scratch_bytes(
            allocation_sizes(self.generation_sizes()),
            resident,
            self.limit.bytes,
        )
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

    /// The last generating frame's working set not resident now. Retained
    /// scratch is already in [`Self::bytes`], so this is zero until the idle
    /// interval releases it ([`Self::release_idle`]); released scratch will be needed again
    /// when picture misses rasterize, so optional image retention is not
    /// offered its working set.
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
            + self.record_runs.capacity() * std::mem::size_of::<Range<usize>>()
            + self.input_runs.capacity() * std::mem::size_of::<[u32; 2]>()
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
        // The generation scratch outlives the frame that sized it, so the
        // next generating frame reuses it instead of allocating its own
        // (water-rs/waterui#4174): a buffer of a size no frame outgrows is
        // allocated once. A frame that generates nothing neither needs nor
        // releases it: it is released once no frame has needed it for an
        // idle interval ([`Self::release_idle`]), so an application that
        // alternates replaying and generating keeps one set.
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
    /// budget, and pushes the generation records the kernels run on —
    /// one, or one per row slice when the whole path's scratch does not
    /// fit a batch ([`GpuCoverage::slices`]). `retain` marks coverage an
    /// ordinary draw needs past picture generation
    /// ([`GpuCoverage::finish_pictures`]). Nothing is allocated here:
    /// `ensure` creates the planned arena once admission accepted it.
    pub fn draw(&mut self, draw: &GpuDraw, retain: bool) -> Result<u32, RenderError> {
        let &GpuDraw {
            key,
            shift,
            raster,
            clip,
            path: ref prepared,
            ..
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
        let slices = self.slices(prepared, raster, clip)?;
        self.generate(draw, &slices, retain)
    }

    /// Allocate `draw`'s entry, `retain`ed across settles when asked,
    /// and push the generation records of `slices`, which cover its rows
    /// top to bottom, as its draw index.
    fn generate(
        &mut self,
        draw: &GpuDraw,
        slices: &[Slice],
        retain: bool,
    ) -> Result<u32, RenderError> {
        let &GpuDraw {
            key,
            shift,
            rule,
            raster,
            path: ref prepared,
            ..
        } = draw;
        // One block holds every slice's output, each slice's capacity
        // after the previous one's.
        let geometry_slots: usize = slices.iter().map(|slice| slice.bound.instance_cap).sum();
        let texels: usize = slices
            .iter()
            .map(|slice| slice.bound.texels.div_ceil(4) + slice.bound.cell_cap)
            .sum();
        let geometry_slots = u32::try_from(geometry_slots).expect("instance cap fits u32");
        let texels = u32::try_from(texels).expect("texel words fit u32");
        let (base, texel_base, block_words) = self.alloc_block(geometry_slots, texels)?;
        let record = u32::try_from(self.paths.len()).expect("path count fits u32");
        for (index, slice) in slices.iter().enumerate() {
            self.push_slice(
                prepared,
                slice,
                index > 0,
                (rule, raster),
                (base, texel_base),
            );
        }
        self.entries.insert(
            key,
            Entry {
                base,
                texel_base,
                words: block_words,
                geometry_slots,
                record,
                slices: u32::try_from(slices.len()).expect("slice count fits u32"),
                generated: false,
                retained: retain,
                src: None,
                last_used: self.epoch,
            },
        );
        self.generated += 1;
        self.draws.push(Draw { key, shift });
        Ok(u32::try_from(self.draws.len() - 1).expect("draw count fits u32"))
    }

    /// The row slices that generate `prepared`, placed at its bound in a
    /// generation frame whose raster is `raster` and clip `clip`: the
    /// whole path when its rows fit an empty batch
    /// ([`BufferBounds::fits`]), otherwise the fewest consecutive windows
    /// of whole strips, top to bottom, each the largest that fits on its
    /// own (water-rs/waterui#4108). The kernels compute winding along rows
    /// only, so a window is generated exactly by the path's record with
    /// its clip narrowed to the window's rows and its inputs narrowed to
    /// the [`InputUnit`]s reaching them (water-rs/waterui#4131): it
    /// flattens and strokes only those, and bins only the segments
    /// reaching its rows. Its grid is the bbox of those units' segments
    /// within the window, a part of the whole path's.
    ///
    /// A unit is read by every window its rows, widened by a row on each
    /// side for the kernels' f32 evaluation as the row profile's charges
    /// are, reach; the slices together read the path's inputs about once,
    /// a unit across a slice boundary once per slice it reaches. A path
    /// one of whose strips does not fit an empty batch, or whose slices'
    /// blocks together pass the binding limit, refuses the draw before
    /// anything is appended.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "the placed bound's rows are integers, and strip indices are clamped to them"
    )]
    fn slices(
        &self,
        prepared: &GpuPath,
        raster: Affine,
        clip: [f32; 4],
    ) -> Result<Vec<Slice>, RenderError> {
        let bound = prepared.bound;
        if path_rows(prepared, &prepared.inputs(), &bound, FrameTotals::default())
            .is_ok_and(|(rows, _)| rows.fits(self.limit))
        {
            return Ok(vec![Slice::whole(prepared, bound, clip)]);
        }
        let [.., ty] = raster.as_coeffs();
        let strip = i32::try_from(STRIP_H).expect("strip height fits i32");
        let [top, bottom] = [bound.rect[1], bound.rect[3]].map(|row| row as i32);
        let strips = usize::try_from(bottom - top)
            .expect("the bound's rows")
            .div_ceil(STRIP_H);
        // The units reaching the bound's rows, each with the strips
        // `[first, last]` it reaches.
        let strip_of = |row: f64| ((row - f64::from(top)) / STRIP_H as f64).floor();
        let units = move || {
            prepared
                .input_units(raster)
                .filter_map(move |unit: InputUnit| {
                    let (first, last) = (strip_of(unit.top - 1.0), strip_of(unit.bottom + 1.0));
                    (last >= 0.0 && first < strips as f64).then(|| {
                        (
                            first.max(0.0) as usize,
                            (last as usize).min(strips - 1),
                            unit,
                        )
                    })
                })
        };
        // `starts[s]` and `ends[s]` sum the units whose first and last
        // strips lie below `s`: the strips `[from, to)` read
        // `starts[to] - ends[from]`.
        let mut starts = vec![InputCounts::default(); strips + 1];
        let mut ends = starts.clone();
        for (first, last, unit) in units() {
            starts[first + 1].add(&unit.counts);
            ends[last + 1].add(&unit.counts);
        }
        for sums in [&mut starts, &mut ends] {
            for i in 1..sums.len() {
                let previous = sums[i - 1];
                sums[i].add(&previous);
            }
        }
        let window = |from: usize, to: usize| -> Result<Option<(Slice, u64)>, RenderError> {
            let inputs = starts[to].less(&ends[from]);
            let row = |index: usize| top + i32::try_from(index).expect("strips fit i32") * strip;
            let rows = row(from)..row(to).min(bottom);
            let mut slice = prepared.row_slice(&bound, ty, rows);
            slice.segments = inputs.segments;
            let (rows, _) = path_rows(prepared, &inputs, &slice, FrameTotals::default())?;
            let [_, rows_top, _, rows_bottom] = slice.rect;
            Ok(rows.fits(self.limit).then(|| {
                let slice = Slice {
                    bound: slice,
                    clip: [
                        clip[0],
                        clip[1].max(rows_top),
                        clip[2],
                        clip[3].min(rows_bottom),
                    ],
                    inputs,
                    runs: Vec::new(),
                };
                (slice, rows.arena)
            }))
        };
        let mut slices = Vec::new();
        // The slice each strip falls in.
        let mut slice_of = vec![0; strips];
        let mut arena = 0;
        let mut from = 0;
        while from < strips {
            let mut to = from + 1;
            let Some(mut fitting) = window(from, to)? else {
                // A single strip's inputs and scratch already pass the
                // binding limit, so every window would too, and the whole
                // path passes it.
                return Err(RenderError::Unsupported(
                    "GPU coverage input exceeds device buffer limit",
                ));
            };
            while to < strips {
                let Some(larger) = window(from, to + 1)? else {
                    break;
                };
                (fitting, to) = (larger, to + 1);
            }
            arena += fitting.1;
            if arena > self.limit.bytes {
                return Err(RenderError::Unsupported(
                    "GPU coverage output exceeds device buffer limit",
                ));
            }
            slice_of[from..to].fill(slices.len());
            slices.push(fitting.0);
            from = to;
        }
        // Each unit, in input order, joins the runs of the slices it
        // reaches.
        for (first, last, unit) in units() {
            for slice in &mut slices[slice_of[first]..=slice_of[last]] {
                match slice.runs.last_mut() {
                    Some(run) if run[1] == unit.inputs[0] => run[1] = unit.inputs[1],
                    _ => slice.runs.push(unit.inputs),
                }
            }
        }
        Ok(slices)
    }

    /// Make the current batch the one a slice reading `inputs` at `bound`
    /// joins — a fresh one when it `continues` a path or does not fit the
    /// current one — add its rows to the batch's bound, and return the
    /// batch's totals with it.
    fn place(
        &mut self,
        prepared: &GpuPath,
        inputs: &InputCounts,
        bound: &GpuBound,
        continues: bool,
    ) -> FrameTotals {
        let empty = |paths: usize| Batch {
            start: paths,
            end: paths,
            ..Batch::default()
        };
        if continues {
            let next = empty(self.paths.len());
            self.batches.push(std::mem::replace(&mut self.batch, next));
        }
        let append = |batch: &Batch| {
            let (rows, totals) = path_rows(prepared, inputs, bound, batch.totals).ok()?;
            let mut next = batch.bound;
            next.add(&rows);
            next.fits(self.limit).then_some((next, totals))
        };
        let (next, totals) = append(&self.batch).unwrap_or_else(|| {
            let fresh = empty(self.paths.len());
            let fits = append(&fresh).expect("a slice fits an empty batch under the binding limit");
            self.batches.push(std::mem::replace(&mut self.batch, fresh));
            fits
        });
        self.batch.bound = next;
        totals
    }

    /// Append one slice's generation record to the frame's batches. A
    /// continuing slice — any after an entry's first — starts a batch of
    /// its own, since it reads the counters its predecessor's batch left
    /// in the block header; every other slice joins the current batch
    /// when it fits there. [`GpuCoverage::slices`] checked that each fits
    /// an empty batch.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "device coordinates are f32"
    )]
    fn push_slice(
        &mut self,
        prepared: &GpuPath,
        slice: &Slice,
        continues: bool,
        (rule, raster): (u32, Affine),
        (base, texel_base): (u32, u32),
    ) {
        let GpuPath {
            elements,
            strokes,
            stroke,
            dash,
            ..
        } = prepared;
        let Slice {
            ref bound,
            clip,
            ref inputs,
            ref runs,
        } = *slice;
        let totals = self.place(prepared, inputs, bound, continues);
        let elem_base = self.batch.element_total;
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
        words[9] = u32::try_from(inputs.elements).expect("elements fit u32");
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
        words[CONTINUES_WORD] = u32::from(continues);

        self.paths.push(words);
        self.batch.end = self.paths.len();
        self.batch.element_total += u32::try_from(inputs.elements).expect("elements fit u32");
        self.element_sources.push(Arc::clone(elements));
        let records = u32::try_from(inputs.records).expect("stroke records fit u32");
        // The R2 cap above bounds the records far below `u32::MAX`.
        self.batch.stroke_total += records;
        self.stroke_sources.push(Arc::clone(strokes));
        if let Some(dash) = dash {
            self.batch.dash_total += records;
            self.batch.stage_total += u32::try_from(inputs.stage).expect("staging fits u32");
            self.batch.list_total +=
                u32::try_from(inputs.dash_fragments()).expect("fragments fit u32");
            self.batch.dash_table_words +=
                u32::try_from(dash.cum.len() * 2).expect("dash table fits u32");
        }
        self.dash_sources.push(dash.clone());
        let first_run = self.input_runs.len();
        self.input_runs.extend_from_slice(runs);
        self.record_runs.push(first_run..self.input_runs.len());
        self.batch.seg_total += u32::try_from(bound.segments).expect("segments");
        self.batch.bin_total += u32::try_from(bound.bins).expect("bins");
        self.batch.totals = totals;
        self.batch.row_total += u32::try_from(bound.grid_h).expect("rows");
    }

    /// Settle the retained set once per commit, after the frame's
    /// entries were touched and before any miss allocates (§3.3).
    /// Retained-but-undrawn entries pay for themselves: they are evicted
    /// LRU-first until they fit the budget again. Entries this frame
    /// drew — `last_used` at the current epoch — are never touched. When
    /// the frame draws coverage and the arena's frontier then exceeds
    /// twice the live words, the live blocks are repacked to the front,
    /// whether or not any draw misses. A frame without GPU
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
        // A block that does not fit at the tail repacks the live blocks
        // with room for it, growing the buffer only when they do not
        // fit it (§3.3).
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

    /// Plan a repack of every live entry to the front of the arena,
    /// which must then hold the live words plus this frame's immediate
    /// need with a quarter of that as headroom (§3.3). The current
    /// buffer is reused whenever it holds that much; only a working set
    /// that outgrows it plans a replacement of exactly that size, so a
    /// scene whose live set churns at a steady size repacks in place and
    /// allocates nothing (water-rs/waterui#4108). A replacement is only
    /// planned: `ensure` creates it once, after layout planning and
    /// admission, however many repacks the commit ran. Generated entries
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
        let words = (need + u64::from(extra).max(need / 4))
            .min(self.limit.words())
            .max(16);
        // Generated content's source: an unrecorded compaction's buffer
        // still holds it, otherwise the live arena does.
        let source = self
            .compaction
            .as_ref()
            .map(|compaction| compaction.source.clone())
            .or_else(|| self.arena.buffer.clone());
        // `words` is the capacity the arena will have at `ensure`: the
        // resident buffer's, or an earlier repack's planned replacement,
        // and zero with neither. A planned replacement too small for this
        // repack is resized, never allocated twice.
        if u64::from(self.arena.words) < words {
            self.arena.words = u32::try_from(words).expect("arena words fit u32");
            self.arena.pending = true;
        }
        // Content repacked within the buffer it already lives in moves
        // only where its block shifts; a planned replacement receives
        // every generated block.
        let in_place = !self.arena.pending && source.as_ref() == self.arena.buffer.as_ref();
        let mut copies = Vec::new();
        for (key, new_base) in moves {
            let entry = self.entries.get_mut(&key).expect("compaction entry");
            if entry.generated {
                let src = entry.src.unwrap_or(entry.base);
                if !(in_place && src == new_base) {
                    copies.push((
                        u64::from(src) * 4,
                        u64::from(new_base) * 4,
                        u64::from(entry.words) * 4,
                    ));
                }
                entry.src = Some(src);
            } else {
                let records = entry.record as usize..(entry.record + entry.slices) as usize;
                for record in &mut self.paths[records] {
                    record[14] = new_base + BLOCK_HEADER_WORDS;
                    record[13] =
                        new_base + BLOCK_HEADER_WORDS + entry.geometry_slots * GEOMETRY_WORDS;
                }
            }
            entry.texel_base =
                new_base + BLOCK_HEADER_WORDS + entry.geometry_slots * GEOMETRY_WORDS;
            entry.base = new_base;
        }
        self.compaction = source
            .filter(|_| !copies.is_empty())
            .map(|source| Compaction { source, copies });
        self.arena.tail = tail;
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
        if let Some(copies) = self.in_place_copies() {
            let size = bounce_size(copies);
            self.arena.bounce = Some(sized_buffer(
                device,
                self.arena.bounce.as_ref(),
                "coverage bounce",
                size,
                wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            ));
        }
        if self.has_misses() {
            self.size_buffers(device, self.generation_sizes())?;
        }
        Ok(())
    }

    /// The pending compaction's copies when it repacks the arena within
    /// its resident buffer, which they reach through the bounce buffer.
    fn in_place_copies(&self) -> Option<&[(u64, u64, u64)]> {
        self.compaction
            .as_ref()
            .filter(|compaction| {
                !self.arena.pending && self.arena.buffer.as_ref() == Some(&compaction.source)
            })
            .map(|compaction| compaction.copies.as_slice())
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

    /// When the generation scratch is due for release: the idle `interval`
    /// after the last commit that needed it, `None` while none is held (or
    /// when the interval never elapses).
    pub fn idle_deadline(&self, interval: Duration) -> Option<Instant> {
        self.buffers.as_ref()?;
        self.scratch_used?.checked_add(interval)
    }

    /// Releases the generation scratch when no commit has needed it for
    /// `interval` as of `now`, returning whether it did. The retained arena
    /// is output, not scratch, and stays.
    pub fn release_idle(&mut self, now: Instant, interval: Duration) -> bool {
        let due = self.idle_deadline(interval).is_some_and(|due| now >= due);
        if due {
            self.buffers = None;
            self.scratch_used = None;
        }
        due
    }

    pub fn trim(&mut self) {
        self.generation_bytes = 0;
        self.entries.clear();
        self.compaction = None;
        self.live_words = 0;
        self.undrawn_bytes = 0;
        self.buffers = None;
        self.scratch_used = None;
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
            if compaction.source == arena {
                let bounce = self.arena.bounce.as_ref().expect("ensure sized the bounce");
                record_in_place(encoder, &arena, &compaction.copies, bounce);
            } else {
                for &(src, dst, len) in &compaction.copies {
                    encoder.copy_buffer_to_buffer(&compaction.source, src, &arena, dst, len);
                }
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
                // The first batch's pass opens the timing and the last one
                // closes it; a batch between them writes no timestamp, and
                // a pass given timestamp writes must carry an index.
                let timestamps = timestamp_writes.and_then(|(query_set, beginning, end)| {
                    let beginning = beginning.filter(|_| index == 0);
                    let end = end.filter(|_| index == self.batches.len());
                    (beginning.is_some() || end.is_some()).then_some(
                        wgpu::ComputePassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: beginning,
                            end_of_pass_write_index: end,
                        },
                    )
                });
                self.dispatch(device, batch, encoder, delivery, timestamps);
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
        // Batching holds every scratch buffer to the binding limit, the
        // device's hard bound. A buffer retained from an earlier frame is
        // kept while it is large enough, so only a frame that needs more
        // than any before it allocates.
        if sizes.into_iter().any(|size| size > self.limit.bytes) {
            return Err(RenderError::Unsupported(
                "GPU coverage input exceeds device buffer limit",
            ));
        }
        self.scratch_used = self.frame_time;
        let sizes = allocation_sizes(sizes);
        let previous = self.buffers.as_ref();
        let limit = self.limit.bytes;
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
            elements: grown_buffer(
                device,
                previous.map(|b| &b.elements),
                "cov elements",
                (sizes[1], limit),
                storage,
            ),
            paths: grown_buffer(
                device,
                previous.map(|b| &b.paths),
                "cov paths",
                (sizes[3], limit),
                storage,
            ),
            segments: grown_buffer(
                device,
                previous.map(|b| &b.segments),
                "cov segments",
                (sizes[4], limit),
                storage,
            ),
            coverage: grown_buffer(
                device,
                previous.map(|b| &b.coverage),
                "cov scratch",
                (sizes[0], limit),
                storage,
            ),
            strokes: grown_buffer(
                device,
                previous.map(|b| &b.strokes),
                "cov strokes",
                (sizes[2], limit),
                storage,
            ),
            args: sized_buffer(
                device,
                previous.map(|b| &b.args),
                "cov args",
                DISPATCH_ARGS_WORDS * 4,
                storage | wgpu::BufferUsages::INDIRECT,
            ),
            work: grown_buffer(
                device,
                previous.map(|b| &b.work),
                "cov work",
                (sizes[6], limit),
                storage,
            ),
        });
        // What a fresh allocation of this working set takes once it is
        // released, not the grown buffers' slack.
        self.generation_bytes = scratch_bytes(sizes, None, limit);

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

        let mut upload = BatchUpload::new(self.staging_chunk());
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
        // Each record reads the runs of its source's inputs that reach its
        // rows, renumbered as one range (water-rs/waterui#4131): a slot or
        // staging base (`head.w`) counts on from the inputs before it by
        // their counts (`head.y`), as the encoders assigned them over the
        // whole path, and a stroke record's vertex links and subpath
        // bounds move with its subpath, which a run holds whole.
        for (index, runs) in self.record_runs[batch.start..batch.end].iter().enumerate() {
            let source = &self.element_sources[batch.start + index];
            if source.is_empty() {
                continue;
            }
            let path_i = u32::try_from(index).expect("path index");
            let mut slot = 0;
            for &[first, end] in &self.input_runs[runs.clone()] {
                self.elements
                    .extend(source[first as usize..end as usize].iter().map(|&element| {
                        let mut element = element;
                        element.head[2] = path_i;
                        element.head[3] = slot;
                        slot += element.head[1];
                        element
                    }));
            }
        }
        upload.write_buffer(&buffers.elements, 0, bytemuck::cast_slice(&self.elements));
        // Undashed paths' records first, dashed ones after (7.4); the dash
        // tables follow in apply order, where `draw` assigned their offsets.
        for dashed in [false, true] {
            for (index, runs) in self.record_runs[batch.start..batch.end].iter().enumerate() {
                let source = &self.stroke_sources[batch.start + index];
                if source.is_empty() || self.dash_sources[batch.start + index].is_some() != dashed {
                    continue;
                }
                let path_i = u32::try_from(index).expect("path index");
                let path_first = self.strokes.len();
                self.paths[batch.start + index][36] =
                    u32::try_from(path_first).expect("stroke records fit u32");
                let mut slot = 0;
                for &[first, end] in &self.input_runs[runs.clone()] {
                    let at = u32::try_from(self.strokes.len() - path_first)
                        .expect("stroke records fit u32");
                    let relink = |link: u32| {
                        if link == u32::MAX {
                            link
                        } else {
                            link - first * SLOTS + at * SLOTS
                        }
                    };
                    self.strokes
                        .extend(source[first as usize..end as usize].iter().map(|&record| {
                            let mut record = record;
                            record.head[2] = path_i;
                            record.head[3] = slot;
                            slot += record.head[1];
                            record.link = record.link.map(relink);
                            if dashed {
                                record.aux[2] = record.aux[2] - first + at;
                                record.aux[3] = record.aux[3] - first + at;
                            }
                            record
                        }));
                }
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
        self.uploads.extend(upload.finish(device, encoder));
        encoder.clear_buffer(&buffers.work, 0, Some(work_header_bytes));
        let arena = self.arena.buffer().expect("coverage arena sized");
        // A continuing slice's counters start where its predecessor's
        // `emit_counts`, in an earlier batch of this encoder, left them in
        // the block header (water-rs/waterui#4108).
        for (index, path) in self.paths[batch.start..batch.end].iter().enumerate() {
            if path[CONTINUES_WORD] != 0 {
                encoder.copy_buffer_to_buffer(
                    arena,
                    u64::from(path[14] - BLOCK_HEADER_WORDS) * 4,
                    &buffers.paths,
                    (index * PATH_WORDS * 4) as u64 + SEEDED_COUNTERS,
                    12,
                );
            }
        }

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
                cache: self.cache.handle.as_ref(),
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
        self.cache.persist();
    }
}

/// A batch's uploads, recorded as copies between its compute passes, since
/// batches share their destinations. The bytes are staged in chunks of at
/// most `chunk` bytes ([`GpuCoverage::staging_chunk`]: the allocator's
/// suballocation block, within the binding limit), a write that does not
/// fit the current chunk filling it and continuing in the next, so every
/// staging buffer suballocates from retained device memory instead of
/// being a dedicated allocation each frame (water-rs/waterui#4174).
struct BatchUpload {
    chunk: u64,
    chunks: Vec<Vec<u8>>,
    /// `(target, target offset, chunk, chunk offset, size)`.
    copies: Vec<(wgpu::Buffer, u64, usize, u64, u64)>,
}

impl BatchUpload {
    const fn new(chunk: u64) -> Self {
        Self {
            chunk,
            chunks: Vec::new(),
            copies: Vec::new(),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "a chunk's room is below the chunk size, which is a buffer size"
    )]
    fn write_buffer(&mut self, target: &wgpu::Buffer, mut offset: u64, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self
                .chunks
                .last()
                .is_none_or(|last| last.len() as u64 >= self.chunk)
            {
                self.chunks.push(Vec::new());
            }
            let index = self.chunks.len() - 1;
            let staged = &mut self.chunks[index];
            let room = (self.chunk - staged.len() as u64) as usize;
            let (piece, rest) = bytes.split_at(room.min(bytes.len()));
            let source_offset = staged.len() as u64;
            staged.extend_from_slice(piece);
            self.copies.push((
                target.clone(),
                offset,
                index,
                source_offset,
                piece.len() as u64,
            ));
            offset += piece.len() as u64;
            bytes = rest;
        }
    }

    fn finish(
        self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Vec<wgpu::Buffer> {
        let sources: Vec<_> = self
            .chunks
            .iter()
            .map(|bytes| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("coverage batch upload"),
                    contents: bytes,
                    usage: wgpu::BufferUsages::COPY_SRC,
                })
            })
            .collect();
        for (target, offset, chunk, source_offset, size) in self.copies {
            encoder.copy_buffer_to_buffer(&sources[chunk], source_offset, &target, offset, size);
        }
        sources
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

/// The device's coverage buffer limits. Every buffer the coverage
/// kernels bind, the retained arena and the generation scratch alike,
/// must fit the binding limit,
/// `max_storage_buffer_binding_size.min(max_buffer_size)`. The scratch
/// buffers are retained across the frames that generate coverage
/// ([`GpuCoverage::submitted`]), so their size is no per-frame
/// allocation (water-rs/waterui#4174).
#[derive(Clone, Copy, Debug)]
pub struct CoverageLimit {
    /// The binding limit.
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
    /// the CPU sweep. The rows are the whole path's, from the same
    /// [`path_rows`] [`GpuCoverage::draw`] sizes from. A path the
    /// route does not admit but that is forced onto the GPU is generated
    /// in row slices that each fit (water-rs/waterui#4108).
    pub fn admits(self, path: &GpuPath, bound: &GpuBound) -> bool {
        path_rows(path, &path.inputs(), bound, FrameTotals::default()).is_ok_and(|(rows, _)| {
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

/// The rows the draw of `path` at `bound`, reading `inputs` of it, adds
/// to decision R2's bound of a frame whose earlier misses reached
/// `before`, by buffer, and the totals after it. [`GpuCoverage::draw`]
/// and [`CoverageLimit::admits`] both size a path from here, so the route
/// predicate and the frame's check cannot disagree on what one path
/// costs; a row slice reads its own inputs, whose segments are
/// `bound.segments`.
fn path_rows(
    path: &GpuPath,
    inputs: &InputCounts,
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
        elements: to64(inputs.elements) * (std::mem::size_of::<GpuElement>() as u64),
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
    bounds.add(&stroke_bounds(inputs, path.dash.as_deref()));
    Ok((bounds, after))
}

/// The bytes the stroke records of `inputs` bound (design section 8), by
/// buffer: the records, and for a dashed path its cumulative-length table,
/// in the strokes buffer; the vertex scratch, and for a dashed path the
/// per-record dash words, the staging and list entries and the scan
/// blocks, in the coverage scratch.
fn stroke_bounds(inputs: &InputCounts, dash: Option<&DashTable>) -> BufferBounds {
    let to64 = |n: usize| u64::try_from(n).expect("bound fits u64");
    let records = inputs.records;
    let mut bounds = BufferBounds {
        strokes: to64(records) * std::mem::size_of::<GpuStrokeRecord>() as u64,
        scratch: to64(records) * u64::from(SLOTS * SLOT_WORDS) * 4,
        ..BufferBounds::default()
    };
    if let Some(dash) = dash {
        bounds.strokes += to64(dash.cum.len()) * 8;
        bounds.scratch += to64(records) * u64::from(DASH_RECORD_WORDS) * 4
            + to64(inputs.stage) * u64::from(STAGE_WORDS) * 4
            + to64(inputs.dash_fragments()) * u64::from(LIST_WORDS) * 4
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

/// The bounce buffer size an in-place compaction moving `copies` needs:
/// its moved bytes, at most one [`BOUNCE_BYTES`] phase.
fn bounce_size(copies: &[(u64, u64, u64)]) -> u64 {
    copies
        .iter()
        .map(|&(.., len)| len)
        .sum::<u64>()
        .min(BOUNCE_BYTES)
}

/// Record an in-place compaction's copies, `(src, dst, len)` byte
/// ranges of `arena`, through the `staging` bounce buffer `ensure`
/// sized, since a copy cannot read and write one buffer. A repack keeps
/// block order, so every destination lies at or below its source:
/// moving the blocks in ascending order, one bounce-sized phase at a
/// time, never overwrites content a later phase still reads
/// (water-rs/waterui#4108).
fn record_in_place(
    encoder: &mut wgpu::CommandEncoder,
    arena: &wgpu::Buffer,
    copies: &[(u64, u64, u64)],
    staging: &wgpu::Buffer,
) {
    let mut ascending = copies.to_vec();
    ascending.sort_unstable_by_key(|&(src, ..)| src);
    // Neighbouring blocks that shift by the same distance move as one run.
    let mut runs: Vec<(u64, u64, u64)> = Vec::with_capacity(ascending.len());
    for (src, dst, len) in ascending {
        assert!(
            dst < src,
            "an in-place repack moves blocks toward the front"
        );
        match runs.last_mut() {
            Some(run) if run.0 + run.2 == src && run.1 + run.2 == dst => run.2 += len,
            _ => runs.push((src, dst, len)),
        }
    }
    let capacity = staging.size();
    // The current phase's `(dst, bounce offset, len)` write-backs, recorded
    // once every read of the phase is.
    let mut phase: Vec<(u64, u64, u64)> = Vec::new();
    let write_back = |encoder: &mut wgpu::CommandEncoder, phase: &mut Vec<(u64, u64, u64)>| {
        for &(dst, offset, len) in phase.iter() {
            encoder.copy_buffer_to_buffer(staging, offset, arena, dst, len);
        }
        phase.clear();
    };
    let mut used = 0;
    for (mut src, mut dst, mut len) in runs {
        while len > 0 {
            let take = len.min(capacity - used);
            encoder.copy_buffer_to_buffer(arena, src, staging, used, take);
            phase.push((dst, used, take));
            used += take;
            src += take;
            dst += take;
            len -= take;
            if used == capacity {
                write_back(encoder, &mut phase);
                used = 0;
            }
        }
    }
    write_back(encoder, &mut phase);
}

/// A retained generation scratch buffer of at least `size` bytes: the
/// `previous` one while it is large enough, otherwise a new one that grows
/// at least twofold over it, within `limit`, so a working set creeping up
/// frame by frame reallocates a logarithmic number of times rather than
/// once per frame (water-rs/waterui#4174).
fn grown_buffer(
    device: &wgpu::Device,
    previous: Option<&wgpu::Buffer>,
    label: &'static str,
    (size, limit): (u64, u64),
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let grown = grown_size(previous.map(wgpu::Buffer::size), size, limit);
    sized_buffer(
        device,
        previous.filter(|buffer| buffer.size() == grown),
        label,
        grown,
        usage,
    )
}

/// The size of the scratch buffer [`grown_buffer`] holds for `size` bytes
/// over a `previous` buffer of that size: the previous one while it is
/// large enough, otherwise twice it within `limit` and at least `size`.
/// Admission ([`GpuCoverage::planned_bytes`]) plans with the same rule.
fn grown_size(previous: Option<u64>, size: u64, limit: u64) -> u64 {
    match previous {
        Some(previous) if previous >= size => previous,
        Some(previous) => (previous * 2).min(limit).max(size),
        None => size,
    }
}

/// The generation scratch bytes held for the per-buffer `sizes` (already
/// through [`allocation_sizes`]) over the `resident` buffers' sizes, in
/// the same order: each grown by [`grown_size`], plus the parameter
/// blocks. `sizes[5]`, the dispatch arguments, is fixed, so it never grows.
fn scratch_bytes(sizes: [u64; 7], resident: Option<[u64; 7]>, limit: u64) -> u64 {
    sizes
        .into_iter()
        .enumerate()
        .map(|(index, size)| grown_size(resident.map(|r| r[index]), size, limit))
        .sum::<u64>()
        + PARAMS_BLOCKS * u64::from(PARAMS_STRIDE)
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

    /// A coverage frame on `device` with an undrawn budget that never
    /// evicts.
    fn uncapped(device: &wgpu::Device) -> GpuCoverage {
        GpuCoverage::new(
            device,
            None,
            u64::MAX,
            super::super::PipelineCache::default(),
        )
    }

    #[test]
    fn coverage_pipelines_follow_later_stroke_and_dash_use() {
        let Some((device, _, delivery)) = delivery_device() else {
            return;
        };
        let mut coverage = uncapped(&device);
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
        let mut frame = uncapped(&device);
        frame.begin_frame(Instant::now());
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
        let mut frame = uncapped(&device);
        frame.begin_frame(Instant::now());
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
        let mut probe = uncapped(&device);
        probe.begin_frame(Instant::now());
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
        let mut frame = GpuCoverage::with_limits(&small, None, u64::MAX);
        frame.begin_frame(Instant::now());
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
        let mut frame = GpuCoverage::with_limits(&between, None, u64::MAX);
        frame.begin_frame(Instant::now());
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
        reason = "one scenario: both roads' frame, its batches, the replay read back and the commit lifecycle"
    )]
    fn road_paths_share_bounded_scratch() {
        let Some((device, queue, delivery)) = delivery_device() else {
            return;
        };
        let draws: Vec<_> = road_paths()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                GpuDraw::new(index as u64 + 1, 0, Affine::IDENTITY, path, (1024, 768))
            })
            .collect();
        let [first, second] = &draws[..] else {
            panic!("two road layers")
        };
        let mut frame = uncapped(&device);
        let output_bytes: u64 = draws
            .iter()
            .map(|draw| {
                assert!(frame.limit.admits(&draw.path, &draw.path.bound));
                path_rows(
                    &draw.path,
                    &draw.path.inputs(),
                    &draw.path.bound,
                    FrameTotals::default(),
                )
                .expect("path rows")
                .0
                .arena
            })
            .sum();
        assert!(
            output_bytes + 64 < frame.limit.bytes,
            "both retained outputs fit"
        );
        frame.begin_frame(Instant::now());
        frame.draw(first, false).expect("first road fits");
        frame
            .draw(second, true)
            .expect("individually admitted roads share bounded scratch");
        assert_eq!(frame.batches.len(), 1, "two dispatch batches");
        frame.begin_frame(Instant::now());
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
        assert!(
            frame.buffers.is_some(),
            "a generating frame retains its scratch"
        );
        assert_eq!(
            frame.generation_reservation(),
            0,
            "retained scratch is not reserved twice"
        );
        assert_eq!(frame.uploads.len(), 0);
        frame.begin_frame(Instant::now());
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
            frame.buffers.is_some(),
            "a frame that generates nothing leaves release to the idle interval"
        );
        let interval = Duration::from_secs(1);
        frame.release_idle(Instant::now() + interval, interval);
        assert!(frame.buffers.is_none(), "idle scratch is released");
        assert_eq!(
            frame.generation_reservation(),
            generation_bytes,
            "released scratch stays reserved for the next generation"
        );
        frame.finish_pictures();
        assert!(!frame.entries.contains_key(&first.key));
        frame.begin_frame(Instant::now());
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

    /// Uploads stage within the chunk bound — the allocator's
    /// suballocation block under the binding limit — a write larger than
    /// the room left continuing in the next chunk (water-rs/waterui#4174).
    #[test]
    fn upload_chunks_stay_within_the_suballocation_block() {
        let Some((device, _, _)) = delivery_device() else {
            return;
        };
        let mut frame = uncapped(&device);
        frame.limit.bytes = 64;
        assert_eq!(frame.staging_chunk(), 64, "no block: the binding limit");
        frame.suballocation = Some(1024);
        assert_eq!(frame.staging_chunk(), 64, "never above the binding limit");
        frame.limit.bytes = 4096;
        assert_eq!(frame.staging_chunk(), 1024, "the block bounds a chunk");
        let chunk = frame.staging_chunk();
        let target = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload target"),
            size: 4096,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut upload = BatchUpload::new(chunk);
        for size in [600, 600, 300, 3000, 100] {
            upload.write_buffer(&target, 0, &vec![0; size]);
        }
        let sizes: Vec<_> = upload.chunks.iter().map(Vec::len).collect();
        assert_eq!(sizes, [1024, 1024, 1024, 1024, 504]);
        let staged: u64 = upload.copies.iter().map(|copy| copy.4).sum();
        assert_eq!(staged, 4600, "every byte is copied once");
    }

    /// The binding limit the sliced frames below run under: below the
    /// scratch of the largest road, above any one strip's.
    const SLICING_LIMIT: u64 = 16 << 20;

    /// A path whose scratch passes the binding limit is generated in row
    /// slices across batches whose every scratch buffer and upload
    /// staging chunk stays within it (water-rs/waterui#4108); the slices
    /// compose into the one entry the draw replays, and its coverage
    /// matches the unsliced coverage of a device whose limit holds the
    /// whole road.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one scenario: every bound the sliced frame holds, then its coverage against the unsliced frame"
    )]
    fn a_road_over_the_binding_limit_is_sliced_to_the_unsliced_coverage() {
        // The per-batch sizes below are the evidence for the slicing; they
        // must be visible on a plain `--no-capture` run.
        let _tracing = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_test_writer()
                .with_max_level(tracing::level_filters::LevelFilter::DEBUG)
                .finish(),
        );
        let Some((device, queue, delivery)) = delivery_device() else {
            return;
        };
        let block = SLICING_LIMIT;
        let paths = road_paths();
        let draws: Vec<_> = (0..2 * paths.len())
            .map(|index| {
                let path = paths[index % paths.len()].clone();
                GpuDraw::new(index as u64 + 1, 0, Affine::IDENTITY, path, (1024, 768))
            })
            .collect();
        let mut frame = uncapped(&device);
        for draw in &draws {
            assert!(frame.limit.admits(&draw.path, &draw.path.bound));
        }
        assert!(
            paths.iter().any(|path| {
                path_rows(path, &path.inputs(), &path.bound, FrameTotals::default())
                    .expect("road rows")
                    .0
                    .largest_scratch()
                    > block
            }),
            "a road's scratch outgrows the limit"
        );
        frame.limit.bytes = block;
        frame.begin_frame(Instant::now());
        for draw in &draws {
            frame.draw(draw, true).expect("road slices fit the limit");
        }
        let slices: Vec<u32> = draws
            .iter()
            .map(|draw| frame.entries[&draw.key].slices)
            .collect();
        assert!(slices.iter().any(|&n| n > 1), "a road is sliced");
        for (index, &n) in slices.iter().enumerate() {
            assert_eq!(n, slices[index % paths.len()], "same road, same slices");
        }
        for batch in frame.batches.iter().chain([&frame.batch]) {
            let bound = batch.bound;
            assert!(bound.largest_scratch() <= block, "bound within limit");
            tracing::debug!(
                records = batch.end - batch.start,
                elements = bound.elements,
                segments = bound.segments,
                scratch = bound.scratch,
                strokes = bound.strokes,
                work = bound.work,
                arena = bound.arena,
                "sliced batch"
            );
        }
        tracing::debug!(
            ?slices,
            batches = frame.batches.len() + 1,
            block,
            "road slices"
        );
        for draw in &draws[..paths.len()] {
            for slice in frame
                .slices(&draw.path, draw.raster, draw.clip)
                .expect("road slices")
            {
                assert_slice_fits_its_bound(draw, &slice, (&device, &queue, delivery));
            }
        }
        let (in_slices, staged) =
            coverage_blocks(&mut frame, (&device, &queue, delivery), &draws, block);
        // Each slice stages only the inputs reaching its rows, so the frame
        // stages every road's inputs about once — a subpath across a slice
        // boundary once per slice it reaches — beside each batch's own
        // parameters, path records, work items and dash tables, never
        // once per slice (water-rs/waterui#4131).
        let inputs: u64 = draws
            .iter()
            .map(|draw| {
                let path = &draw.path;
                (path.elements.len() * std::mem::size_of::<GpuElement>()
                    + path.strokes.len() * std::mem::size_of::<GpuStrokeRecord>()
                    + path.dash.as_deref().map_or(0, |dash| dash.cum.len() * 8))
                    as u64
            })
            .sum();
        let per_batch: u64 = frame
            .batches
            .iter()
            .chain([&frame.batch])
            .map(|&batch| {
                let BatchPlan { sizes, .. } = BatchPlan::new(batch);
                PARAMS_BLOCKS * u64::from(PARAMS_STRIDE)
                    + sizes[3]
                    + sizes[6]
                    + u64::from(batch.dash_table_words) * 4
            })
            .sum();
        tracing::debug!(staged, inputs, per_batch, "sliced frame upload");
        assert!(
            staged <= inputs + inputs / 4 + per_batch,
            "a sliced frame staged {staged} bytes for {inputs} bytes of inputs and \
             {per_batch} of per-batch data"
        );
        let mut whole = uncapped(&device);
        whole.begin_frame(Instant::now());
        for draw in &draws {
            whole.draw(draw, true).expect("road fits uncapped");
            assert_eq!(whole.entries[&draw.key].slices, 1, "uncapped is unsliced");
        }
        let (at_once, _) =
            coverage_blocks(&mut whole, (&device, &queue, delivery), &draws, u64::MAX);
        for (sliced, unsliced) in in_slices.iter().zip(&at_once) {
            cherenkov_oracle::metrics::assert_images_match(unsliced, sliced);
        }
    }

    /// A path one of whose strips alone passes the binding limit cannot
    /// be sliced under it, and no batch holds it whole either: the draw
    /// is refused before anything is appended. The limit here is one
    /// stroke record, less than any strip's path record alone.
    #[test]
    fn a_path_with_a_strip_over_the_binding_limit_is_refused() {
        let Some((device, _, _)) = delivery_device() else {
            return;
        };
        let mut frame = uncapped(&device);
        frame.limit.bytes = std::mem::size_of::<GpuStrokeRecord>() as u64;
        frame.begin_frame(Instant::now());
        for (index, path) in road_paths().into_iter().enumerate() {
            let draw = GpuDraw::new(index as u64 + 1, 0, Affine::IDENTITY, path, (1024, 768));
            assert!(frame.draw(&draw, true).is_err(), "an unfittable road");
        }
        assert_eq!(frame.paths.len(), 0, "refused before appending a path");
    }

    /// Handles to the retained scratch buffers, compared by identity.
    fn scratch(frame: &GpuCoverage) -> [wgpu::Buffer; 7] {
        let buffers = frame.buffers.as_ref().expect("scratch retained");
        [
            buffers.elements.clone(),
            buffers.paths.clone(),
            buffers.segments.clone(),
            buffers.coverage.clone(),
            buffers.strokes.clone(),
            buffers.args.clone(),
            buffers.work.clone(),
        ]
    }

    /// Admitted roads are never sliced: they run in the batches the
    /// binding limit and the per-batch cap alone require, where the
    /// allocator's block size once split them. The frame's scratch
    /// outlives its submit: a later generating frame
    /// of the same size reuses the very buffers, one that needs more
    /// replaces them, and they are released once no frame has needed
    /// them for the idle interval (water-rs/waterui#4174, #4193).
    #[test]
    fn generation_scratch_is_released_after_the_idle_interval() {
        let Some((device, queue, delivery)) = delivery_device() else {
            return;
        };
        let gpu = (&device, &queue, delivery);
        let draws: Vec<_> = road_paths()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                GpuDraw::new(index as u64 + 1, 0, Affine::IDENTITY, path, (1024, 768))
            })
            .collect();
        let moved = |draw: &GpuDraw, content| {
            GpuDraw::new(content, 0, Affine::IDENTITY, draw.path.clone(), (1024, 768))
        };
        let mut frame = uncapped(&device);
        // The smaller road first, so the larger one outgrows its scratch.
        let size = |draw: &GpuDraw| {
            path_rows(
                &draw.path,
                &draw.path.inputs(),
                &draw.path.bound,
                FrameTotals::default(),
            )
            .expect("road rows")
            .0
            .total()
        };
        let (small, large) = if size(&draws[0]) < size(&draws[1]) {
            (&draws[0], &draws[1])
        } else {
            (&draws[1], &draws[0])
        };
        // The clock is injected: each frame starts a `step` after the last.
        let interval = Duration::from_secs(1);
        let step = Duration::from_millis(400);
        let start = Instant::now();
        let first = [moved(small, 10)];
        coverage_frame(&mut frame, gpu, &[&first[0]], start);
        let retained = scratch(&frame);
        let bytes = frame.bytes();
        assert_eq!(
            frame.idle_deadline(interval),
            Some(start + interval),
            "the deadline is an idle interval after the generating frame"
        );
        // The same work at new placements misses again: nothing allocates.
        let second = [moved(small, 11)];
        coverage_frame(&mut frame, gpu, &[&second[0]], start + step);
        assert_eq!(scratch(&frame), retained, "an equal frame reuses scratch");
        assert_eq!(frame.bytes(), bytes, "and holds no more");
        assert_eq!(
            frame.idle_deadline(interval),
            Some(start + step + interval),
            "a generating frame defers the deadline"
        );
        // A frame needing more grows what it needs.
        let (a, b) = (moved(small, 12), moved(large, 13));
        coverage_frame(&mut frame, gpu, &[&a, &b], start + step * 2);
        assert!(frame.bytes() > bytes, "a larger frame grows scratch");
        let grown = scratch(&frame);
        assert_ne!(grown, retained, "a larger frame replaces what it outgrew");
        // A smaller frame afterwards keeps the larger buffers.
        coverage_frame(&mut frame, gpu, &[&moved(small, 14)], start + step * 3);
        assert_eq!(scratch(&frame), grown, "scratch never shrinks while used");
        let last = start + step * 3;
        // A frame that only replays neither releases nor defers.
        coverage_frame(&mut frame, gpu, &[&moved(small, 14)], last + step);
        assert_eq!(scratch(&frame), grown, "a replaying frame keeps scratch");
        assert_eq!(frame.idle_deadline(interval), Some(last + interval));
        // No frame follows: scratch survives until the interval elapses.
        assert!(!frame.release_idle(last + Duration::from_millis(999), interval));
        assert_eq!(
            scratch(&frame),
            grown,
            "released only once the idle time is up"
        );
        assert!(frame.release_idle(last + interval, interval));
        assert!(frame.buffers.is_none(), "an idle interval releases scratch");
        assert_eq!(
            frame.idle_deadline(interval),
            None,
            "nothing is left to release"
        );
        assert_eq!(
            frame.bytes(),
            frame.arena().map_or(0, wgpu::Buffer::size)
                + frame.arena.bounce.as_ref().map_or(0, wgpu::Buffer::size)
                + frame.draws_buffer.as_ref().map_or(0, wgpu::Buffer::size),
            "only the arena and draw list remain"
        );
        let mut whole = uncapped(&device);
        whole.begin_frame(Instant::now());
        for draw in &draws {
            whole.draw(draw, true).expect("road fits");
            assert_eq!(whole.entries[&draw.key].slices, 1, "the road is whole");
        }
        assert!(
            whole.batches.len() < draws.len(),
            "roads share batches under the binding limit"
        );
    }

    /// `slice` of `draw`, generated alone, uses no more bins and
    /// fragments than the bound its rows were sized for.
    fn assert_slice_fits_its_bound(
        draw: &GpuDraw,
        slice: &Slice,
        (device, queue, delivery): (
            &wgpu::Device,
            &wgpu::Queue,
            crate::render::shaders::ShaderDelivery,
        ),
    ) {
        let mut probe = uncapped(device);
        probe.begin_frame(Instant::now());
        probe
            .generate(draw, std::slice::from_ref(slice), true)
            .expect("a slice draws alone");
        probe.ensure(device).expect("slice buffers fit");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        probe.record_prologue(device, queue, &mut encoder, delivery, None);
        let work = probe.buffers.as_ref().expect("scratch sized").work.clone();
        let used = read_back(device, queue, &[(&work, 0, 8)], encoder);
        let (bins, fragments) = (used[0][0] as usize, used[0][1] as usize);
        assert!(
            bins <= slice.bound.bins && fragments <= slice.bound.fragments,
            "slice rows {:?} used ({bins}, {fragments}) of ({}, {})",
            slice.clip,
            slice.bound.bins,
            slice.bound.fragments
        );
        probe.submitted();
    }

    /// Records `frame`'s prologue for `draws`, checks every generation
    /// buffer and upload staging chunk against `limit`, and returns each
    /// draw's coverage on its 1024 × 768 target, decoded from the block
    /// the draw replays the way the cell-buffer vertex and fragment
    /// stages read it, and the bytes the batches' ordered uploads staged.
    fn coverage_blocks(
        frame: &mut GpuCoverage,
        (device, queue, delivery): (
            &wgpu::Device,
            &wgpu::Queue,
            crate::render::shaders::ShaderDelivery,
        ),
        draws: &[GpuDraw],
        limit: u64,
    ) -> (Vec<cherenkov_oracle::image::F32Image>, u64) {
        frame.ensure(device).expect("coverage buffers fit");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let timing = device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| {
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("coverage prologue timing"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                })
            });
        frame.record_prologue(
            device,
            queue,
            &mut encoder,
            delivery,
            timing
                .as_ref()
                .map(|query_set| wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
        );
        let buffers = frame.buffers.as_ref().expect("scratch sized");
        for buffer in [
            &buffers.params,
            &buffers.elements,
            &buffers.paths,
            &buffers.segments,
            &buffers.coverage,
            &buffers.strokes,
            &buffers.args,
            &buffers.work,
        ] {
            assert!(buffer.size() <= limit, "scratch is {} bytes", buffer.size());
        }
        for upload in &frame.uploads {
            assert!(
                upload.size() <= limit,
                "upload staging is {}",
                upload.size()
            );
        }
        let staged = frame.uploads.iter().map(wgpu::Buffer::size).sum();
        let arena = frame.arena().expect("retained arena");
        let copies: Vec<_> = draws
            .iter()
            .map(|draw| {
                let entry = &frame.entries[&draw.key];
                (arena, u64::from(entry.base) * 4, u64::from(entry.words) * 4)
            })
            .collect();
        let blocks = read_back(device, queue, &copies, encoder);
        let images = draws
            .iter()
            .zip(&blocks)
            .map(|(draw, block)| {
                let entry = &frame.entries[&draw.key];
                assert!(block[0] > 0, "the road emits");
                assert!(block[0] <= entry.geometry_slots, "within the block's slots");
                decode_block(block, entry.texel_base - entry.base, draw.shift)
            })
            .collect();
        frame.submitted();
        assert!(frame.buffers.is_some(), "submit retains generation scratch");
        (images, staged)
    }

    /// The coverage one arena block holds — its count word, geometry
    /// records and packed R8 texels (`shared.wgsl`'s `join_gpu_instance`
    /// and `path_cell_coverage`) — on a 1024 × 768 target, moved by the
    /// draw's shift.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "integer pixel bounds on the target"
    )]
    fn decode_block(
        block: &[u32],
        texel_base: u32,
        shift: [i32; 2],
    ) -> cherenkov_oracle::image::F32Image {
        const SIZE: (usize, usize) = (1024, 768);
        let mut coverage = vec![0.0f32; SIZE.0 * SIZE.1];
        for record in block[BLOCK_HEADER_WORDS as usize..]
            .as_chunks::<{ GEOMETRY_WORDS as usize }>()
            .0
            .iter()
            .take(block[0] as usize)
        {
            let [x0, y0, x1, y1] = [0, 1, 2, 3].map(|i| f32::from_bits(record[i]) as usize);
            let (offset, interior) = (record[4], record[5]);
            let (start, end) = ((interior & 0xffff) as usize, (interior >> 16) as usize);
            let stored = x1 - x0 - (end - start);
            for y in y0..y1 {
                for x in x0..x1 {
                    let column = x - x0;
                    let value = if offset == u32::MAX || (start..end).contains(&column) {
                        1.0
                    } else {
                        let packed = column - if column >= end { end - start } else { 0 };
                        let pixel = (y - y0) * stored + packed;
                        let word = block[(texel_base + offset) as usize + pixel / 4];
                        f32::from(((word >> ((pixel % 4) * 8)) & 255) as u8) / 255.0
                    };
                    let (px, py) = (
                        usize::try_from(i64::try_from(x).unwrap() + i64::from(shift[0])).unwrap(),
                        usize::try_from(i64::try_from(y).unwrap() + i64::from(shift[1])).unwrap(),
                    );
                    assert!(px < SIZE.0 && py < SIZE.1, "coverage lies on the target");
                    coverage[py * SIZE.0 + px] = value;
                }
            }
        }
        cherenkov_oracle::image::F32Image {
            width: 1024,
            height: 768,
            pixels: coverage.into_iter().map(|value| [value; 4]).collect(),
        }
    }

    /// The generated road scene's two stroked roads, prepared for GPU
    /// coverage on a 1024 × 768 target.
    fn road_paths() -> Vec<crate::render::path::GpuPath> {
        let scene = cherenkov_scene::Scene::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scenes/perf/road-network-batches"),
        )
        .expect("generated road scene");
        let cherenkov_scene::Item::Layer(layer) = &scene.root.items[0] else {
            panic!("road layer")
        };
        layer
            .items
            .iter()
            .map(|item| {
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
                crate::render::path::stroke::prepare(
                    &shape,
                    &stroke.into(),
                    Affine::IDENTITY,
                    crate::render::path::FLATTEN,
                    (1024.0, 768.0),
                )
                .expect("road encodes")
            })
            .collect()
    }

    /// One commit of `draws` through the lifecycle the renderer drives —
    /// touch, settle, draw, size, prologue, submit — returning the count
    /// word each draw's block holds once the prologue ran.
    fn coverage_frame(
        frame: &mut GpuCoverage,
        (device, queue, delivery): (
            &wgpu::Device,
            &wgpu::Queue,
            crate::render::shaders::ShaderDelivery,
        ),
        draws: &[&GpuDraw],
        at: Instant,
    ) -> Vec<u32> {
        frame.begin_frame(at);
        for draw in draws {
            frame.touch(draw.key);
        }
        frame.settle(true);
        for draw in draws {
            frame.draw(draw, true).expect("road draw fits");
        }
        let planned = frame.planned_bytes();
        frame.ensure(device).expect("coverage buffers fit");
        assert_eq!(
            frame.bytes(),
            planned,
            "admission budgets the arena, bounce and scratch ensure holds"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        frame.record_prologue(device, queue, &mut encoder, delivery, None);
        let arena = frame.arena().expect("retained arena");
        let copies: Vec<_> = draws
            .iter()
            .map(|draw| (arena, u64::from(frame.entries[&draw.key].base) * 4, 4))
            .collect();
        let counts = read_back(device, queue, &copies, encoder)
            .into_iter()
            .map(|words| words[0])
            .collect();
        frame.submitted();
        counts
    }

    /// A pan re-places content every step, so each frame misses with new
    /// placements while what stays put keeps hitting. Once the first
    /// frame sized it, the arena repacks in place — moving the still
    /// road's block to the front through the bounce buffer — and no step
    /// replaces it (water-rs/waterui#4108).
    #[test]
    fn panning_coverage_reuses_its_arena() {
        let Some((device, queue, delivery)) = delivery_device() else {
            return;
        };
        let gpu = (&device, &queue, delivery);
        let paths = road_paths();
        let words = |path: &crate::render::path::GpuPath| {
            let mut probe = GpuCoverage::with_limits(&device.limits(), None, 0);
            probe.begin_frame(Instant::now());
            let draw = GpuDraw::new(1, 0, Affine::IDENTITY, path.clone(), (1024, 768));
            probe.draw(&draw, true).expect("road draw fits");
            probe.entries[&draw.key].words
        };
        // The moving road is the larger, so evicting its last placement
        // leaves the frontier past twice the live words and the settle
        // repacks the still road's block, which sits above it.
        let (moving, still) = if words(&paths[0]) > words(&paths[1]) {
            (&paths[0], &paths[1])
        } else {
            (&paths[1], &paths[0])
        };
        let still = GpuDraw::new(1, 0, Affine::IDENTITY, still.clone(), (1024, 768));
        let step =
            |content| GpuDraw::new(content, 0, Affine::IDENTITY, moving.clone(), (1024, 768));
        // A zero undrawn budget evicts each step's placement at the next.
        let mut frame = GpuCoverage::new(&device, None, 0, super::super::PipelineCache::default());
        let first = coverage_frame(&mut frame, gpu, &[&step(100), &still], Instant::now());
        assert!(first.iter().all(|&count| count > 0), "both roads emit");
        let arena = frame.arena().expect("sized arena").clone();
        let generation = frame.arena_generation();
        let base = frame.entries[&still.key].base;
        for content in 101..105 {
            let counts = coverage_frame(&mut frame, gpu, &[&step(content), &still], Instant::now());
            assert_eq!(counts, first, "every step emits what the first did");
            assert_eq!(
                frame.arena_generation(),
                generation,
                "the arena is never replaced"
            );
            assert_eq!(frame.arena(), Some(&arena));
        }
        assert!(
            frame.entries[&still.key].base < base && frame.arena.bounce.is_some(),
            "the still road's block moved to the front in place"
        );
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
            // Timestamps where the adapter has them, so the prologue's pass
            // timing is validated across every batch it splits into.
            required_features: required_features
                | wgpu::Features::SHADER_F16
                | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
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
        // The callback runs on wgpu's thread: panicking there aborts the
        // whole test process (the Windows nightly died as 0xc0000409 on a
        // timed-out map). Channel each result back so the test thread owns
        // every failure (#3909).
        let (tx, rx) = std::sync::mpsc::channel();
        for target in &targets {
            let tx = tx.clone();
            target.map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
        }
        drop(tx);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                // WARP rasterizes the 17k-dash coverage frame on the CPU;
                // the observed ~170 s on the Windows runner exceeded the
                // 120 s budget and aborted the map (water-rs/waterui#3909).
                // 300 s covers the software path while a genuinely hung
                // kernel still dies under nextest's watchdog.
                timeout: Some(std::time::Duration::from_secs(300)),
            })
            .expect("the stroke kernels finish");
        for _ in &targets {
            rx.recv()
                .expect("every map_async callback reports")
                .expect("map readback");
        }
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
            let mut frame = uncapped(&device);
            frame.begin_frame(Instant::now());
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
