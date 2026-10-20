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
//! word, the emitted 32-byte geometry records and the packed R8 texels.
//! The cell-buffer vertex stage joins each record to the draw's anchor
//! instance (water-rs/waterui#3081); the draw loops read each draw's
//! indirect arguments from `draws`, whose count words arrive through
//! 4-byte buffer copies.

use cherenkov::RenderError;
use kurbo::Affine;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use crate::render::instance::Instance;
use crate::render::path::{GpuElement, GpuPath};

/// u32 words of one `GpuPath` record on the device.
const PATH_WORDS: usize = 40;
/// u32 words of `Params`.
const PARAMS_WORDS: usize = 20;
/// u32 words of indirect arguments per GPU draw: `draw_indirect` (4) +
/// `draw_indexed_indirect` (5) + `draw_indirect` (4) = 13.
pub const ARGS_WORDS: u32 = 13;
/// u32 words of dispatch-domain arguments in `args`.
const DISPATCH_ARGS_WORDS: u64 = 12;
/// Byte offset of a draw's opaque-phase `draw_indirect` record.
pub const ARGS_OPAQUE: u32 = 0;
/// Byte offset of its partial-phase `draw_indexed_indirect` record.
pub const ARGS_PARTIAL: u32 = 16;
/// Byte offset of its normal-phase `draw_indirect` record.
pub const ARGS_NORMAL: u32 = 36;
/// The kernel workgroup size `coverage.wgsl` declares.
const WORKGROUP: u32 = 64;
const CLASSIFY_WIDTH: u32 = 128;
/// Decision R2's hard per-frame cap on the coverage buffers' total
/// size: a bound over it is an explicit `Unsupported`, never a retry.
const GPU_COV_MAX_BYTES: u64 = 256 << 20;

/// u32 words of one geometry record in the arena: the device bounds
/// vec4 plus `(texel offset, width, interior, kind)`.
const GEOMETRY_WORDS: u32 = 8;
/// u32 words of one block's header: the emitted instance count and pad.
const BLOCK_HEADER_WORDS: u32 = 4;

/// One GPU draw of the frame: the coverage entry it replays and the
/// draw's placement shift. Block addresses resolve at `write_draw` and
/// prologue time, after any compaction has settled (water-rs/waterui#3081).
#[derive(Clone, Copy)]
struct Draw {
    /// The entry's retention key.
    key: u64,
    /// `draw offset − entry offset`: the integer translation the
    /// anchor's `shape.half` carries into the vertex-stage join.
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
    /// Integer translation the block's coverage is relative to.
    offset: [i32; 2],
    /// The generating frame's submit landed. Entries that never made
    /// it into a submission die at the next `begin_frame`, so no frame
    /// can replay a block whose kernels never ran.
    generated: bool,
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
    /// Bumped on every buffer replacement so `bind0` is rebuilt.
    generation: u64,
    /// The buffer moved since the last `ensure` — `bind0` is stale.
    dirty: bool,
}

impl CoverageArena {
    const fn new() -> Self {
        Self {
            buffer: None,
            words: 0,
            tail: 0,
            generation: 0,
            dirty: false,
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
        self.generation += 1;
        self.dirty = true;
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
}

/// The frame's coverage scratch buffers, sized exactly by the R2 bound.
struct Buffers {
    params: wgpu::Buffer,
    elements: wgpu::Buffer,
    paths: wgpu::Buffer,
    segments: wgpu::Buffer,
    coverage: wgpu::Buffer,
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
    /// Retained coverage entries by placement key (water-rs/waterui#3081).
    entries: FxHashMap<u64, Entry>,
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
    /// `max_storage_buffer_binding_size.min(max_buffer_size)` in words.
    limit_words: u64,
    /// An unrecorded compaction's copy plan. Held until a submit lands
    /// it — a failed or retried commit re-records it rather than
    /// pointing entries at content that never moved.
    compaction: Option<Compaction>,
    /// Compute input elements, all paths concatenated.
    elements: Vec<GpuElement>,
    element_sources: Vec<Arc<[GpuElement]>>,
    element_total: u32,
    /// `(path, strip)` items uploaded after atomic counters and row windings.
    work: Vec<[u32; 2]>,
    /// One GPU draw per applied `GpuPath`, in apply order.
    draws: Vec<Draw>,
    /// CPU words of the frame's `draws` buffer: `ARGS_WORDS` per draw,
    /// the three count words supplied by the prologue's copies.
    draw_args: Vec<u32>,
    /// The per-draw indirect records: `INDIRECT | COPY_DST`.
    draws_buffer: Option<wgpu::Buffer>,
    /// Segments across all paths.
    seg_total: u32,
    /// Logical strip/column key range across paths; no dense allocation.
    cov_total: u32,
    bin_total: u32,
    fragment_total: u32,
    /// Row winding counters across all paths.
    row_total: u32,
    /// The R2 bound's accumulated byte size — the cap's denominator.
    bound_bytes: u64,
    /// GPU buffer bytes currently held — `Engine::memory()` reports it.
    bytes: u64,
    /// The coverage arena: geometry records and texels, one block per draw.
    arena: CoverageArena,
    /// The prologue went into some surface's encoder already.
    encoded: bool,
    // Device objects, created on the first dispatch that needs them.
    layout: Option<wgpu::BindGroupLayout>,
    args_layout: Option<wgpu::BindGroupLayout>,
    pipes: Option<Pipes>,
    buffers: Option<Buffers>,
}

impl GpuCoverage {
    pub fn new(device: &wgpu::Device, budget: u64) -> Self {
        let limits = device.limits();
        Self {
            paths: Vec::new(),
            entries: FxHashMap::default(),
            epoch: 0,
            generated: 0,
            live_words: 0,
            undrawn_bytes: 0,
            budget,
            limit_words: limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size)
                / 4,
            compaction: None,
            elements: Vec::new(),
            element_sources: Vec::new(),
            element_total: 0,
            work: Vec::new(),
            draws: Vec::new(),
            draw_args: Vec::new(),
            draws_buffer: None,
            seg_total: 0,
            cov_total: 0,
            bin_total: 0,
            fragment_total: 0,
            row_total: 0,
            bound_bytes: 0,
            bytes: 0,
            arena: CoverageArena::new(),
            encoded: false,
            layout: None,
            args_layout: None,
            pipes: None,
            buffers: None,
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
        self.elements.clear();
        self.element_sources.clear();
        self.element_total = 0;
        self.work.clear();
        self.draws.clear();
        self.draw_args.clear();
        self.seg_total = 0;
        self.cov_total = 0;
        self.bin_total = 0;
        self.fragment_total = 0;
        self.row_total = 0;
        self.bound_bytes = 0;
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

    /// GPU buffer bytes held — the `Engine::memory()` contribution of
    /// scope item 8.
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Retained host input/draw-list allocation capacities.
    pub fn cpu_bytes(&self) -> u64 {
        (self.entries.capacity() * std::mem::size_of::<(u64, Entry)>()
            + self.paths.capacity() * std::mem::size_of::<[u32; PATH_WORDS]>()
            + self.elements.capacity() * std::mem::size_of::<GpuElement>()
            + self.element_sources.capacity() * std::mem::size_of::<Arc<[GpuElement]>>()
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
    pub fn touch(&mut self, key: u64) {
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
        if !self.encoded {
            return;
        }
        for entry in self.entries.values_mut() {
            entry.generated = true;
            entry.src = None;
        }
        self.compaction = None;
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
        clippy::too_many_arguments,
        reason = "the draw record's six fields plus the device"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "hit arm plus the miss's bound and record"
    )]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        key: u64,
        offset: [i32; 2],
        rule: u32,
        raster: Affine,
        clip: [f32; 4],
        prepared: &GpuPath,
    ) -> Result<u32, RenderError> {
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.last_used != self.epoch {
                self.undrawn_bytes -= u64::from(entry.words) * 4;
                entry.last_used = self.epoch;
            }
            self.draws.push(Draw {
                key,
                shift: [offset[0] - entry.offset[0], offset[1] - entry.offset[1]],
            });
            return Ok(u32::try_from(self.draws.len() - 1).expect("draw count fits u32"));
        }
        let GpuPath {
            elements, bound, ..
        } = prepared;
        // Decision R2: the bound sizes everything, so over the cap is a
        // fail, not a resize.
        let rows = bound.grid_h;
        let strips = bound.grid_h.div_ceil(crate::render::path::STRIP_H);
        let to64 = |n: usize| u64::try_from(n).expect("bound fits u64");
        let fragment_total =
            self.fragment_total + u32::try_from(bound.fragments).expect("fragments");
        let cov_total = self
            .cov_total
            .checked_add(
                u32::try_from(bound.grid_w * strips)
                    .map_err(|_| RenderError::Unsupported("GPU coverage key range exceeds u32"))?,
            )
            .ok_or(RenderError::Unsupported(
                "GPU coverage key range exceeds u32",
            ))?;
        let previous_groups = group_words(self.fragment_total, self.cov_total);
        let next_groups = group_words(fragment_total, cov_total);
        let path_bytes = to64(elements.len()) * (std::mem::size_of::<GpuElement>() as u64)
            + to64(bound.segments) * 16
            + to64(bound.bins) * 12
            + to64(bound.fragments) * 40
            + to64(bound.fragments.div_ceil(64)) * 64
            + to64(strips) * 12
            + to64(rows) * 4
            + to64(bound.texels.div_ceil(4) + bound.cell_cap) * 4
            + to64(bound.instance_cap) * u64::from(GEOMETRY_WORDS) * 4
            + 16
            + u64::from(ARGS_WORDS) * 4
            + (PATH_WORDS as u64) * 4
            + to64(strips) * (1 + to64(bound.grid_w.div_ceil(CLASSIFY_WIDTH as usize))) * 8
            + to64(strips * bound.grid_w.div_ceil(CLASSIFY_WIDTH as usize)) * 16;
        let previous_tokens = self.fragment_total.min(self.cov_total) * 6;
        let next_tokens = fragment_total.min(cov_total) * 6;
        self.bound_bytes += path_bytes
            + u64::from(next_groups - previous_groups + next_tokens - previous_tokens) * 4;
        if self.bound_bytes + 18 * 1024 + 128 + 48 > GPU_COV_MAX_BYTES {
            return Err(RenderError::Unsupported(
                "GPU coverage bound exceeds the per-frame cap",
            ));
        }
        // Retained-but-undrawn entries pay for themselves: evict them
        // LRU-first until they fit the budget again (§3.3).
        self.evict_undrawn();
        let record = u32::try_from(self.paths.len()).expect("path count fits u32");
        let elem_base = self.element_total;
        let geometry_slots = u32::try_from(bound.instance_cap).expect("instance cap fits u32");
        let texels =
            u32::try_from(bound.texels.div_ceil(4) + bound.cell_cap).expect("texel words fit u32");
        let (base, texel_base, block_words) = self.alloc_block(device, geometry_slots, texels)?;
        let grid_base = self.cov_total;
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
        words[10] = self.seg_total;
        words[11] = u32::try_from(bound.segments).expect("segments fit u32");
        words[12] = grid_base;
        words[13] = texel_base;
        words[14] = base + BLOCK_HEADER_WORDS;
        words[16] = rule;
        words[17] = u32::try_from(bound.grid_w).expect("grid fits u32");
        words[18] = u32::try_from(bound.grid_h).expect("grid fits u32");
        words[19] = geometry_slots;
        words[21] = self.row_total;
        words[20] = texels;
        // affine = (a, c, b, d); translate = (e, f)
        words[24] = (xx as f32).to_bits();
        words[25] = (xy as f32).to_bits();
        words[26] = (yx as f32).to_bits();
        words[27] = (yy as f32).to_bits();
        words[28] = (tx as f32).to_bits();
        words[29] = (ty as f32).to_bits();
        words[30] = clip[2].to_bits();
        words[31] = clip[3].to_bits();
        words[38] = clip[0].to_bits();
        words[39] = clip[1].to_bits();

        self.paths.push(words);
        self.element_total += u32::try_from(elements.len()).expect("elements fit u32");
        self.element_sources.push(Arc::clone(elements));
        self.entries.insert(
            key,
            Entry {
                base,
                texel_base,
                words: block_words,
                geometry_slots,
                record,
                offset,
                generated: false,
                src: None,
                last_used: self.epoch,
            },
        );
        self.generated += 1;
        self.draws.push(Draw { key, shift: [0, 0] });
        self.seg_total += u32::try_from(bound.segments).expect("segments");
        self.bin_total += u32::try_from(bound.bins).expect("bins");
        self.fragment_total = fragment_total;
        self.cov_total = cov_total;
        self.row_total += u32::try_from(rows).expect("rows");

        Ok(u32::try_from(self.draws.len() - 1).expect("draw count fits u32"))
    }

    /// Evict undrawn entries LRU-first until their retained bytes fit
    /// the budget again. Entries this frame drew — `last_used` at the
    /// current epoch — are never touched (§3.3).
    fn evict_undrawn(&mut self) {
        if self.undrawn_bytes <= self.budget {
            return;
        }
        let mut undrawn: Vec<u64> = self
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
        device: &wgpu::Device,
        geometry_slots: u32,
        texels: u32,
    ) -> Result<(u32, u32, u32), RenderError> {
        let block_words = (BLOCK_HEADER_WORDS + geometry_slots * GEOMETRY_WORDS + texels)
            .next_multiple_of(GEOMETRY_WORDS);
        // The 2x fragmentation rule, or a block that does not fit at the
        // tail, rebuilds live blocks into a fresh buffer (§3.3).
        if self.arena.tail + block_words > self.arena.words
            || u64::from(self.arena.tail) > 2 * self.live_words
        {
            self.compact(device, block_words)?;
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

    /// Repack every live entry into a fresh arena buffer sized to the
    /// live words plus this frame's immediate need, with the doubling
    /// headroom `sized_buffer` growth uses (§3.3). Generated entries
    /// schedule one copy each into the coverage prologue; ungenerated
    /// ones redirect their dispatch record to the new block.
    fn compact(&mut self, device: &wgpu::Device, extra: u32) -> Result<(), RenderError> {
        let mut order: Vec<u64> = self.entries.keys().copied().collect();
        order.sort_by_key(|key| self.entries[key].base);
        let mut tail = 0u32;
        let mut moves = Vec::with_capacity(order.len());
        for key in order {
            let entry = &self.entries[&key];
            moves.push((key, tail));
            tail += entry.words;
        }
        if u64::from(tail) + u64::from(extra) > self.limit_words {
            return Err(RenderError::Unsupported(
                "GPU coverage output exceeds device buffer limit",
            ));
        }
        let size = ((u64::from(tail) + u64::from(extra)) * 8).max(64);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coverage arena"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
        self.arena.buffer = Some(buffer);
        self.arena.words = u32::try_from(size / 4).expect("arena words fit u32");
        self.arena.tail = tail;
        self.arena.generation += 1;
        self.arena.dirty = true;
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
        let first_instance = entry.base / 8;
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

    /// Size the draws buffer to the frame's draws. The arena itself is
    /// already sized — commit-time allocations create or compact it —
    /// and this returns whether it moved, so `bind0` is rebuilt.
    pub fn ensure(&mut self, device: &wgpu::Device) -> bool {
        self.draws_buffer = Some(sized_buffer(
            device,
            self.draws_buffer.as_ref(),
            "gpu draws",
            (self.draws.len() as u64).max(1) * u64::from(ARGS_WORDS) * 4,
            wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        ));
        let dirty = self.arena.dirty;
        self.arena.dirty = false;
        dirty
    }

    pub fn trim(&mut self) {
        self.entries.clear();
        self.compaction = None;
        self.live_words = 0;
        self.undrawn_bytes = 0;
        self.buffers = None;
        self.draws_buffer = None;
        self.bytes = 0;
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
    ) -> Result<(), RenderError> {
        if self.encoded || !self.has_draws() {
            return Ok(());
        }
        self.encoded = true;
        let arena = self.arena.buffer().expect("coverage arena sized").clone();
        if let Some(compaction) = &self.compaction {
            for &(src, dst, len) in &compaction.copies {
                encoder.copy_buffer_to_buffer(&compaction.source, src, &arena, dst, len);
            }
        }
        if self.has_misses() {
            self.dispatch(device, queue, encoder, delivery, timestamp_writes)?;
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
        self.bytes = self.buffers.as_ref().map_or(0, |b| {
            b.params.size()
                + b.elements.size()
                + b.paths.size()
                + b.segments.size()
                + b.coverage.size()
                + b.args.size()
                + b.work.size()
        }) + self.arena.buffer.as_ref().map_or(0, wgpu::Buffer::size)
            + self.draws_buffer.as_ref().map_or(0, wgpu::Buffer::size);
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
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        delivery: crate::render::shaders::ShaderDelivery,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> Result<(), RenderError> {
        self.ensure_pipes(device, delivery);

        for (index, path) in self.paths.iter().enumerate() {
            for strip in 0..path[18].div_ceil(
                u32::try_from(crate::render::path::STRIP_H).expect("coverage capacity fits u32"),
            ) {
                self.work.push([
                    u32::try_from(index).expect("coverage capacity fits u32"),
                    strip,
                ]);
            }
        }
        let strip_total = u32::try_from(self.work.len()).expect("coverage capacity fits u32");
        for (index, path) in self.paths.iter_mut().enumerate() {
            path[32] = u32::try_from(self.work.len()).expect("work count fits u32") - strip_total;
            let tiles = path[17].div_ceil(CLASSIFY_WIDTH) * path[18].div_ceil(4);
            for tile in 0..tiles {
                self.work
                    .push([u32::try_from(index).expect("path index fits u32"), tile]);
            }
        }
        let tile_total = u32::try_from(self.work.len()).expect("work count fits u32") - strip_total;
        let bin_base = 0;
        let fragment_base = bin_base + self.bin_total * 3;
        let sorted = fragment_base + self.fragment_total * 8;
        let groups = sorted + self.fragment_total * 2;
        let sort_scratch = groups + group_words(self.fragment_total, self.cov_total);
        let radix_pass_count = radix_passes(self.cov_total);
        // Start on the opposite side for odd pass counts so the result always
        // precedes the group arena, leaving sort_scratch free for tokens.
        let (sort_a, sort_b) = if radix_pass_count.is_multiple_of(2) {
            (sorted, sort_scratch)
        } else {
            (sort_scratch, sorted)
        };
        let histogram = sort_scratch + self.fragment_total * 2;
        let histogram_end =
            histogram + self.fragment_total.div_ceil(WORKGROUP).div_ceil(256) * 256 * 16;
        // Classification starts after radix sorting has released sort_scratch and
        // its histogram. The sorted keys stay live through texel generation.
        let tokens = sort_scratch;
        let token_end = tokens + (self.fragment_total.min(self.cov_total) * 2 + strip_total) * 3;
        let params: [u32; PARAMS_WORDS] = [
            self.element_total,
            self.row_total,
            strip_total,
            u32::try_from(self.paths.len()).expect("coverage capacity fits u32"),
            tile_total,
            CLASSIFY_WIDTH,
            self.cov_total,
            bin_base,
            sort_scratch,
            fragment_base,
            self.fragment_total,
            sort_a,
            sort_b,
            histogram,
            groups,
            tokens,
            sorted,
            0,
            0,
            0,
        ];
        let cov_words = u64::from(
            token_end
                .max(histogram_end)
                .max(sort_scratch + tile_total * 4),
        );
        let scratch_bytes = cov_words * 4;
        let limits = device.limits();
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let work_header_bytes = u64::from(4 + self.row_total) * 4;
        let work_bytes = work_header_bytes + self.work.len() as u64 * 8;
        let sizes = [
            scratch_bytes,
            u64::from(self.element_total) * 48,
            self.paths.len() as u64 * PATH_WORDS as u64 * 4,
            u64::from(self.seg_total) * 16,
            DISPATCH_ARGS_WORDS * 4,
            work_bytes,
        ];
        if sizes.into_iter().any(|size| size > limit) {
            return Err(RenderError::Unsupported(
                "GPU coverage input exceeds device buffer limit",
            ));
        }

        let seg_words = u64::from(self.seg_total).max(1);
        let previous = self.buffers.as_ref();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let buffers = Buffers {
            params: sized_buffer(
                device,
                previous.map(|b| &b.params),
                "cov params",
                8 * 256,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            elements: sized_buffer(
                device,
                previous.map(|b| &b.elements),
                "cov elements",
                u64::from(self.element_total).max(1) * (std::mem::size_of::<GpuElement>() as u64),
                storage,
            ),
            paths: sized_buffer(
                device,
                previous.map(|b| &b.paths),
                "cov paths",
                (self.paths.len() as u64) * (PATH_WORDS as u64) * 4,
                storage,
            ),
            segments: sized_buffer(
                device,
                previous.map(|b| &b.segments),
                "cov segments",
                seg_words * 16,
                storage,
            ),
            coverage: sized_buffer(
                device,
                previous.map(|b| &b.coverage),
                "cov scratch",
                cov_words * 4,
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
                work_bytes,
                storage,
            ),
        };

        for radix_pass in 0..radix_pass_count {
            let mut block = params;
            block[17] = radix_pass;
            queue.write_buffer(
                &buffers.params,
                u64::from(radix_pass) * 256,
                bytemuck::cast_slice(&block),
            );
        }
        for (index, source) in self.element_sources.iter().enumerate() {
            let path_i = u32::try_from(index).expect("path index");
            self.elements
                .extend(source.iter().copied().map(|mut element| {
                    element.head[2] = path_i;
                    element
                }));
        }
        queue.write_buffer(&buffers.elements, 0, bytemuck::cast_slice(&self.elements));
        queue.write_buffer(
            &buffers.work,
            work_header_bytes,
            bytemuck::cast_slice(&self.work),
        );
        queue.write_buffer(
            &buffers.paths,
            0,
            bytemuck::cast_slice(self.paths.as_flattened()),
        );
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
                resource: buffers.args.as_entire_binding(),
            },
        ];
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu coverage"),
            layout,
            entries: &entries
                .iter()
                .filter(|entry| entry.binding != 7)
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
            pass.set_pipeline(&pipes.bin);
            dispatch(&mut pass, params[0]);
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
                pass.set_bind_group(0, &bind, &[radix_pass * 256]);
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
                u32::try_from(self.paths.len()).expect("coverage capacity fits u32"),
            );
        }
        self.buffers = Some(buffers);
        Ok(())
    }

    /// Lazily build the bind layout and the pipelines from the
    /// precompiled coverage module.
    fn ensure_pipes(
        &mut self,
        device: &wgpu::Device,
        delivery: crate::render::shaders::ShaderDelivery,
    ) {
        if self.pipes.is_some() {
            return;
        }
        let entries = super::layout_entries(crate::render::bindings::COVERAGE_GROUP0);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu coverage"),
            entries: &entries
                .iter()
                .filter(|entry| entry.binding != 7)
                .copied()
                .collect::<Vec<_>>(),
        });
        let args_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu coverage argument writes"),
            entries: &entries,
        });
        let module = delivery.coverage_module(device);
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu coverage"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let args_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu coverage argument writes"),
            bind_group_layouts: &[Some(&args_layout)],
            immediate_size: 0,
        });
        let make = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(if matches!(entry, "prepare_dispatch" | "prepare_cells") {
                    &args_pipe_layout
                } else {
                    &pipe_layout
                }),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        self.pipes = Some(Pipes {
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
        });
        self.layout = Some(layout);
        self.args_layout = Some(args_layout);
    }
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
    use kurbo::{PathEl, Point};

    #[test]
    fn radix_passes_order_keys_below_the_sentinel() {
        assert_eq!(radix_passes(255), 2);
        assert_eq!(radix_passes(256), 3);
        assert_eq!(radix_passes(1 << 16), 5);
        assert_eq!(radix_passes(1 << 24), 7);
        assert_eq!(radix_passes(u32::MAX), 8);
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
                &device,
                1,
                [0, 0],
                0,
                Affine::IDENTITY,
                [0.0, 0.0, 4096.0, 4096.0],
                &large_path,
            )
            .expect("sparse 4096-square capacity");
        assert!(frame.bound_bytes < GPU_COV_MAX_BYTES);
        let huge_path = rectangle(1_000_000);
        let before = frame.paths.len();
        assert!(
            frame
                .draw(
                    &device,
                    2,
                    [0, 0],
                    0,
                    Affine::IDENTITY,
                    [0.0, 0.0, 1_000_000.0, 1_000_000.0],
                    &huge_path,
                )
                .is_err()
        );
        assert_eq!(frame.paths.len(), before, "reject before appending a path");
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
}
