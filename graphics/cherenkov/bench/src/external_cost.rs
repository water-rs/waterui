//! `external-cost`: issue #168's hand-off A/B — the engine's retained
//! external-frame path (`--path e`) against copy-and-convert
//! (`--path c`), over identical produced frames.
//!
//! Both paths share one producer: a ring of platform video buffers —
//! `CVPixelBuffer` on Apple, `AHardwareBuffer` on Android — filled on the
//! CPU with a drifting gradient once per frame. Path `e` installs that
//! buffer as a retained [`ExternalFrame`]. On Apple the planes are
//! integer textures and the engine runs `ext_frame_yuv`. On Android a
//! YUV `AHardwareBuffer` may import as an external format instead, and
//! then the driver's YCbCr sampler decodes it — that frame never enters
//! `ext_frame_yuv`. The report's `import_form` records which one
//! happened.
//!
//! Path `c` copies the planes on the CPU into a staging buffer, then
//! one GPU submission does `copy_buffer_to_texture` of both planes and
//! a [`GpuContent`] draw. The draw's shader is
//! [`DECODE_WGSL`](cherenkov_gpu::bench::DECODE_WGSL) plus
//! `external_convert.wgsl`, and its uniform is [`yuv_frame_params`] —
//! the same bake the engine writes for an installed YUV frame.
//! `external_convert.wgsl` is the bench's fullscreen caller of
//! `ext_frame_yuv`.
//!
//! Timing: the engine's own GPU timestamps cover the composite
//! submission. Path `c`'s handoff is only the GPU work inside that one
//! submission — a marker-pass end stamp, the two copies, then the
//! convert pass — multiplied by the queue's timestamp period. A missing
//! stamp drops the frame from the percentiles and increments
//! `dropped_frames`; it is never stored as zero.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::bench::{YuvFrameParams, YuvLayout, yuv_frame_params};
use cherenkov_gpu::interop::{
    ExternalFrame, FrameColor, GpuContent, GpuContentBox, SharedDevice, Transfer, wgpu,
};
use cherenkov_gpu::{Gpu, GpuConfig};

use crate::cli::{ExternalCostArgs, ExternalPath, ExternalSize, ExternalTransfer};
use crate::memory::{
    AdapterMemory, EngineBytes, MemoryReport, MemorySnapshot, Reading, SampleDetail,
    wgpu_allocator, wgpu_vk_memory_budget,
};
use crate::motion::Clock;
use crate::report::{Conditions, EnergyReport, Pacing, percentiles};
use crate::timing::Timings;
use crate::{BenchError, DeviceInfo, PhaseSample, affinity, conditions, energy};

/// Producer ring depth — a decode queue cycles a few buffers ahead.
const RING: usize = 3;

/// How long a slot wait or a staging map may block.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);

/// The offscreen target every run and the correctness pair render.
const TARGET: OffscreenFormat = OffscreenFormat::LinearF16;

/// Codes of ramp drift in the generated pattern: the window the
/// frame/row offset slides over. Only the platform producers read it.
#[cfg(any(target_vendor = "apple", target_os = "android"))]
const SHIFT: usize = 256;

/// wgpu name of an [`OffscreenFormat`], the string `DeviceInfo` records.
const fn target_format_name(format: OffscreenFormat) -> &'static str {
    match format {
        OffscreenFormat::LinearF16 => "Rgba16Float",
        OffscreenFormat::LinearF32 => "Rgba32Float",
    }
}

/// `bytes_per_row` alignment `copy_buffer_to_texture` requires.
const fn aligned_pitch(tight: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    tight.div_ceil(align) * align
}

/// What one run measures.
#[derive(Clone, Copy)]
struct Spec {
    width: u32,
    height: u32,
    /// 8 for NV12 (SDR), 10 for P010 (PQ).
    bits: u32,
    color: FrameColor,
}

impl Spec {
    const fn new(size: ExternalSize, transfer: ExternalTransfer) -> Self {
        let (width, height) = match size {
            ExternalSize::P1080 => (1920, 1080),
            ExternalSize::P4k => (3840, 2160),
        };
        let (bits, color) = match transfer {
            ExternalTransfer::Sdr => (8, FrameColor::BT709_VIDEO),
            ExternalTransfer::Pq => (10, FrameColor::BT2020_PQ),
        };
        Self {
            width,
            height,
            bits,
            color,
        }
    }

    /// Bytes per stored code: 1 for NV12, 2 for P010's 16-bit words.
    const fn code_bytes(self) -> usize {
        if self.bits == 8 { 1 } else { 2 }
    }

    /// `wgpu` formats of the luma and interleaved-chroma planes — the
    /// [`ExternalFrame::yuv`] contract (path `e`) and the copy targets
    /// (path `c`).
    const fn formats(self) -> (wgpu::TextureFormat, wgpu::TextureFormat) {
        if self.bits == 8 {
            (wgpu::TextureFormat::R8Uint, wgpu::TextureFormat::Rg8Uint)
        } else {
            (wgpu::TextureFormat::R16Uint, wgpu::TextureFormat::Rg16Uint)
        }
    }

    const fn layout(self) -> &'static str {
        if self.bits == 8 { "nv12" } else { "p010" }
    }

    const fn transfer_name(self) -> &'static str {
        match self.color.transfer {
            Transfer::Bt709 => "bt709-sdr",
            _ => "bt2020-pq",
        }
    }
}

/// Tight plane rows and the 256-byte pitches the staging copy uses.
#[derive(Clone, Copy)]
struct PlaneLayout {
    width: u32,
    height: u32,
    luma_stride: u32,
    /// Byte offset of the chroma plane in a staging buffer. A multiple
    /// of [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`].
    luma_bytes: u64,
    chroma_stride: u32,
    chroma_height: u32,
}

impl PlaneLayout {
    fn new(spec: &Spec) -> Self {
        let code = u32::try_from(spec.code_bytes()).expect("code bytes fit u32");
        let luma_stride = aligned_pitch(spec.width * code);
        let chroma_stride = aligned_pitch(spec.width.div_ceil(2) * 2 * code);
        let chroma_height = spec.height.div_ceil(2);
        Self {
            width: spec.width,
            height: spec.height,
            luma_stride,
            luma_bytes: u64::from(luma_stride) * u64::from(spec.height),
            chroma_stride,
            chroma_height,
        }
    }

    fn staging_size(self) -> u64 {
        self.luma_bytes + u64::from(self.chroma_stride) * u64::from(self.chroma_height)
    }
}

/// The spec's decode uniform — [`yuv_frame_params`], the same bake the
/// engine writes for an installed YUV frame.
fn frame_params(spec: &Spec) -> YuvFrameParams {
    let layout = if spec.bits == 8 {
        YuvLayout::Nv12
    } else {
        YuvLayout::P010
    };
    yuv_frame_params(&spec.color, layout, (spec.width, spec.height))
}

/// The synthetic frame pattern: a luma ramp and an interleaved chroma
/// ramp that drift by `frame` and `row` — valid video-range codes,
/// non-constant, identical for both paths.
#[cfg(any(target_vendor = "apple", target_os = "android"))]
struct Ramps {
    /// `(w + SHIFT)` luma codes of `bytes` each.
    luma: Vec<u8>,
    /// `(w / 2 + SHIFT)` interleaved `(cb, cr)` pairs of `bytes` each.
    chroma: Vec<u8>,
    /// Bytes per stored code (1 NV12, 2 P010).
    bytes: usize,
    /// Row width in luma codes.
    width: usize,
}

#[cfg(any(target_vendor = "apple", target_os = "android"))]
impl Ramps {
    /// Video-range code ramps: luma sweeps `16..=235` (8-bit) or
    /// `64..=940` (10-bit `<<6`) left to right; the chroma pair steps
    /// through its range at a different rate so the picture varies in
    /// both planes.
    fn new(spec: &Spec) -> Self {
        let width = spec.width as usize;
        let bytes = spec.code_bytes();
        let mut luma = Vec::with_capacity((width + SHIFT) * bytes);
        let mut chroma = Vec::with_capacity((width / 2 + SHIFT) * 2 * bytes);
        for i in 0..(width + SHIFT) {
            let code = if spec.bits == 8 {
                16 + u32::try_from(i * 219 / (width + SHIFT)).expect("luma code")
            } else {
                (64 + u32::try_from(i * 876 / (width + SHIFT)).expect("luma code")) << 6
            };
            luma.extend_from_slice(&code.to_le_bytes()[..bytes]);
        }
        for i in 0..(width / 2 + SHIFT) {
            for component in 0..2 {
                let code = if spec.bits == 8 {
                    16 + u32::try_from((i * 7 + component * 113) % 224).expect("chroma code")
                } else {
                    (64 + u32::try_from((i * 7 + component * 449) % 896).expect("chroma code")) << 6
                };
                chroma.extend_from_slice(&code.to_le_bytes()[..bytes]);
            }
        }
        Self {
            luma,
            chroma,
            bytes,
            width,
        }
    }

    /// Writes the `row`'th luma row of `frame` into `dst`
    /// (`width * bytes` long).
    fn luma_row(&self, frame: u32, row: u32, dst: &mut [u8]) {
        let row_bytes = self.width * self.bytes;
        let off = (row.wrapping_mul(3).wrapping_add(frame.wrapping_mul(11)) as usize % SHIFT)
            * self.bytes;
        dst[..row_bytes].copy_from_slice(&self.luma[off..off + row_bytes]);
    }

    /// Writes the `row`'th chroma row of `frame` into `dst`
    /// (`width * bytes` long — `width / 2` interleaved pairs, which is
    /// the same byte count as a luma row when the width is even).
    fn chroma_row(&self, frame: u32, row: u32, dst: &mut [u8]) {
        let row_bytes = (self.width / 2) * self.bytes * 2;
        let pair = self.bytes * 2;
        let off =
            (row.wrapping_mul(5).wrapping_add(frame.wrapping_mul(13)) as usize % SHIFT) * pair;
        dst[..row_bytes].copy_from_slice(&self.chroma[off..off + row_bytes]);
    }
}

/// This crate's fullscreen convert pass, which calls `ext_frame_yuv`
/// from [`cherenkov_gpu::bench::DECODE_WGSL`]. Path `c` runs that
/// integer-plane decode over the copied planes. Path `e` on Apple does
/// too; an Android external-format import does not.
const CONVERT_WGSL: &str = include_str!("external_convert.wgsl");

/// The convert pipeline's whole module: the engine's decode, then
/// [`CONVERT_WGSL`].
fn convert_source() -> String {
    [cherenkov_gpu::bench::DECODE_WGSL, CONVERT_WGSL].concat()
}

/// Queries per set: the largest multiple of 3 that fits in one set.
const fn query_span() -> u32 {
    (wgpu::QUERY_SET_MAX_QUERIES / 3) * 3
}

/// `(chunk, index)` of frame `frame`'s three stamps.
fn locate(frame: u32, span: u32) -> (usize, u32) {
    let q = frame * 3;
    (
        usize::try_from(q / span).expect("query chunk fits usize"),
        q % span,
    )
}

/// One timestamp-query allocation. A long run exceeds
/// [`wgpu::QUERY_SET_MAX_QUERIES`], so the stamps are split into chunks.
struct QueryChunk {
    set: Arc<wgpu::QuerySet>,
    count: u32,
}

fn query_chunks(device: &wgpu::Device, query_count: u32) -> Vec<QueryChunk> {
    let span = query_span();
    let mut left = query_count;
    let mut chunks = Vec::new();
    while left > 0 {
        let count = left.min(span);
        let set = Arc::new(device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("external-cost handoff stamps"),
            ty: wgpu::QueryType::Timestamp,
            count,
        }));
        chunks.push(QueryChunk { set, count });
        left -= count;
    }
    chunks
}

/// The `GpuContent` producer of path `c`: one fullscreen triangle
/// sampling the copied planes through the engine's decode into the
/// layer's working-space attachment.
struct Convert {
    y: wgpu::Texture,
    uv: wgpu::Texture,
    y_view: wgpu::TextureView,
    uv_view: wgpu::TextureView,
    /// The baked `ExtParams` uniform — `external.wgsl`'s group-1
    /// binding 4.
    params: wgpu::Buffer,
    /// Ring of `MAP_WRITE | COPY_SRC` plane buffers. Slot `f % RING`
    /// holds the planes `stage()` wrote for this render's own counter.
    staging: [wgpu::Buffer; RING],
    layout: PlaneLayout,
    /// One entry per query chunk, in order.
    queries: Vec<Arc<wgpu::QuerySet>>,
    query_span: u32,
    /// The 1×1 renderable the marker pass clears.
    marker: wgpu::TextureView,
    /// Render calls. Independent of the pattern frame `composite_frame`
    /// asks the producer to draw: this counter starts at 0, so a
    /// one-frame composite writes stamps 0, 1 and 2 into a set of 3.
    frame: u32,
    /// Built on the render thread in `setup`.
    live: Option<Live>,
}

/// What `setup` builds: the convert pipeline and its bound planes.
struct Live {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
}

impl GpuContent for Convert {
    fn setup(&mut self, ctx: &wgpu::Context<'_>) -> impl Future<Output = ()> {
        let u32_tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("external-cost convert planes"),
                entries: &[
                    u32_tex(0),
                    u32_tex(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                                YuvFrameParams,
                            >()
                                as u64),
                        },
                        count: None,
                    },
                ],
            });
        let empty = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("external-cost empty group"),
                entries: &[],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("external-cost convert layout"),
                bind_group_layouts: &[Some(&empty), Some(&layout)],
                immediate_size: 0,
            });
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("external-cost convert"),
                source: wgpu::ShaderSource::Wgsl(convert_source().into()),
            });
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("external-cost convert"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_convert"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_convert"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("external-cost convert planes"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        self.live = Some(Live { pipeline, bind });
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut wgpu::Frame<'_>) {
        let Live { pipeline, bind } = self.live.as_ref().expect("setup ran before render");
        let f = self.frame;
        self.frame += 1;
        let (chunk, base) = locate(f, self.query_span);
        let queries = &self.queries[chunk];
        let staging = &self.staging[f as usize % RING];
        let layout = self.layout;
        // One submission. The marker's end stamp, the two plane copies
        // and the convert pass share an encoder, so the interval is GPU
        // work only. Draining the queue before a separate marker would
        // still time the CPU gap (`write_texture`, lowering) and would
        // let a tiled GPU run that marker beside frame f−1.
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("external-cost convert"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("external-cost stamp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.marker,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: queries,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(base),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let copy = |encoder: &mut wgpu::CommandEncoder, offset, stride, rows, texture, width| {
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(rows),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
        };
        copy(
            &mut encoder,
            0,
            layout.luma_stride,
            layout.height,
            &self.y,
            layout.width,
        );
        copy(
            &mut encoder,
            layout.luma_bytes,
            layout.chroma_stride,
            layout.chroma_height,
            &self.uv,
            layout.width.div_ceil(2),
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("external-cost convert"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: queries,
                    beginning_of_pass_write_index: Some(base + 1),
                    end_of_pass_write_index: Some(base + 2),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        frame.queue.submit([encoder.finish()]);
        // The producer draws every frame: a new frame's planes arrive
        // each interval, so the content is never settled.
        frame.request_redraw();
    }
}

/// Path `c`'s host-side state: the copy targets, the staging ring and
/// the stamp resolution buffers.
struct CopyPath {
    y: wgpu::Texture,
    uv: wgpu::Texture,
    /// 1×1 clear target. Four bytes, counted in [`Self::gpu_bytes`].
    marker_tex: wgpu::Texture,
    marker: wgpu::TextureView,
    staging: [wgpu::Buffer; RING],
    layout: PlaneLayout,
    chunks: Vec<QueryChunk>,
    query_span: u32,
    /// Reused across chunks. Large enough for the biggest chunk.
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    params: wgpu::Buffer,
    /// Staging slot of the next [`Self::stage`]. Starts at 0, as
    /// [`Convert::frame`] does, and both advance once per frame.
    cursor: Cell<u32>,
}

impl CopyPath {
    fn new(shared: &SharedDevice, spec: &Spec, query_count: u32) -> Result<Self, BenchError> {
        let device = &shared.device;
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Err(BenchError::Gpu(
                "external-cost --path c needs TIMESTAMP_QUERY for the handoff stamps".into(),
            ));
        }
        let (y_format, uv_format) = spec.formats();
        let layout = PlaneLayout::new(spec);
        let plane_texture = |name, width, height, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(name),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let y = plane_texture(
            "external-cost copied luma",
            spec.width,
            spec.height,
            y_format,
        );
        let uv = plane_texture(
            "external-cost copied chroma",
            spec.width.div_ceil(2),
            spec.height.div_ceil(2),
            uv_format,
        );
        let marker_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("external-cost stamp marker"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let marker = marker_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let staging = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("external-cost plane staging"),
                size: layout.staging_size(),
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        });
        let chunks = query_chunks(device, query_count);
        let resolve_queries = chunks.iter().map(|chunk| chunk.count).max().unwrap_or(0);
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("external-cost stamp resolve"),
            size: u64::from(resolve_queries) * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("external-cost stamp readback"),
            size: u64::from(resolve_queries) * 8,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("external-cost convert params"),
            size: std::mem::size_of::<YuvFrameParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        shared
            .queue
            .write_buffer(&params, 0, bytemuck::bytes_of(&frame_params(spec)));
        Ok(Self {
            y,
            uv,
            marker_tex,
            marker,
            staging,
            layout,
            chunks,
            query_span: query_span(),
            resolve,
            readback,
            params,
            cursor: Cell::new(0),
        })
    }

    /// The [`Convert`] installed once for the run. Its frame counter
    /// starts at 0, matching [`Self::cursor`].
    fn converter(&self) -> Convert {
        Convert {
            y: self.y.clone(),
            uv: self.uv.clone(),
            y_view: self.y.create_view(&wgpu::TextureViewDescriptor::default()),
            uv_view: self.uv.create_view(&wgpu::TextureViewDescriptor::default()),
            params: self.params.clone(),
            staging: self.staging.clone(),
            layout: self.layout,
            queries: self
                .chunks
                .iter()
                .map(|chunk| Arc::clone(&chunk.set))
                .collect(),
            query_span: self.query_span,
            marker: self.marker.clone(),
            frame: 0,
            live: None,
        }
    }

    /// Copies pattern `pattern` into staging slot `cursor % RING`.
    ///
    /// The buffer is `MAP_WRITE`: the bytes the GPU copies are the ones
    /// written here, not a `write_buffer` that would itself be an
    /// unmeasured upload. Padding past each tight row is zero.
    fn stage(
        &self,
        producer: &platform::Producer,
        pattern: u32,
        shared: &SharedDevice,
    ) -> Result<(), BenchError> {
        let slot = self.cursor.get() as usize % RING;
        let buf = &self.staging[slot];
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..)
            .map_async(wgpu::MapMode::Write, move |result| {
                let _ = tx.send(result);
            });
        let start = Instant::now();
        loop {
            // `Poll`, not `Wait`: a wait with no submission index blocks
            // on the newest submit, which is frame f−1, and that would
            // fold the previous frame's GPU time into this map.
            shared
                .device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| BenchError::Gpu(format!("external-cost staging map poll: {e}")))?;
            match rx.try_recv() {
                Ok(Ok(())) => break,
                Ok(Err(e)) => {
                    return Err(BenchError::Gpu(format!("external-cost staging map: {e}")));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if start.elapsed() >= COMPLETION_TIMEOUT {
                        return Err(BenchError::Gpu(
                            "external-cost staging map timed out".into(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(BenchError::Gpu(
                        "external-cost staging map callback dropped".into(),
                    ));
                }
            }
        }
        let copied = (|| {
            let mut view = buf
                .slice(..)
                .get_mapped_range_mut()
                .map_err(|e| BenchError::Gpu(format!("external-cost staging range: {e}")))?;
            // Mapped memory is write-only. Zero the padding, then the
            // producer copies each tight row through the same view.
            view.slice(..).fill(0);
            producer.copy_planes(
                pattern,
                &mut view,
                self.layout.luma_stride as usize,
                usize::try_from(self.layout.luma_bytes).expect("chroma offset fits usize"),
                self.layout.chroma_stride as usize,
            )
        })();
        buf.unmap();
        copied?;
        self.cursor.set(self.cursor.get() + 1);
        Ok(())
    }

    /// Resolves every chunk and returns per measured frame
    /// `(handoff, convert)` seconds. `None` where `b <= a`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a tick delta of one timed frame fits the f64 mantissa"
    )]
    fn read_stamps(
        &self,
        shared: &SharedDevice,
        warmup: u32,
        frames: u32,
    ) -> Result<Vec<StampPair>, BenchError> {
        let mut ticks = Vec::new();
        for chunk in &self.chunks {
            let count = chunk.count;
            let mut encoder =
                shared
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("external-cost stamp resolve"),
                    });
            encoder.resolve_query_set(&chunk.set, 0..count, &self.resolve, 0);
            encoder.copy_buffer_to_buffer(
                &self.resolve,
                0,
                &self.readback,
                0,
                u64::from(count) * 8,
            );
            let submission = shared.queue.submit([encoder.finish()]);
            let (send, recv) = std::sync::mpsc::channel();
            self.readback.slice(..u64::from(count) * 8).map_async(
                wgpu::MapMode::Read,
                move |result| {
                    let _ = send.send(result);
                },
            );
            shared
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(COMPLETION_TIMEOUT),
                })
                .map_err(|e| BenchError::Gpu(format!("external-cost stamp wait: {e}")))?;
            recv.recv()
                .map_err(|e| BenchError::Gpu(format!("external-cost stamp readback: {e}")))?
                .map_err(|e| BenchError::Gpu(format!("external-cost stamp map: {e}")))?;
            {
                let data = self
                    .readback
                    .slice(..u64::from(count) * 8)
                    .get_mapped_range()
                    .map_err(|e| BenchError::Gpu(format!("external-cost stamp range: {e}")))?;
                ticks.extend(
                    data.as_chunks::<8>()
                        .0
                        .iter()
                        .map(|bytes| u64::from_le_bytes(*bytes)),
                );
            }
            self.readback.unmap();
        }
        let scale = f64::from(shared.queue.get_timestamp_period()) * 1.0e-9;
        let seconds = |a: u64, b: u64| (b > a).then_some((b - a) as f64 * scale);
        Ok((0..frames)
            .map(|i| {
                let f = (warmup + i) as usize;
                let (open, begin, end) = (ticks[3 * f], ticks[3 * f + 1], ticks[3 * f + 2]);
                (seconds(open, end), seconds(begin, end))
            })
            .collect())
    }

    /// Path `c`'s own GPU bytes: copy textures, the staging ring, the
    /// uniform, the query resolve and readback, and the 1×1 marker.
    fn gpu_bytes(&self, spec: &Spec) -> u64 {
        let code = u64::try_from(spec.code_bytes()).expect("code bytes fit u64");
        let y = self.y.size();
        let uv = self.uv.size();
        let marker = self.marker_tex.size();
        let y_bytes =
            u64::from(y.width) * u64::from(y.height) * u64::from(y.depth_or_array_layers) * code;
        let uv_bytes = u64::from(uv.width)
            * u64::from(uv.height)
            * u64::from(uv.depth_or_array_layers)
            * code
            * 2;
        let marker_bytes = u64::from(marker.width)
            * u64::from(marker.height)
            * u64::from(marker.depth_or_array_layers)
            * 4;
        let buffers = self.staging.iter().map(wgpu::Buffer::size).sum::<u64>()
            + self.params.size()
            + self.resolve.size()
            + self.readback.size();
        y_bytes + uv_bytes + marker_bytes + buffers
    }
}

/// One frame's resolved path-`c` stamps: `(handoff, convert)` seconds.
type StampPair = (Option<f64>, Option<f64>);

/// Per-frame sample of the report.
#[derive(Clone, serde::Serialize)]
struct CostSample {
    /// Host-side seconds: the producer fill plus, on path `c`, the
    /// staging-buffer map and plane copy.
    encode_seconds: f64,
    /// `Engine::render` wall seconds.
    submit_seconds: f64,
    /// Engine composite GPU seconds. `null` where the adapter wrote none.
    gpu_seconds: Option<f64>,
    /// Path `c` only: GPU seconds from the marker's end stamp to the
    /// convert pass's end — the plane copies and the conversion, in
    /// one submission. `null` when a stamp is missing.
    handoff_seconds: Option<f64>,
    /// Path `c` only: GPU seconds of the convert pass alone.
    convert_seconds: Option<f64>,
    /// Render-thread CPU phases of the engine frame.
    phases: Vec<PhaseSample>,
}

/// One `external-cost` report.
#[derive(serde::Serialize)]
struct CostReport {
    adapter: String,
    backend: String,
    driver: String,
    driver_info: String,
    /// `external` (retained planes sampled in place) or `copy-convert`.
    path: &'static str,
    /// `nv12` or `p010`.
    layout: &'static str,
    /// `bt709-sdr` or `bt2020-pq`.
    transfer: &'static str,
    width: u32,
    height: u32,
    warmup_frames: u32,
    measured_frames: u32,
    /// Measured frames whose required GPU stamps were missing. Excluded
    /// from the GPU percentiles, never written as zero.
    dropped_frames: u32,
    /// One sample per measured frame, including dropped ones.
    samples: Vec<CostSample>,
    /// `gpu_seconds` percentiles `[p50, p90, p99]`.
    composite_seconds: Option<[f64; 3]>,
    /// Path `c`'s `handoff_seconds` percentiles.
    handoff_seconds: Option<[f64; 3]>,
    /// Path `c`'s `convert_seconds` percentiles.
    convert_seconds: Option<[f64; 3]>,
    /// The whole per-frame GPU cost: `gpu_seconds` for path `e`,
    /// `composite + handoff` for path `c`. Frames in `dropped_frames`
    /// are absent.
    total_seconds: Option<[f64; 3]>,
    /// Host-side per-frame work percentiles.
    encode_seconds: [f64; 3],
    submit_seconds: [f64; 3],
    pacing: Option<Pacing>,
    energy: Option<EnergyReport>,
    conditions: Conditions,
    memory: MemoryReport,
    device: DeviceInfo,
    /// Path `e`'s import binding (`planes`, `external-format`, `rgb`).
    /// `null` on path `c`, which does not import.
    import_form: Option<&'static str>,
    /// Path `c`'s own GPU resources. See [`CopyPath::gpu_bytes`].
    /// Zero on path `e`.
    bench_gpu_bytes: u64,
    /// `git rev-parse HEAD` at the bench build.
    git_sha: &'static str,
    note: &'static str,
}

/// Maps a render-time error into a `BenchError`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Timings::render_frame takes a fn(RenderError) pointer"
)]
fn render_error(e: cherenkov::RenderError) -> BenchError {
    BenchError::Engine(format!("cherenkov render: {e}"))
}

/// The engine + shared device every external-cost run builds on.
fn engine_and_device() -> Result<(Engine<Gpu>, SharedDevice), BenchError> {
    let config = GpuConfig {
        timestamps: true,
        ..GpuConfig::default()
    };
    let shared = SharedDevice::create(&config)
        .map_err(|e| BenchError::Gpu(format!("external-cost device: {e}")))?;
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        timestamps: true,
        ..GpuConfig::default()
    })
    .map_err(|e| BenchError::Gpu(format!("external-cost engine: {e}")))?;
    Ok((engine, shared))
}

/// Snapshots engine counters + the shared device's allocator.
fn memory_snapshot(
    engine: &Engine<Gpu>,
    shared: &SharedDevice,
    detail: SampleDetail,
) -> MemorySnapshot {
    let usage = engine.memory();
    let info = shared.adapter.get_info();
    MemorySnapshot::capture(
        AdapterMemory {
            engine: Reading::Measured(EngineBytes {
                cpu_bytes: usage.cpu.0,
                gpu_bytes: usage.gpu.0,
                backdrop_capture_bytes: usage.backdrop_captures.0,
            }),
            wgpu_allocator: wgpu_allocator(&shared.device, info.backend),
            skia_budgeted: Reading::unavailable("not a Skia adapter"),
            vk_memory_budget: wgpu_vk_memory_budget(&shared.device, info.backend, &info.name),
        },
        detail,
    )
}

/// The offscreen target, one format for the run and the report.
const fn offscreen(spec: &Spec) -> Offscreen {
    Offscreen::new((spec.width, spec.height), TARGET)
}

/// Nearest-rank percentiles, or an error when the series is empty.
fn required_percentiles(name: &str, samples: &[f64]) -> Result<[f64; 3], BenchError> {
    percentiles(samples).ok_or_else(|| BenchError::Gpu(format!("external-cost {name}: no samples")))
}

/// How far past a pacing deadline a frame may start before it counts as
/// missed — `measure`'s tolerance.
const PACING_TOLERANCE: Duration = Duration::from_millis(1);

/// Runs the measurement and writes the JSON report.
///
/// # Errors
/// Platform producer allocation, GPU setup, metering and I/O failures.
#[expect(
    clippy::too_many_lines,
    reason = "one linear setup-measure-report sequence"
)]
pub(crate) fn run(args: &ExternalCostArgs) -> Result<(), BenchError> {
    let ExternalCostArgs {
        path,
        size,
        transfer,
        frames,
        warmup,
        rate,
        energy: measure_energy,
        cpu,
        out,
    } = args;
    let (frames, warmup, rate, measure_energy, out) =
        (*frames, *warmup, *rate, *measure_energy, out.as_path());
    if frames == 0 {
        return Err(BenchError::Gpu(
            "external-cost: --frames must be at least 1".into(),
        ));
    }
    let spec = Spec::new(*size, *transfer);
    let path = *path;
    if measure_energy {
        energy::Meter::probe()?;
    }
    if let Some(cpus) = cpu {
        affinity::pin_current_thread(cpus)?;
    }
    let (engine, shared) = engine_and_device()?;
    let adapter = shared.adapter.get_info();
    let idle = memory_snapshot(&engine, &shared, SampleDetail::Full);
    let mut producer = platform::Producer::new(&spec, &shared)?;
    let surface = engine
        .surface(offscreen(&spec))
        .map_err(|e| BenchError::Gpu(format!("external-cost surface: {e}")))?;
    let layer = surface.layer();
    let total = warmup
        .checked_add(frames)
        .ok_or_else(|| BenchError::Gpu("external-cost: frame count overflows".into()))?;
    let query_count = total
        .checked_mul(3)
        .ok_or_else(|| BenchError::Gpu("external-cost: query count overflows".into()))?;

    let (video, sink) = engine.frame_producer();
    let copy = match path {
        ExternalPath::External => {
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(video.at((spec.width, spec.height)));
            });
            None
        }
        ExternalPath::Copy => {
            let copy = CopyPath::new(&shared, &spec, query_count)?;
            let producer = engine.gpu_producer(GpuContentBox::new(copy.converter(), || {}));
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(producer.at((spec.width, spec.height)));
            });
            Some(copy)
        }
    };
    let bench_gpu_bytes = copy.as_ref().map_or(0, |copy| copy.gpu_bytes(&spec));

    let preparation = memory_snapshot(&engine, &shared, SampleDetail::Full);
    let mut warmup_snapshots = Vec::with_capacity(warmup as usize);
    let mut samples: Vec<CostSample> = Vec::with_capacity(frames as usize);
    let mut timings = Timings::default();
    let mut clock = Clock::new();
    let period = Duration::from_secs_f64(1.0 / rate);
    let window_hint = period * total;
    let mut meter = None;
    let mut start = Instant::now();
    let mut missed_deadlines = 0u32;
    let mut import_form = None;

    for frame in 0..total {
        if frame == warmup {
            if measure_energy {
                meter = Some(energy::Meter::begin(window_hint)?);
            }
            start = Instant::now();
        }
        if frame >= warmup
            && let Some(deadline) = period
                .checked_mul(frame - warmup)
                .and_then(|d| start.checked_add(d))
        {
            let now = Instant::now();
            if now < deadline {
                std::thread::sleep(deadline - now);
                if Instant::now().saturating_duration_since(deadline) > PACING_TOLERANCE {
                    missed_deadlines += 1;
                }
            } else if frame > warmup {
                missed_deadlines += 1;
            }
        }
        let t0 = Instant::now();
        producer.fill(frame)?;
        match path {
            ExternalPath::External => {
                let (external, form) = producer.external(frame, spec.color)?;
                if let Some(prev) = import_form {
                    if prev != form {
                        return Err(BenchError::Engine(format!(
                            "external-cost import form changed from {prev} to {form}"
                        )));
                    }
                } else {
                    import_form = Some(form);
                }
                sink.submit(external);
            }
            ExternalPath::Copy => {
                copy.as_ref()
                    .expect("path c has copy state")
                    .stage(&producer, frame, &shared)?;
            }
        }
        let t1 = Instant::now();
        timings.render_frame(&engine, &mut clock, u64::from(frame), false, render_error)?;
        producer.retire(frame, &shared.queue);
        clock.advance();
        let stats = engine.stats();
        let t2 = Instant::now();
        if frame < warmup {
            warmup_snapshots.push(memory_snapshot(&engine, &shared, SampleDetail::Frame));
        } else {
            let phases = stats.phases;
            samples.push(CostSample {
                encode_seconds: t1.duration_since(t0).as_secs_f64(),
                submit_seconds: t2.duration_since(t1).as_secs_f64(),
                gpu_seconds: None,
                handoff_seconds: None,
                convert_seconds: None,
                phases: [
                    ("lower", phases.lower_seconds),
                    ("encode", phases.encode_seconds),
                    ("stamp", phases.stamp_seconds),
                    ("wait", phases.wait_seconds),
                ]
                .into_iter()
                .map(|(name, seconds)| PhaseSample {
                    name: name.to_string(),
                    seconds,
                })
                .collect(),
            });
        }
    }

    let end = Instant::now();
    let energy_outcome = match meter {
        Some(meter) => Some(meter.finish(start, end, frames)?),
        None => None,
    };
    let steady = memory_snapshot(&engine, &shared, SampleDetail::Full);
    engine.trim(cherenkov::Pressure::Critical);
    let post_retire = memory_snapshot(&engine, &shared, SampleDetail::Full);

    for timing in timings.samples(engine.finish_timings().map_err(render_error)?) {
        let Some(index) = timing.frame.checked_sub(u64::from(warmup)) else {
            continue;
        };
        samples[usize::try_from(index).expect("a frame index fits usize")].gpu_seconds =
            timing.gpu_seconds;
    }

    if let Some(copy) = &copy {
        for (sample, (handoff, convert)) in samples
            .iter_mut()
            .zip(copy.read_stamps(&shared, warmup, frames)?)
        {
            sample.handoff_seconds = handoff;
            sample.convert_seconds = convert;
        }
    }

    let mut dropped_frames = 0u32;
    let mut composited = Vec::new();
    let mut handoff = Vec::new();
    let mut convert = Vec::new();
    let mut totals = Vec::new();
    for sample in &samples {
        if let Some(gpu) = sample.gpu_seconds {
            composited.push(gpu);
        }
        if let Some(stamp) = sample.handoff_seconds {
            handoff.push(stamp);
        }
        if let Some(stamp) = sample.convert_seconds {
            convert.push(stamp);
        }
        match (
            path,
            sample.gpu_seconds,
            sample.handoff_seconds,
            sample.convert_seconds,
        ) {
            (ExternalPath::External, Some(gpu), _, _) => totals.push(gpu),
            (ExternalPath::Copy, Some(gpu), Some(stamp), Some(_)) => totals.push(gpu + stamp),
            _ => dropped_frames += 1,
        }
    }
    let encode: Vec<f64> = samples.iter().map(|s| s.encode_seconds).collect();
    let submit: Vec<f64> = samples.iter().map(|s| s.submit_seconds).collect();
    let window_seconds = end.duration_since(start).as_secs_f64();
    let pacing = Some(Pacing {
        requested_hz: rate,
        achieved_hz: f64::from(frames) / window_seconds,
        missed_deadlines,
        window_seconds,
    });

    let report = CostReport {
        adapter: adapter.name.clone(),
        backend: format!("{:?}", adapter.backend),
        driver: adapter.driver.clone(),
        driver_info: adapter.driver_info.clone(),
        path: match path {
            ExternalPath::External => "external",
            ExternalPath::Copy => "copy-convert",
        },
        layout: spec.layout(),
        transfer: spec.transfer_name(),
        width: spec.width,
        height: spec.height,
        warmup_frames: warmup,
        measured_frames: frames,
        dropped_frames,
        samples,
        composite_seconds: percentiles(&composited),
        handoff_seconds: percentiles(&handoff),
        convert_seconds: percentiles(&convert),
        total_seconds: percentiles(&totals),
        encode_seconds: required_percentiles("encode_seconds", &encode)?,
        submit_seconds: required_percentiles("submit_seconds", &submit)?,
        pacing,
        conditions: conditions::collect(
            energy_outcome
                .as_ref()
                .and_then(|o| o.thermal_pressure.clone()),
        ),
        energy: energy_outcome.map(|o| o.report),
        memory: MemoryReport::new(
            idle,
            &{
                let mut all = vec![preparation];
                all.extend(warmup_snapshots);
                all.push(steady);
                all
            },
            Some(post_retire),
        ),
        device: DeviceInfo {
            adapter: Some(adapter.name),
            backend: Some(format!("{:?}", adapter.backend)),
            driver: Some(adapter.driver),
            driver_info: Some(adapter.driver_info),
            vendor: Some(adapter.vendor),
            device: Some(adapter.device),
            target_format: Some(target_format_name(TARGET).to_string()),
            cpu: crate::cpu_model(),
            thermal_celsius: crate::thermal_celsius(),
        },
        import_form,
        bench_gpu_bytes,
        git_sha: env!("CHERENKOV_GIT_SHA"),
        note: "path c times plane copies and the convert pass in one submission \
               (marker end stamp, copy_buffer_to_texture, convert pass). \
               handoff_seconds is that GPU interval; convert_seconds is the pass; \
               gpu_seconds is the engine composite. A missing stamp is a dropped \
               frame, not a zero. bench_gpu_bytes counts path c's copy textures, \
               staging buffers, uniform, query resolve and readback, and the marker.",
    };
    let file = std::fs::File::create(out)
        .map_err(|e| BenchError::Gpu(format!("write {}: {e}", out.display())))?;
    serde_json::to_writer_pretty(file, &report)
        .map_err(|e| BenchError::Gpu(format!("report json: {e}")))?;
    tracing::info!(out = %out.display(), "external-cost");
    Ok(())
}

/// Renders one synthetic frame through `path` on a fresh engine and
/// returns the composited working-space pixels — the E-vs-C pair the
/// correctness test (`tests/external_cost.rs`) compares.
///
/// The pattern frame and the stamp slot are separate. `frame` selects
/// the gradient; the convert pass writes stamps 0, 1 and 2.
///
/// # Errors
/// Device, producer, engine and readback failures.
pub fn composite_frame(
    path: ExternalPath,
    size: ExternalSize,
    transfer: ExternalTransfer,
    frame: u32,
) -> Result<Vec<[f32; 4]>, BenchError> {
    let spec = Spec::new(size, transfer);
    let (engine, shared) = engine_and_device()?;
    let mut producer = platform::Producer::new(&spec, &shared)?;
    let surface = engine
        .surface(offscreen(&spec))
        .map_err(|e| BenchError::Gpu(format!("external-cost surface: {e}")))?;
    let layer = surface.layer();

    producer.fill(frame)?;
    match path {
        ExternalPath::External => {
            let (external, _) = producer.external(frame, spec.color)?;
            let (video, sink) = engine.frame_producer();
            sink.submit(external);
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(video.at((spec.width, spec.height)));
            });
        }
        ExternalPath::Copy => {
            let copy = CopyPath::new(&shared, &spec, 3)?;
            let gpu = engine.gpu_producer(GpuContentBox::new(copy.converter(), || {}));
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(gpu.at((spec.width, spec.height)));
            });
            copy.stage(&producer, frame, &shared)?;
        }
    }
    engine.render(FrameTime::now()).map_err(render_error)?;
    producer.retire(frame, &shared.queue);
    let rb = surface.readback().map_err(render_error)?;
    Ok(rb.pixels)
}

/// Writes `src` at `start` in a mapped staging buffer.
///
/// [`wgpu::BufferViewMut`] does not dereference to `&mut [u8]`: the
/// mapping may be write-combining memory.
#[cfg(any(target_vendor = "apple", target_os = "android"))]
fn write_tight(dst: &mut wgpu::BufferViewMut, start: usize, src: &[u8]) {
    dst.slice(start..start + src.len()).copy_from_slice(src);
}

/// Completion of the submission that last used each ring slot.
///
/// `recv` alone would deadlock: the callback runs on a thread that is
/// polling the device. [`SlotDone::wait`] polls without waiting for the
/// newest submission, so frame f−1 can stay in flight while slot
/// `f % RING` is reused.
#[cfg(any(target_vendor = "apple", target_os = "android"))]
struct SlotDone {
    slots: [Option<std::sync::mpsc::Receiver<()>>; RING],
    used: [bool; RING],
}

#[cfg(any(target_vendor = "apple", target_os = "android"))]
impl SlotDone {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            used: [false; RING],
        }
    }

    fn wait(&mut self, frame: u32, device: &wgpu::Device) -> Result<(), BenchError> {
        let slot = frame as usize % RING;
        if !self.used[slot] {
            self.used[slot] = true;
            return Ok(());
        }
        let rx = self.slots[slot].take().ok_or_else(|| {
            BenchError::Gpu(format!(
                "external-cost: frame {frame} reuses slot {slot} with no completion armed"
            ))
        })?;
        let start = Instant::now();
        loop {
            device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| BenchError::Gpu(format!("external-cost completion poll: {e}")))?;
            match rx.try_recv() {
                Ok(()) => return Ok(()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if start.elapsed() >= COMPLETION_TIMEOUT {
                        return Err(BenchError::Gpu(format!(
                            "external-cost: frame {frame} slot {slot} still in flight after 30s"
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(BenchError::Gpu(format!(
                        "external-cost: frame {frame} slot {slot} completion dropped"
                    )));
                }
            }
        }
    }

    fn arm(&mut self, frame: u32, queue: &wgpu::Queue) {
        let (tx, rx) = std::sync::mpsc::channel();
        queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });
        self.slots[frame as usize % RING] = Some(rx);
    }
}

/// `CVPixelBuffer` producer — the Apple video-decode output model.
#[cfg(target_vendor = "apple")]
#[path = "external_cost/apple.rs"]
mod platform;

/// `AHardwareBuffer` producer — the Android video-decode output model.
#[cfg(target_os = "android")]
#[path = "external_cost/android.rs"]
mod platform;

/// No platform buffer API: `external-cost` fails fast.
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
mod platform {
    use super::{BenchError, ExternalFrame, FrameColor, SharedDevice, Spec, wgpu};

    /// The stub producer.
    pub struct Producer;

    impl Producer {
        /// Always fails: no platform video-buffer API exists here.
        pub fn new(_spec: &Spec, _shared: &SharedDevice) -> Result<Self, BenchError> {
            Err(BenchError::Gpu(
                "external-cost has no platform frame producer on this OS".into(),
            ))
        }

        /// Unreachable — [`Producer::new`] always fails.
        #[expect(
            clippy::needless_pass_by_ref_mut,
            reason = "matches the platform producer, which fills through &mut self"
        )]
        pub fn fill(&mut self, _frame: u32) -> Result<(), BenchError> {
            unreachable!("Producer::new failed ({self:p})")
        }

        /// Unreachable — [`Producer::new`] always fails.
        pub fn copy_planes(
            &self,
            _frame: u32,
            _dst: &mut wgpu::BufferViewMut,
            _luma_stride: usize,
            _chroma_offset: usize,
            _chroma_stride: usize,
        ) -> Result<(), BenchError> {
            unreachable!("Producer::new failed ({self:p})")
        }

        /// Unreachable — [`Producer::new`] always fails.
        pub fn external(
            &self,
            _frame: u32,
            _color: FrameColor,
        ) -> Result<(ExternalFrame, &'static str), BenchError> {
            unreachable!("Producer::new failed ({self:p})")
        }

        /// Unreachable — [`Producer::new`] always fails.
        #[expect(
            clippy::needless_pass_by_ref_mut,
            reason = "matches the platform producer, which arms the slot through &mut self"
        )]
        pub fn retire(&mut self, _frame: u32, _queue: &wgpu::Queue) {
            unreachable!("Producer::new failed ({self:p})")
        }
    }
}
