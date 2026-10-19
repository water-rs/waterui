//! The GPU coverage compute route of issue #2543, decision S2: one set
//! of dispatches in the frame's own encoder, ahead of its raster
//! passes, in the same submission, without readback or a fence. Retained
//! geometric coverage is replayed with the current frame's drawing state.
//!
//! The lowering leaves each routed path's curve elements, fill rule,
//! transform and template instance in a [`PendingRaster::GpuPath`]
//! record. `apply_pending` joins them into this frame context, which
//! accumulates the per-path dispatch records and the decision-R2 bound
//! they size from. The first encoded surface writes the compute input
//! and records flattening, strip binning, boundary sorting, winding prefix,
//! local area integration, classification and indirect argument emission; and the draw loops read each path's indirect arguments
//! from `args`.

use cherenkov::RenderError;
use kurbo::Affine;
use rustc_hash::FxHashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use crate::render::instance::Instance;
use crate::render::path::{GpuElement, GpuPath};

/// u32 words of one `GpuPath` record on the device.
const PATH_WORDS: usize = 108;
/// u32 words of `Params`.
const PARAMS_WORDS: usize = 20;
/// u32 words of indirect arguments per path: `draw_indirect` (4) +
/// `draw_indexed_indirect` (5) + `draw_indirect` (4) = 13.
pub const ARGS_WORDS: u32 = 13;
/// Byte offset of a path's opaque-phase `draw_indirect` record.
pub const ARGS_OPAQUE: u32 = 0;
/// Byte offset of its partial-phase `draw_indexed_indirect` record.
pub const ARGS_PARTIAL: u32 = 16;
/// Byte offset of its normal-phase `draw_indirect` record.
pub const ARGS_NORMAL: u32 = 36;
/// The kernel workgroup size `coverage.wgsl` declares.
const WORKGROUP: u32 = 64;
/// Decision R2's hard per-frame cap on the coverage buffers' total
/// size: a bound over it is an explicit `Unsupported`, never a retry.
const GPU_COV_MAX_BYTES: u64 = 256 << 20;

/// Instance vec4 count; `emit_instance` strides by it.
const INSTANCE_VEC4S: u32 = 17;

/// The dispatch context of one frame's GPU-routed paths. Built at
/// `apply_pending`, finalized once the frame's instance total is known,
/// dispatched once on the first encoded surface.
pub struct GpuCoverage {
    /// `GpuPath` record words, in apply order.
    paths: Vec<[u32; PATH_WORDS]>,
    /// Compute input elements, all paths concatenated.
    elements: Vec<GpuElement>,
    element_sources: Vec<Arc<[GpuElement]>>,
    element_total: u32,
    /// Work list: `(path, row)` entries then `(path, strip)` entries —
    /// `work_strip_off` words in.
    work: Vec<[u32; 2]>,
    /// Per-path byte offset into `args`, apply order.
    args_bases: Vec<u32>,
    /// Instance slots reserved for kernel emission, all paths.
    inst_total: u32,
    /// Segments across all paths.
    seg_total: u32,
    /// Logical strip/column key range across paths; no dense allocation.
    cov_total: u32,
    bin_total: u32,
    fragment_total: u32,
    /// R8 texel words across all paths.
    texel_words: u32,
    /// Work-list row entries (strips follow).
    row_total: u32,
    /// The R2 bound's accumulated byte size — the cap's denominator.
    bound_bytes: u64,
    /// GPU buffer bytes currently held — `Engine::memory()` reports it.
    bytes: u64,
    /// Where the path records' emission begins in the instance buffer.
    inst_origin: u32,
    /// The dispatch went into some surface's encoder already.
    dispatched: bool,
    // Device objects, created on the first dispatch that needs them.
    layout: Option<wgpu::BindGroupLayout>,
    args_layout: Option<wgpu::BindGroupLayout>,
    pipes: Option<Pipes>,
    buffers: Option<Buffers>,
    identity: DefaultHasher,
    cache: FxHashMap<u64, CachedCoverage>,
    cache_bytes: u64,
    cache_epoch: u64,
}

/// Only geometric output survives: drawing state is supplied again on replay.
#[derive(Clone)]
struct CachedCoverage {
    geometry: wgpu::Buffer,
    texels: wgpu::Buffer,
    translations: Vec<[f32; 2]>,
    last_used: u64,
}

impl CachedCoverage {
    fn bytes(&self) -> u64 {
        self.geometry.size() + self.texels.size()
    }
}

const COVERAGE_CACHE_BYTES: u64 = 64 << 20;

/// Pipelines for the sparse boundary coverage stages.
struct Pipes {
    prepare_dispatch: wgpu::ComputePipeline,
    prepare_cells: wgpu::ComputePipeline,
    store_cache: wgpu::ComputePipeline,
    replay_cache: wgpu::ComputePipeline,
    replay_texels: wgpu::ComputePipeline,
    radix_count: wgpu::ComputePipeline,
    radix_scan_blocks: wgpu::ComputePipeline,
    radix_prefix: wgpu::ComputePipeline,
    radix_scatter: wgpu::ComputePipeline,
    flatten: wgpu::ComputePipeline,
    bin: wgpu::ComputePipeline,
    fragment_bins: wgpu::ComputePipeline,
    link_groups: wgpu::ComputePipeline,
    prefix: wgpu::ComputePipeline,
    integrate_edges: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    classify: wgpu::ComputePipeline,
    fill_cells: wgpu::ComputePipeline,
    emit_args: wgpu::ComputePipeline,
}

/// The frame's coverage buffers, sized exactly by the R2 bound.
struct Buffers {
    params: wgpu::Buffer,
    elements: wgpu::Buffer,
    paths: wgpu::Buffer,
    segments: wgpu::Buffer,
    coverage: wgpu::Buffer,
    args: wgpu::Buffer,
    work: wgpu::Buffer,
}

impl GpuCoverage {
    pub fn new() -> Self {
        Self {
            paths: Vec::new(),
            elements: Vec::new(),
            element_sources: Vec::new(),
            element_total: 0,
            work: Vec::new(),
            args_bases: Vec::new(),
            inst_total: 0,
            seg_total: 0,
            cov_total: 0,
            bin_total: 0,
            fragment_total: 0,
            texel_words: 0,
            row_total: 0,
            bound_bytes: 0,
            bytes: 0,
            inst_origin: 0,
            dispatched: false,
            layout: None,
            args_layout: None,
            pipes: None,
            buffers: None,
            identity: DefaultHasher::new(),
            cache: FxHashMap::default(),
            cache_bytes: 0,
            cache_epoch: 0,
        }
    }

    /// A frame's apply starts over: the pending records of a planned
    /// frame are committed from scratch (`pending.clear()` precedes
    /// the lowers that produce them, `mod.rs` commit flow).
    pub fn begin_frame(&mut self) {
        self.identity = DefaultHasher::new();
        self.cache_epoch += 1;
        self.paths.clear();
        self.elements.clear();
        self.element_sources.clear();
        self.element_total = 0;
        self.work.clear();
        self.args_bases.clear();
        self.inst_total = 0;
        self.seg_total = 0;
        self.cov_total = 0;
        self.bin_total = 0;
        self.fragment_total = 0;
        self.texel_words = 0;
        self.row_total = 0;
        self.bound_bytes = 0;
        self.inst_origin = 0;
        self.dispatched = false;
    }

    /// Whether the frame carries routed paths — the dispatch and the
    /// instance-tail reservation gate on it.
    pub const fn active(&self) -> bool {
        !self.paths.is_empty()
    }

    /// Whether the first dirty surface still needs to encode this frame's work.
    pub const fn needs_dispatch(&self) -> bool {
        self.active() && !self.dispatched
    }

    /// GPU buffer bytes held — the `Engine::memory()` contribution of
    /// scope item 8.
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Retained host input/work-list allocation capacities.
    pub fn cpu_bytes(&self) -> u64 {
        (self.paths.capacity() * std::mem::size_of::<[u32; PATH_WORDS]>()
            + self.elements.capacity() * std::mem::size_of::<GpuElement>()
            + self.element_sources.capacity() * std::mem::size_of::<Arc<[GpuElement]>>()
            + self.work.capacity() * std::mem::size_of::<[u32; 2]>()
            + self.args_bases.capacity() * std::mem::size_of::<u32>()
            + self.cache.capacity() * std::mem::size_of::<(u64, CachedCoverage)>()
            + self
                .cache
                .values()
                .map(|entry| entry.translations.capacity() * std::mem::size_of::<[f32; 2]>())
                .sum::<usize>()) as u64
    }

    /// Instance slots the kernels may emit into the buffer tail.
    pub const fn inst_total(&self) -> u32 {
        self.inst_total
    }

    /// Where kernel emission begins — `set_inst_origin`'s value.
    pub const fn inst_origin(&self) -> u32 {
        self.inst_origin
    }

    /// R8 texel words the classifier may write — sizes `cell_texels`.
    pub const fn texel_words(&self) -> u32 {
        self.texel_words
    }

    /// The indirect-argument buffer — valid after `dispatch`.
    pub const fn args_buffer(&self) -> &wgpu::Buffer {
        &self.buffers.as_ref().expect("coverage buffers").args
    }

    /// Byte offset of `path`'s three indirect argument records —
    /// `ARGS_OPAQUE`/`ARGS_PARTIAL`/`ARGS_NORMAL` further in.
    pub fn args_base(&self, path: u32) -> u64 {
        u64::from(self.args_bases[usize::try_from(path).expect("path index")]) * 4
    }

    /// Join one applied `GpuPath` record into the frame input, as the
    /// path's compute index. The bound converts to absolute buffer
    /// bases; the R2 cap rejects over it.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "device coordinates are f32"
    )]
    #[expect(
        clippy::cast_precision_loss,
        reason = "device surface dimensions fit f32"
    )]
    pub fn push(
        &mut self,
        content: u64,
        rule: u32,
        raster: Affine,
        surface: (u32, u32),
        template: &Instance,
        prepared: &GpuPath,
    ) -> Result<u32, RenderError> {
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
            + to64(bound.instance_cap) * u64::from(INSTANCE_VEC4S) * 16
            + u64::from(ARGS_WORDS) * 4
            + (PATH_WORDS as u64) * 4
            + to64(rows + strips) * 8;
        let previous_tokens = self.fragment_total.min(self.cov_total) * 6;
        let next_tokens = fragment_total.min(cov_total) * 6;
        self.bound_bytes += path_bytes
            + u64::from(next_groups - previous_groups + next_tokens - previous_tokens) * 4;
        if self.bound_bytes + 18 * 1024 + 128 + 48 > GPU_COV_MAX_BYTES {
            return Err(RenderError::Unsupported(
                "GPU coverage bound exceeds the per-frame cap",
            ));
        }
        let path_i = u32::try_from(self.paths.len()).expect("path count fits u32");
        let elem_base = self.element_total;
        let args_base =
            u32::try_from(self.args_bases.len()).expect("coverage capacity fits u32") * ARGS_WORDS;
        let inst_base = self.inst_total;
        let grid_base = self.cov_total;
        let texel_base = self.texel_words;
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
        words[14] = inst_base;
        words[15] = args_base;
        words[16] = rule;
        words[17] = u32::try_from(bound.grid_w).expect("grid fits u32");
        words[18] = u32::try_from(bound.grid_h).expect("grid fits u32");
        words[19] = u32::try_from(bound.instance_cap).expect("instance cap fits u32");
        words[21] = self.row_total;
        words[20] = u32::try_from(bound.cell_cap).expect("cell cap fits u32");
        // affine = (a, c, b, d); translate = (e, f)
        words[24] = (xx as f32).to_bits();
        words[25] = (xy as f32).to_bits();
        words[26] = (yx as f32).to_bits();
        words[27] = (yy as f32).to_bits();
        words[28] = (tx as f32).to_bits();
        words[29] = (ty as f32).to_bits();
        words[30] = (surface.0 as f32).to_bits();
        words[31] = (surface.1 as f32).to_bits();
        // The template instance's 17 vec4s, bitcast verbatim.
        let template_words: &[u32] = bytemuck::cast_slice(std::slice::from_ref(template));
        words[40..40 + 68].copy_from_slice(&template_words[..68]);

        self.hash_path(content, bound.clipped, &words);
        self.paths.push(words);
        self.element_total += u32::try_from(elements.len()).expect("elements fit u32");
        self.element_sources.push(Arc::clone(elements));
        self.args_bases.push(args_base);
        self.inst_total += u32::try_from(bound.instance_cap).expect("instances");
        self.seg_total += u32::try_from(bound.segments).expect("segments");
        self.bin_total += u32::try_from(bound.bins).expect("bins");
        self.fragment_total = fragment_total;
        self.cov_total = cov_total;
        self.texel_words +=
            u32::try_from(bound.texels.div_ceil(4) + bound.cell_cap).expect("texel words");
        for row in 0..rows {
            self.work.push([path_i, u32::try_from(row).expect("row")]);
        }
        self.row_total += u32::try_from(rows).expect("rows");

        Ok(path_i)
    }

    fn hash_path(&mut self, content: u64, clipped: bool, words: &[u32; PATH_WORDS]) {
        content.hash(&mut self.identity);
        clipped.hash(&mut self.identity);
        let mut geometry = *words;
        if !clipped {
            for word in &mut geometry[28..30] {
                let value = f32::from_bits(*word);
                *word = (value - value.floor()).to_bits();
            }
        }
        geometry[8..40].hash(&mut self.identity);
    }

    pub fn trim_cache(&mut self) {
        self.bytes -= self.cache_bytes;
        self.cache.clear();
        self.cache.shrink_to_fit();
        self.cache_bytes = 0;
    }

    /// Apply final mask patches, paint offsets and painter depth after
    /// surface placement. The anchor itself is never drawn.
    pub fn set_template(&mut self, index: u32, template: &Instance, surface_base: u32) {
        let mut template = *template;
        template.params[0] = f32::from_bits(template.params[0].to_bits() + surface_base);
        let words: &[u32] = bytemuck::cast_slice(std::slice::from_ref(&template));
        self.paths[index as usize][40..108].copy_from_slice(words);
    }

    /// Finalize bases once the frame's CPU instance total is known —
    /// `inst_origin` is where kernel emission begins.
    pub const fn set_inst_origin(&mut self, inst_origin: u32) {
        self.inst_origin = inst_origin;
    }

    /// Reject device limits before allocating the generated raster output.
    pub fn validate_output_sizes(&self, limits: &wgpu::Limits) -> Result<(), RenderError> {
        let instances = (u64::from(self.inst_origin) + u64::from(self.inst_total)) * 272;
        let texels = u64::from(self.texel_words) * 4;
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        if instances > limit || texels > limit {
            return Err(RenderError::Unsupported(
                "GPU coverage output exceeds device buffer limit",
            ));
        }
        // Raster attributes represent the word offset as f32; every integer
        // address must be exact. The frame cap normally imposes a tighter bound.
        if self.texel_words > 1 << 24 {
            return Err(RenderError::Unsupported(
                "GPU coverage texel address exceeds exact f32 range",
            ));
        }
        Ok(())
    }

    /// Record the dispatch set on the encoder — coverage clearing,
    /// then the sparse coverage kernels, batching every routed path of the frame
    /// (decision S2). Idempotent per frame.
    #[expect(
        clippy::too_many_lines,
        clippy::too_many_arguments,
        reason = "one ordered compute dispatch sequence and its output resources"
    )]
    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        instances: &wgpu::Buffer,
        cell_texels: &wgpu::Buffer,
        delivery: crate::render::shaders::ShaderDelivery,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> Result<(), RenderError> {
        if self.dispatched || !self.active() {
            return Ok(());
        }
        self.dispatched = true;
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
        let strip_total =
            u32::try_from(self.work.len()).expect("coverage capacity fits u32") - self.row_total;
        let bin_base = 2 + self.row_total;
        let fragment_base = bin_base + self.bin_total * 3;
        let sort_a = fragment_base + self.fragment_total * 8;
        let groups = sort_a + self.fragment_total * 2;
        let sort_b = groups + group_words(self.fragment_total, self.cov_total);
        let histogram = sort_b + self.fragment_total * 2;
        let histogram_end =
            histogram + self.fragment_total.div_ceil(WORKGROUP).div_ceil(256) * 256 * 16;
        // Classification starts after radix sorting has released sort_b and
        // its histogram. sort_a stays live through texel generation.
        let tokens = sort_b;
        let token_counter =
            tokens + (self.fragment_total.min(self.cov_total) * 2 + strip_total) * 3;
        let params: [u32; PARAMS_WORDS] = [
            self.element_total,
            self.row_total,
            strip_total,
            u32::try_from(self.paths.len()).expect("coverage capacity fits u32"),
            self.inst_origin,
            1,
            self.row_total,
            self.cov_total,
            bin_base,
            self.bin_total,
            fragment_base,
            self.fragment_total,
            sort_a,
            sort_b,
            histogram,
            groups,
            tokens,
            token_counter,
            0,
            self.texel_words,
        ];
        let cov_words = u64::from((token_counter + 1).max(histogram_end));
        let scratch_bytes = cov_words * 4;
        let limits = device.limits();
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let sizes = [
            scratch_bytes,
            u64::from(self.element_total) * 48,
            self.paths.len() as u64 * PATH_WORDS as u64 * 4,
            u64::from(self.seg_total) * 16,
            u64::from(ARGS_WORDS) * 4 * self.paths.len() as u64 + 48,
            self.work.len() as u64 * 8,
        ];
        if sizes.into_iter().any(|size| size > limit) {
            return Err(RenderError::Unsupported(
                "GPU coverage input exceeds device buffer limit",
            ));
        }

        let seg_words = u64::from(self.seg_total).max(1);
        let work_words = u64::try_from(self.work.len()).expect("work").max(1);
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
                u64::from(ARGS_WORDS) * 4 * (self.paths.len() as u64).max(1) + 48,
                storage | wgpu::BufferUsages::INDIRECT,
            ),
            work: sized_buffer(
                device,
                previous.map(|b| &b.work),
                "cov work",
                work_words * 8,
                storage,
            ),
        };
        self.bytes = buffers.params.size()
            + buffers.elements.size()
            + buffers.paths.size()
            + buffers.segments.size()
            + buffers.coverage.size()
            + buffers.args.size()
            + buffers.work.size();

        let key = self.identity.finish();
        let cached = self.cache.get_mut(&key).map(|entry| {
            entry.last_used = self.cache_epoch;
            entry.clone()
        });
        if let Some(entry) = &cached {
            for (path, old) in self.paths.iter_mut().zip(&entry.translations) {
                path[22] = (f32::from_bits(path[28]) - old[0]).to_bits();
                path[23] = (f32::from_bits(path[29]) - old[1]).to_bits();
            }
        }
        let geometry_bytes = self.paths.len() as u64 * 16 + u64::from(self.inst_total) * 32;
        let texel_bytes = u64::from(self.texel_words) * 4;
        let candidate = if cached.is_none() && geometry_bytes + texel_bytes <= COVERAGE_CACHE_BYTES
        {
            while self.cache_bytes + geometry_bytes + texel_bytes > COVERAGE_CACHE_BYTES {
                let oldest = *self
                    .cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .expect("cache is nonempty")
                    .0;
                self.cache_bytes -= self.cache.remove(&oldest).expect("oldest entry").bytes();
            }
            let entry = CachedCoverage {
                geometry: sized_buffer(
                    device,
                    None,
                    "cached coverage geometry",
                    geometry_bytes,
                    storage,
                ),
                texels: sized_buffer(device, None, "cached coverage texels", texel_bytes, storage),
                translations: self
                    .paths
                    .iter()
                    .map(|path| [f32::from_bits(path[28]), f32::from_bits(path[29])])
                    .collect(),
                last_used: self.cache_epoch,
            };
            encoder.clear_buffer(&entry.geometry, 0, Some(self.paths.len() as u64 * 16));
            Some(entry)
        } else {
            None
        };
        let radix_pass_count = radix_passes(self.cov_total);
        for radix_pass in 0..radix_pass_count {
            let mut block = params;
            block[18] = radix_pass;
            queue.write_buffer(
                &buffers.params,
                u64::from(radix_pass) * 256,
                bytemuck::cast_slice(&block),
            );
        }
        if cached.is_none() {
            for (index, source) in self.element_sources.iter().enumerate() {
                let path_i = u32::try_from(index).expect("path index");
                self.elements
                    .extend(source.iter().copied().map(|mut element| {
                        element.head[2] = path_i;
                        element
                    }));
            }
            queue.write_buffer(&buffers.elements, 0, bytemuck::cast_slice(&self.elements));
            queue.write_buffer(&buffers.work, 0, bytemuck::cast_slice(&self.work));
        }
        let path_bytes: Vec<u32> = self.paths.iter().flatten().copied().collect();
        queue.write_buffer(&buffers.paths, 0, bytemuck::cast_slice(&path_bytes));

        encoder.clear_buffer(&buffers.coverage, 0, Some(u64::from(bin_base) * 4));

        encoder.clear_buffer(&buffers.coverage, u64::from(token_counter) * 4, Some(4));

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
                resource: cell_texels.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: buffers.args.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: buffers.work.as_entire_binding(),
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
        let cache_bind = cached.as_ref().or(candidate.as_ref()).map(|entry| {
            let mut cached_entries = entries.clone();
            cached_entries[3].resource = entry.geometry.as_entire_binding();
            cached_entries[4].resource = entry.texels.as_entire_binding();
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cached coverage"),
                layout,
                entries: &cached_entries
                    .iter()
                    .filter(|entry| entry.binding != 7)
                    .cloned()
                    .collect::<Vec<_>>(),
            })
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
            if cached.is_some() {
                pass.set_bind_group(0, cache_bind.as_ref().expect("cache binding"), &[0]);
                pass.set_pipeline(&pipes.replay_cache);
                dispatch(&mut pass, self.inst_total);
                pass.set_pipeline(&pipes.replay_texels);
                dispatch(&mut pass, self.texel_words);
            } else {
                pass.set_bind_group(0, &bind, &[0]);
                pass.set_pipeline(&pipes.flatten);
                dispatch(&mut pass, params[0]);
                pass.set_pipeline(&pipes.bin);
                dispatch(&mut pass, params[0]);
                let dispatch_base = self.paths.len() as u64 * u64::from(ARGS_WORDS) * 4;
                pass.set_bind_group(0, &args_bind, &[0]);
                pass.set_pipeline(&pipes.prepare_dispatch);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_bind_group(0, &bind, &[0]);
                pass.set_pipeline(&pipes.fragment_bins);
                pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base);
                pass.set_bind_group(0, &args_bind, &[0]);
                pass.set_pipeline(&pipes.prepare_dispatch);
                pass.dispatch_workgroups(1, 1, 1);
                for radix_pass in 0..radix_pass_count {
                    pass.set_bind_group(0, &bind, &[radix_pass * 256]);
                    pass.set_pipeline(&pipes.radix_count);
                    pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 12);
                    pass.set_pipeline(&pipes.radix_scan_blocks);
                    pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 24);
                    pass.set_pipeline(&pipes.radix_prefix);
                    pass.dispatch_workgroups(1, 1, 1);
                    pass.set_pipeline(&pipes.radix_scatter);
                    pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 12);
                }
                pass.set_bind_group(0, &bind, &[0]);
                pass.set_pipeline(&pipes.link_groups);
                pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 12);
                pass.set_pipeline(&pipes.prefix);
                dispatch(&mut pass, strip_total);
                pass.set_pipeline(&pipes.integrate_edges);
                pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 12);
                pass.set_pipeline(&pipes.integrate);
                pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 12);
                pass.set_pipeline(&pipes.classify);
                dispatch(&mut pass, strip_total);
                pass.set_bind_group(0, &args_bind, &[0]);
                pass.set_pipeline(&pipes.prepare_cells);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_bind_group(0, &bind, &[0]);
                pass.set_pipeline(&pipes.fill_cells);
                pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 36);
                if candidate.is_some() {
                    pass.set_bind_group(0, cache_bind.as_ref().expect("cache binding"), &[0]);
                    pass.set_pipeline(&pipes.store_cache);
                    pass.dispatch_workgroups_indirect(&buffers.args, dispatch_base + 36);
                }
            }
            pass.set_bind_group(0, &args_bind, &[0]);
            pass.set_pipeline(&pipes.emit_args);
            dispatch(
                &mut pass,
                u32::try_from(self.paths.len()).expect("coverage capacity fits u32"),
            );
        }
        if let Some(entry) = candidate {
            self.cache_bytes += entry.bytes();
            self.cache.insert(key, entry);
        }
        self.bytes += self.cache_bytes;
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
                layout: Some(
                    if matches!(entry, "prepare_dispatch" | "prepare_cells" | "emit_args") {
                        &args_pipe_layout
                    } else {
                        &pipe_layout
                    },
                ),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        self.pipes = Some(Pipes {
            prepare_dispatch: make("prepare_dispatch"),
            prepare_cells: make("prepare_cells"),
            store_cache: make("store_cache"),
            replay_cache: make("replay_cache"),
            replay_texels: make("replay_texels"),
            radix_count: make("radix_count"),
            radix_scan_blocks: make("radix_scan_blocks"),
            radix_prefix: make("radix_prefix"),
            radix_scatter: make("radix_scatter"),
            flatten: make("flatten"),
            bin: make("bin"),
            fragment_bins: make("fragment_bins"),
            link_groups: make("link_groups"),
            prefix: make("prefix"),
            integrate_edges: make("integrate_edges"),
            integrate: make("integrate"),
            classify: make("classify"),
            fill_cells: make("fill_cells"),
            emit_args: make("emit_args"),
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
/// Passes ping-pong between the sort buffers and the sorted keys must end
/// in `sort_a`, so the count is even.
const fn radix_passes(key_end: u32) -> u32 {
    let digits = key_end.bit_width().div_ceil(4);
    digits.div_ceil(2) * 2
}

fn sized_buffer(
    device: &wgpu::Device,
    previous: Option<&wgpu::Buffer>,
    label: &'static str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    if let Some(buffer) = previous.filter(|buffer| buffer.size() == size) {
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
        assert_eq!(radix_passes(256), 4);
        assert_eq!(radix_passes(1 << 16), 6);
        assert_eq!(radix_passes(1 << 24), 8);
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
        let mut frame = GpuCoverage::new();
        frame
            .push(
                0,
                0,
                Affine::IDENTITY,
                (4096, 4096),
                &Instance::new(0),
                &large_path,
            )
            .expect("sparse 4096-square capacity");
        assert!(frame.bound_bytes < GPU_COV_MAX_BYTES);
        let huge_path = rectangle(1_000_000);
        let before = frame.paths.len();
        assert!(
            frame
                .push(
                    0,
                    0,
                    Affine::IDENTITY,
                    (1_000_000, 1_000_000),
                    &Instance::new(0),
                    &huge_path
                )
                .is_err()
        );
        assert_eq!(frame.paths.len(), before, "reject before appending a path");
    }
}
