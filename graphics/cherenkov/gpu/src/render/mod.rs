//! The render thread: sole owner of GPU state.

mod bindings;
mod bitmap;
mod colr;
#[cfg(target_vendor = "apple")]
mod composite;
pub mod diag;
#[cfg(target_os = "linux")]
pub mod dmabuf_export;
pub mod external;
pub mod filter;
mod glyph;
mod gpu_content;
mod instance;
mod lower;
mod paint;
mod path;
pub mod planes;
mod prepared;
pub mod present;
mod projective;
mod raster;
mod reduce;
mod resolve;
pub mod shaders;
mod shadow;
pub mod surface_control;
mod upload;

use cherenkov::Instant;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHashSet};
use wgpu::util::DeviceExt;

use crate::interop::ExternalFrame;
use crate::{
    CreationPhase, CreationPoint, GpuConfig, GpuInfo, GpuTarget, ScratchFormat, TimestampSupport,
    names,
};
use bitmap::BitmapKey;
use cherenkov::{
    ContentOp, EngineError, FontData as EngineFontData, FontId, Frame, FrameId, FrameRedraw,
    FrameStats, FrameTiming, ImageId, ImageUpload, LayerId, MemoryUsage, PassTiming, Pressure,
    ProducerId, Readback, RenderError, Renderer, ResourceError, ResourceId, SurfaceError,
    SurfaceFrame, SurfaceId, SurfaceInfo, Visibility,
};
/// The export pool's platform type: the bounded dma-buf pool on Linux
/// (#1687), an uninhabited stand-in elsewhere.
#[cfg(target_os = "linux")]
use dmabuf_export::Pool as ExportPool;
use glyph::{Atlas, FontData, PendingRaster, PreparedFont};
use lower::{
    BackdropGroupInfo, ContentData, Frame as LoweredFrame, GlyphContext, Lowered, Lowering,
    PipelineKind, ShaderVariant, Source, Target,
};
use shaders::backdrop_effect_text;

/// Stands in for the dma-buf export pool where dma-buf does not exist: no
/// value exists, so no surface there has an export pool.
#[cfg(not(target_os = "linux"))]
enum ExportPool {}

/// The pipeline bound for a pass range: engine pipelines and the external
/// frame pipeline are mutually exclusive, so an engine range always rebinds
/// after an external one and vice versa.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Engine(PipelineKind, ShaderVariant),
    External,
    /// The projective composite, source-over or replace.
    Projective(bool),
}

#[derive(Clone, Copy)]
enum CoveragePhase {
    Opaque = 0,
    Partial = 1,
}

/// The instance-index bound a coverage-order pass may assign: `shader.wgsl`
/// derives each depth as `bitcast<f32>(0x3e000000u + ii * 8u)`, injective —
/// and below 0.5 — only while `ii` is under this bound. The lowering fails
/// rather than alias two instances to one depth.
pub const MAX_PASS_INSTANCES: usize = 1 << 21;

#[derive(Clone, Copy)]
enum CoveragePass {
    Painter(bool),
    Coverage(CoveragePhase),
}

impl CoveragePass {
    const fn topology(self) -> wgpu::PrimitiveTopology {
        match self {
            Self::Painter(_) | Self::Coverage(CoveragePhase::Partial) => {
                wgpu::PrimitiveTopology::TriangleList
            }
            Self::Coverage(CoveragePhase::Opaque) => wgpu::PrimitiveTopology::TriangleStrip,
        }
    }

    const fn vertex(self) -> &'static str {
        match self {
            Self::Painter(_) => "vs_main",
            Self::Coverage(CoveragePhase::Opaque) => "vs_opaque",
            Self::Coverage(CoveragePhase::Partial) => "vs_partial",
        }
    }

    const fn fragment(self) -> &'static str {
        match self {
            Self::Painter(_) => "fs_main",
            Self::Coverage(CoveragePhase::Opaque) => "fs_opaque",
            Self::Coverage(CoveragePhase::Partial) => "fs_partial",
        }
    }

    const fn blend(self) -> Option<wgpu::BlendState> {
        match self {
            Self::Painter(false) | Self::Coverage(CoveragePhase::Partial) => {
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
            }
            Self::Painter(true) | Self::Coverage(CoveragePhase::Opaque) => None,
        }
    }

    fn depth(self) -> Option<wgpu::DepthStencilState> {
        match self {
            Self::Painter(_) => None,
            Self::Coverage(phase) => Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(matches!(phase, CoveragePhase::Opaque)),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        }
    }
}

/// The surface target format: premultiplied linear Display P3.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Amortize Metal's limited number of counter sample buffers across frames.
/// Each range remains exclusive until its frame's readback completes.
const TIMESTAMP_FRAMES_PER_SET: u32 = 64;

const TARGET_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::from_bits_retain(
    wgpu::TextureUsages::RENDER_ATTACHMENT.bits()
        | wgpu::TextureUsages::COPY_SRC.bits()
        | wgpu::TextureUsages::COPY_DST.bits()
        | wgpu::TextureUsages::TEXTURE_BINDING.bits(),
);

/// Linear sRGB (BT.709 primaries, D65) to CIE XYZ — the oracle's
/// `SRGB_TO_XYZ`.
const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_390_799_265_959_4,
        0.357_584_339_383_878,
        0.180_480_788_401_834_3,
    ],
    [
        0.212_639_005_871_510_4,
        0.715_168_678_767_756,
        0.072_192_315_360_733_7,
    ],
    [
        0.019_330_818_715_591_8,
        0.119_194_779_410_625_9,
        0.950_532_152_249_660_5,
    ],
];

/// CIE XYZ to linear Display P3 (`P3_TO_XYZ` inverted), precomputed.
const XYZ_TO_P3: [[f64; 3]; 3] = [
    [
        2.493_496_911_941_425,
        -0.931_383_617_919_123_9,
        -0.402_710_784_450_716_2,
    ],
    [
        -0.829_488_969_561_574_7,
        1.762_664_060_318_226_3,
        0.023_624_685_848_943_6,
    ],
    [
        0.035_845_830_243_784_5,
        -0.076_172_389_268_041_4,
        0.956_884_524_007_687_1,
    ],
];

/// The `wgpu` format for a [`ScratchFormat`].
const fn scratch_wgpu(format: ScratchFormat) -> wgpu::TextureFormat {
    match format {
        ScratchFormat::LinearF16 => wgpu::TextureFormat::Rgba16Float,
        ScratchFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
    }
}

/// The pass-report spelling of a texture format.
const fn format_name(format: wgpu::TextureFormat) -> &'static str {
    match format {
        wgpu::TextureFormat::Rgba16Float => "rgba16float",
        wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
        _ => "unknown",
    }
}

/// One isolation scratch or backdrop texture.
struct ScratchTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl ScratchTarget {
    /// Resident texel bytes at the texture's format.
    fn bytes(&self) -> u64 {
        target_bytes(self, 0)
    }
}

/// Resident bytes of a transient depth texture. Metal exposes its actual
/// backing allocation; memoryless attachments have no persistent backing.
fn coverage_depth_bytes(texture: &wgpu::Texture) -> u64 {
    #[cfg(target_vendor = "apple")]
    {
        use objc2_metal::MTLResource;
        // SAFETY: the native texture is only inspected while its guard lives.
        if let Some(native) = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() } {
            return native.raw_handle().allocatedSize() as u64;
        }
    }
    u64::from(texture.width()) * u64::from(texture.height()) * 4
}

/// A GPU-resident image registered with the engine.
pub struct GpuImage {
    /// The texture holding premultiplied linear-P3 f16 texels.
    pub texture: wgpu::Texture,
    /// Its view for bind group 1.
    pub view: wgpu::TextureView,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
}

pub struct GpuBitmap {
    pub(super) image: GpuImage,
    pub(super) em: kurbo::Rect,
}

/// Validated painter commands; uploads may change their instance contents.
struct PainterReplay {
    bind0: wgpu::BindGroup,
    bind1: wgpu::BindGroup,
    offset: u32,
    base: u32,
    format: wgpu::TextureFormat,
    ranges: Vec<(ShaderVariant, std::ops::Range<u32>)>,
    bundle: wgpu::RenderBundle,
}

/// One surface's GPU-side state.
struct SurfaceState {
    window: Option<present::WindowSurface>,
    /// Whether the target exposes a system-compositor parent, so eligible
    /// layers are promoted onto planes (`GpuRenderer::planes`).
    promotes: bool,
    /// This frame's promotion decision; empty unless `promotes`.
    plan: planes::Plan,
    /// A discarded plan must be lowered again before any presentation.
    plan_dirty: bool,
    /// Tree version of the retained engine parts, excluding plane poses.
    plane_stamp: u64,
    /// Resource version of those parts.
    plane_resources: (u64, u64, u64),
    /// Surface-level dependencies absent from the layer tree's stamps.
    plane_clear: cherenkov::WorkingColor,
    plane_size: (u32, u32),
    /// Source observations and immutable captures for recorded planes.
    static_layers: FxHashMap<LayerId, planes::static_layer::Observation>,
    static_walk: Vec<(LayerId, kurbo::Affine)>,
    /// Engine parts above the first (`target` is part 0): one per promoted
    /// plane with layers painted above it, at the surface size.
    parts: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// Texture-target hand-off: each rendered texture goes to the host.
    textures: Option<std::sync::mpsc::Sender<wgpu::Texture>>,
    refresh: cherenkov::RefreshRange,
    present_pending: bool,
    /// The last display properties the frame carried — a change triggers
    /// the window's output re-selection (#98).
    display: cherenkov::Display,
    size: (u32, u32),
    target: wgpu::Texture,
    view: wgpu::TextureView,
    /// Scratch textures, one per isolation depth, sized to the largest
    /// region seen so far.
    scratch: FxHashMap<usize, ScratchTarget>,
    /// Transient depth for opaque regions; cleared and discarded in one pass.
    coverage_depth: Option<ScratchTarget>,
    /// Backdrop copies for blend composites: index 0 matches the surface
    /// format, index 1 the scratch format.
    backdrop: [Option<ScratchTarget>; 2],
    /// The staged resolves' shared 1:1 device copies: every reduced
    /// capture with looked-through composites to apply first copies its
    /// device rect into the slot of its format, draws the composites
    /// over it and resolves from it — its contents die with that pass's
    /// resolve, so one texture per format serves every group, within
    /// and across frames. Slot 0 matches the surface format (a capture
    /// copying a part, projected image or plane), slot 1 the scratch
    /// format when it differs from the surface format (a capture
    /// copying a semantic isolation's scratch); otherwise slot 0
    /// serves both. Each grows, never shrinks, to the largest staged
    /// device rect of its format.
    staging: [Option<ScratchTarget>; 2],
    /// The staged-resolve bind group beside each staging slot: every
    /// staged resolve binds the same globals buffer and staging view at
    /// a dynamic offset, so one bind per slot serves all of them. It is
    /// dropped and rebuilt with its texture — a stale entry would keep
    /// the replaced view alive after the accounting says it is freed.
    /// The per-region bind caches hold only direct resolves.
    staging_bind: [Option<resolve::Bind>; 2],
    /// Backdrop groups registered on this surface by raw id.
    backdrop_groups: FxHashMap<u64, BackdropGroupState>,
    layers: FxHashMap<LayerId, ContentData>,
    /// GPU producer bindings by layer (`cherenkov::GpuContent`): a layer
    /// binds one producer, a producer serves bindings on any surface,
    /// and every binding samples the producer's current frame.
    bindings: FxHashMap<LayerId, gpu_content::Binding>,
    /// Hosted system layers by layer (`cherenkov::HostedLayers`): shown
    /// only on planes. Lowering never sees them — the map reaches nothing
    /// but the plan's candidates and the plane stack.
    hosted: FxHashMap<LayerId, planes::HostedBinding>,
    shader_textures: FxHashMap<std::sync::Arc<paint::Key>, paint::Texture>,
    frame: LoweredFrame,
    /// Image lifetimes and attachment assignment, before physical allocation.
    #[cfg(target_vendor = "apple")]
    composition: composite::plan::ExecutionPlan,
    #[cfg(target_vendor = "apple")]
    composition_cache: composite::cache::Cache,
    #[cfg(target_vendor = "apple")]
    tile_targets: composite::metal::Attachments,
    /// This frame's offsets into the shared buffers: instances and globals
    /// (256-byte slots) are laid out surface by surface so one upload covers
    /// every dirty surface.
    inst_base: u32,
    globals_base: u32,
    /// Bumped whenever a scratch or backdrop texture is (re)created — a
    /// cached group-1 bind group referencing the old view must rebuild.
    bind_gen: u64,
    /// Group-1 bind groups keyed by `(source, backdrop, image,
    /// mask texture)`, reused across frames while `binds1_stamp` is
    /// current.
    binds1: FxHashMap<Bind1Key, wgpu::BindGroup>,
    painter_replays: FxHashMap<usize, PainterReplay>,
    /// The `(bind_gen, images_gen, mask_texture_gen)` triple `binds1` was
    /// built under.
    binds1_stamp: (u64, u64, u64),
    /// Projective layers' retained local images (#84), by layer, one per
    /// density bucket.
    projective: FxHashMap<LayerId, Vec<projective::Entry>>,
    /// The local images this frame renders, allocated before encoding.
    realized: Vec<projective::Realize>,
    /// The local images this frame composes.
    composed: Vec<projective::LocalKey>,
    /// Bumped whenever GPU content or an external frame is installed or
    /// resized: local images that sampled one are then stale.
    interop: u64,
    /// The host's announced visibility as the render loop applied it. A
    /// hidden surface is in no frame, and its producers and filters want
    /// no redraw.
    visibility: Visibility,
    /// The surface's host wake-up: the producers and filters its frames
    /// draw wake the host through it, so they stop waking the moment the
    /// host hides the surface.
    waker: cherenkov::CompletionWaker,
}

/// One backdrop region's capture: the mipmapped texture whose mip 0 is
/// level 0 — the resolve's target, optionally filtered in place — and
/// whose deeper mips hold the pyramid's levels.
struct Capture {
    /// The texture and its level-0 view; `width`/`height` are the
    /// allocation's texels, the region size aligned up to the deepest
    /// level's grid.
    target: ScratchTarget,
    /// The whole-pyramid view `backdrop_sample_at` reads; `None` for a
    /// one-level capture, whose `target.view` is read instead.
    source: Option<wgpu::TextureView>,
    /// `levels[k − 1]` is level `k`'s single-mip view, for `k` in `1..n`.
    levels: Vec<wgpu::TextureView>,
    /// `reduces[k − 1]` caches the level `k` step's bind group.
    reduces: Vec<Option<reduce::Bind>>,
}

impl Capture {
    /// The view the member composites sample.
    fn sample_view(&self) -> &wgpu::TextureView {
        self.source.as_ref().unwrap_or(&self.target.view)
    }
}

/// A registered backdrop group: its optional capture filter, its capture
/// spec and the capture textures, one per region (#117 sparse capture),
/// each exactly sized to this frame's region and level count.
struct BackdropGroupState {
    /// The group's filter chain key, when registered with a filter.
    filter: Option<filter::FilterKey>,
    /// The group's capture spec (scale and level count).
    spec: cherenkov::BackdropSpec,
    /// The capture textures indexed by region, empty until a frame
    /// samples the group.
    captures: Vec<Capture>,
    /// Each region's cached resolve bind group.
    resolves: Vec<Option<resolve::Bind>>,
}

impl BackdropGroupState {
    /// Bytes the group's capture textures hold: every region's capture
    /// with its pyramid levels. Staging is the surface's, counted there.
    fn bytes(&self) -> u64 {
        let deep = self.spec.levels().get() - 1;
        self.captures
            .iter()
            .map(|capture| target_bytes(&capture.target, deep))
            .sum()
    }

    /// Keeps the first `regions` regions' textures and bind groups.
    fn truncate(&mut self, regions: usize) {
        self.captures.truncate(regions);
        self.resolves.truncate(regions);
    }
}

impl SurfaceState {
    /// Engine part `n`'s texture: `target` for part 0.
    fn part(&self, n: u32) -> (&wgpu::TextureView, &wgpu::Texture) {
        match n {
            0 => (&self.view, &self.target),
            n => {
                let (texture, view) = &self.parts[n as usize - 1];
                (view, texture)
            }
        }
    }

    /// The `BackdropGroupInfo` map lowering needs for this surface.
    fn backdrop_info(&self, filters: &mut filter::Registry) -> FxHashMap<u64, BackdropGroupInfo> {
        self.backdrop_groups
            .iter()
            .map(|(g, state)| {
                (
                    *g,
                    BackdropGroupInfo {
                        filter: state.filter,
                        footprint: state
                            .filter
                            .map_or(Some(filtrate_core::Footprint::ZERO), |key| {
                                filters.footprint_bound(key)
                            }),
                        spec: state.spec,
                    },
                )
            })
            .collect()
    }

    /// Bytes held by the surface's shared backdrop staging textures.
    fn staging_bytes(&self) -> u64 {
        self.staging
            .iter()
            .flatten()
            .map(ScratchTarget::bytes)
            .sum()
    }

    /// Bytes held by this surface's backdrop captures, summed over all
    /// regions of all groups, pyramid levels included; the shared
    /// staging is counted by [`staging_bytes`](Self::staging_bytes).
    fn backdrop_bytes(&self) -> u64 {
        self.backdrop_groups
            .values()
            .map(BackdropGroupState::bytes)
            .sum()
    }

    /// Retained local images whose renderer-side inputs changed since
    /// they were realized: an animating filter, redrawn GPU content, an
    /// animated shader paint, or a new GPU content or external frame.
    fn stale_projective(
        &self,
        filters: &filter::Registry,
        shaders: &paint::Registry,
        producers: &FxHashMap<ProducerId, gpu_content::Producer>,
    ) -> FxHashSet<projective::LocalKey> {
        self.projective
            .iter()
            .flat_map(|(layer, entries)| entries.iter().map(move |e| (*layer, e)))
            .filter(|(_, e)| {
                e.deps.interop.is_some_and(|g| g != self.interop)
                    || e.deps.filters.iter().any(|f| filters.wants_redraw(*f))
                    || e.deps.images.iter().any(|image| match image {
                        lower::ImageSource::Content(id) => producers
                            .get(id)
                            .is_none_or(gpu_content::Producer::wants_redraw),
                        lower::ImageSource::Shader(key) => shaders.animated(key),
                        _ => false,
                    })
            })
            .map(|(layer, e)| projective::LocalKey {
                layer,
                bucket: e.bucket,
            })
            .collect()
    }

    /// Whether a local image the latest frame composed went stale.
    fn projective_wants_redraw(
        &self,
        filters: &filter::Registry,
        shaders: &paint::Registry,
        producers: &FxHashMap<ProducerId, gpu_content::Producer>,
    ) -> bool {
        !self.composed.is_empty() && {
            let stale = self.stale_projective(filters, shaders, producers);
            self.composed.iter().any(|key| stale.contains(key))
        }
    }

    /// Bytes held by this surface's retained local images.
    fn projective_bytes(&self) -> u64 {
        self.projective
            .values()
            .flatten()
            .map(projective::Entry::bytes)
            .sum()
    }

    fn content_wants_redraw(
        &self,
        producers: &FxHashMap<ProducerId, gpu_content::Producer>,
    ) -> bool {
        !self.bindings.is_empty()
            && self.frame.content.iter().any(|(id, _)| {
                producers
                    .get(id)
                    .is_some_and(gpu_content::Producer::wants_redraw)
            })
    }

    /// Bytes held by this surface's textures.
    fn gpu_bytes(&self) -> u64 {
        let surface_bytes = u64::from(self.size.0) * u64::from(self.size.1) * 8;
        let scratch_bytes: u64 = self.scratch.values().map(ScratchTarget::bytes).sum();
        let backdrop_bytes: u64 = self
            .backdrop
            .iter()
            .flatten()
            .map(ScratchTarget::bytes)
            .sum();
        let shader_bytes = self
            .shader_textures
            .values()
            .map(|texture| {
                u64::from(texture.image.width) * u64::from(texture.image.height) * 8 + 272
            })
            .sum::<u64>();
        surface_bytes * (1 + self.parts.len() as u64)
            + self
                .coverage_depth
                .as_ref()
                .map_or(0, |depth| coverage_depth_bytes(&depth.texture))
            + scratch_bytes
            + backdrop_bytes
            + self.staging_bytes()
            + shader_bytes
            + self.projective_bytes()
            + self
                .static_layers
                .values()
                .filter_map(|entry| entry.capture.as_ref())
                .map(planes::static_layer::Capture::bytes)
                .sum::<u64>()
            + self.backdrop_bytes()
    }
}

/// The retained local image `key` names: the frame composes only images
/// it realized or found current, so a missing one is a lowering bug.
/// A reduced capture pass's resolve, prepared before its render pass: the
/// pipeline, the bind group reading its source, and — for a staged
/// resolve, which runs in a pass of its own after the draws — the
/// capture it writes.
struct ResolveDraw {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    /// The capture view and the region's size, for a staged resolve.
    staged: Option<(wgpu::TextureView, (u32, u32))>,
}

/// The view a capture's `copy_from` target is read through.
fn source_view(surf: &SurfaceState, target: Target) -> Result<&wgpu::TextureView, RenderError> {
    Ok(match target {
        Target::Plane(layer) => {
            &surf.static_layers[&layer]
                .capture
                .as_ref()
                .expect("allocated capture")
                .source
                .as_ref()
                .expect("capture source allocated before encode")
                .1
        }
        Target::Part(n) => surf.part(n).0,
        Target::Projected(key) => &projective_entry(&surf.projective, key).levels[0],
        Target::Scratch(k) => &surf.scratch[&k].view,
        Target::Backdrop { group, .. } => {
            return Err(RenderError::Render(format!(
                "backdrop group {group} copies from a capture"
            )));
        }
    })
}

/// The format a capture takes: the surface format when its copy
/// source is a part, a projected image or a plane; the scratch format
/// when it is a semantic isolation's scratch. A capture never copies
/// a capture.
fn capture_format(
    copy_from: Target,
    scratch_format: wgpu::TextureFormat,
) -> Result<wgpu::TextureFormat, RenderError> {
    match copy_from {
        Target::Part(_) | Target::Projected(_) | Target::Plane(_) => Ok(TARGET_FORMAT),
        Target::Scratch(_) => Ok(scratch_format),
        Target::Backdrop { group, .. } => Err(RenderError::Render(format!(
            "backdrop group {group} copies from a capture"
        ))),
    }
}

/// The staging slot a texture format uses: slot 1 the scratch format's
/// staging when it differs from the surface format; otherwise slot 0
/// serves both.
fn staging_slot(format: wgpu::TextureFormat) -> usize {
    usize::from(format != TARGET_FORMAT)
}

/// Prepares pass `index`'s resolve when its capture is reduced: a direct
/// resolve reads `copy_from` inside the pass, a staged one reads the
/// staging texture after it.
fn prepare_resolve(
    device: &wgpu::Device,
    pipelines: &mut resolve::Pipelines,
    globals: &wgpu::Buffer,
    surf: &mut SurfaceState,
    index: usize,
    scratch_format: wgpu::TextureFormat,
) -> Result<Option<ResolveDraw>, RenderError> {
    let pass = &surf.frame.passes[index];
    let (Some(capture), Target::Backdrop { group, region }) = (pass.capture, pass.target) else {
        return Ok(None);
    };
    if capture.resolve.is_none() {
        return Ok(None);
    }
    let staged = resolve::staged(pass);
    let size = (pass.region[2], pass.region[3]);
    let state = &surf.backdrop_groups[&group];
    let target = &state.captures[region as usize].target;
    let pipeline = pipelines.pipeline(device, target.texture.format()).clone();
    let capture_view = target.view.clone();
    let bind = if staged {
        // A staged resolve reads the staging slot of its capture's
        // format; one bind beside the slot serves every staged resolve.
        let slot = staging_slot(capture_format(capture.copy_from, scratch_format)?);
        let staging = &surf.staging[slot]
            .as_ref()
            .expect("staging allocated before encode")
            .view;
        pipelines
            .bind(device, &mut surf.staging_bind[slot], globals, staging)
            .clone()
    } else {
        let source = source_view(surf, capture.copy_from)?.clone();
        let state = surf
            .backdrop_groups
            .get_mut(&group)
            .expect("looked up above");
        if state.resolves.len() <= region as usize {
            state.resolves.resize_with(region as usize + 1, || None);
        }
        pipelines
            .bind(
                device,
                &mut state.resolves[region as usize],
                globals,
                &source,
            )
            .clone()
    };
    Ok(Some(ResolveDraw {
        pipeline,
        bind,
        staged: staged.then_some((capture_view, size)),
    }))
}

fn projective_entry(
    entries: &FxHashMap<LayerId, Vec<projective::Entry>>,
    key: projective::LocalKey,
) -> &projective::Entry {
    entries
        .get(&key.layer)
        .and_then(|entries| entries.iter().find(|e| e.bucket == key.bucket))
        .expect("a composed local image is retained")
}

/// [`projective_entry`], mutably.
fn projective_entry_mut(
    entries: &mut FxHashMap<LayerId, Vec<projective::Entry>>,
    key: projective::LocalKey,
) -> &mut projective::Entry {
    entries
        .get_mut(&key.layer)
        .and_then(|entries| entries.iter_mut().find(|e| e.bucket == key.bucket))
        .expect("a composed local image is retained")
}

/// Drops the retained local images of `surf` that `dead` selects and
/// forgets them as composed.
fn retire_projective(
    surf: &mut SurfaceState,
    device: &wgpu::Device,
    reason: &'static str,
    dead: impl Fn(LayerId, &projective::Entry) -> bool,
) {
    let mut bytes = 0;
    let mut composed = false;
    for (layer, entries) in &mut surf.projective {
        entries.retain(|entry| {
            if !dead(*layer, entry) {
                return true;
            }
            bytes += entry.bytes();
            let key = projective::LocalKey {
                layer: *layer,
                bucket: entry.bucket,
            };
            let before = surf.composed.len();
            surf.composed.retain(|k| *k != key);
            composed |= surf.composed.len() != before;
            false
        });
    }
    surf.projective.retain(|_, entries| !entries.is_empty());
    if bytes > 0 {
        diag::retire(
            device,
            diag::RetireArgs {
                label: "projective image",
                class: diag::Class::Target,
                bytes,
                used_in_latest_submit: composed,
                reason,
            },
        );
    }
}

/// The group-1 bind group key: `(source, backdrop-needed,
/// image, mask texture)`.
type Bind1Key = (
    Option<lower::Source>,
    bool,
    Option<lower::ImageSource>,
    Option<u64>,
);

/// Drops the group-1 bind groups `reject` marks obsolete — unsubmitted
/// groups whose views keep a replaced texture's predecessor alive
/// (#169 A4) — and refreshes `binds1_stamp` so the survivors persist
/// past the next encode. Bind groups already consumed by a submitted
/// encoder are wgpu's to retain until that submission completes.
fn retire_binds1(
    surf: &mut SurfaceState,
    device: &wgpu::Device,
    images_gen: u64,
    mask_gen: u64,
    reason: &'static str,
    reject: impl Fn(&Bind1Key) -> bool,
) {
    let before = surf.binds1.len();
    surf.binds1.retain(|key, _| !reject(key));
    let dropped = before - surf.binds1.len();
    if dropped > 0 {
        diag::bind_groups_dropped(device, dropped as u64, reason);
    }
    surf.binds1_stamp = (surf.bind_gen, images_gen, mask_gen);
}

impl SurfaceState {
    /// Drops every cached bind group of the surface: all the views they
    /// sample were just replaced or freed. The group-1 cache was counted
    /// by the caller; the direct-resolve drops report their own count.
    fn retire_binds(&mut self, device: &wgpu::Device) {
        self.binds1.clear();
        self.drop_resolve_binds(device, "backdrop resolve");
    }

    /// Drops every backdrop group's cached direct-resolve binds: the views
    /// they read are all gone. Kept, a cached bind would retain its
    /// source's retired texture past the accounting that says it is freed.
    fn drop_resolve_binds(&mut self, device: &wgpu::Device, reason: &'static str) {
        let dropped = self
            .backdrop_groups
            .values()
            .flat_map(|group| group.resolves.iter())
            .flatten()
            .count() as u64;
        for group in self.backdrop_groups.values_mut() {
            group.resolves.clear();
        }
        if dropped > 0 {
            diag::bind_groups_dropped(device, dropped, reason);
        }
    }

    /// The `drop_resolve_binds` of one view: only the groups' entries
    /// built on `view` are dropped — the source the regenerated texture
    /// replaced.
    fn drop_resolve_binds_on(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        reason: &'static str,
    ) {
        let mut dropped = 0u64;
        for group in self.backdrop_groups.values_mut() {
            for bind in &mut group.resolves {
                if bind.as_ref().is_some_and(|bind| bind.source() == view) {
                    *bind = None;
                    dropped += 1;
                }
            }
        }
        if dropped > 0 {
            diag::bind_groups_dropped(device, dropped, reason);
        }
    }
}

/// A registered backdrop effect shader. Registration builds only the
/// plain-variant `SrcOver` pipelines (slots `format_i` — one per distinct
/// target format); the union-linked module is stored so a union-field
/// member's pipeline (`slot format_i + 2`) is built on its first use —
/// lazily inside encode on native targets, ahead of encode by
/// `prepare_union_pipelines` on wasm.
struct BackdropShaderEntry {
    /// The union-shaped module text, validated at registration; the
    /// module is built on the first union use.
    union_source: std::sync::Arc<str>,
    /// The union-shaped module, built lazily with the first union slot.
    union_module: Option<wgpu::ShaderModule>,
    /// The plain-variant pipelines by format index — built at
    /// registration, `Some` for each distinct target format (the
    /// scratch-format slot stays `None` when it equals the target's).
    plain: [Option<wgpu::RenderPipeline>; 2],
    /// The union-shaped pipelines by format index — built on first use.
    union: [Option<wgpu::RenderPipeline>; 2],
}

/// All render-thread state.
pub struct GpuRenderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    presenter: Option<present::Presenter>,
    /// The plane realization of every surface whose target exposes a
    /// system-compositor parent. Platform objects stay on the render
    /// thread, outside the surface states lowering moves to its workers.
    #[cfg_attr(
        not(any(target_vendor = "apple", target_os = "android")),
        expect(
            clippy::zero_sized_map_values,
            reason = "no plane realization exists on this platform, so the map stays empty"
        )
    )]
    planes: FxHashMap<SurfaceId, planes::Platform>,
    /// The surfaces this frame admits to a plane-only refresh — cleared
    /// and refilled at the top of every render, so an admission can never
    /// outlive the frame that made it (#90).
    plane_only: Vec<SurfaceId>,
    /// Workspace a pose frame's plan check reuses, including the plan it
    /// rebuilds, so a steady pose allocates nothing.
    plan_scratch: planes::PlanScratch,
    next_plan: planes::Plan,
    /// The candidate map each promotion check or plan fills and reuses.
    candidates: FxHashMap<LayerId, planes::Candidate>,
    candidate_frames: FxHashMap<LayerId, (ExternalFrame, u64)>,
    /// The per-surface ready-candidate sets `ready_planes` fills for the
    /// frame's lowered batch — kept between renders so a plane prepare
    /// allocates nothing steady-state.
    ready_sets: Vec<FxHashSet<LayerId>>,
    /// The ready-candidate set `plane_only_frames` fills with the
    /// surface's current readiness — the filter the committed plan saw.
    ready: FxHashSet<LayerId>,
    /// How the fixed modules reach this device (`shaders.rs`): SPIR-V,
    /// metallib, or WGSL — decided once at init by the adapter backend.
    shader_delivery: shaders::ShaderDelivery,
    shaders: paint::Registry,
    filters: filter::Registry,
    /// The silhouette-morphology pipeline — `None` until a frame actually
    /// composes a shadow, so shadow-free engines pay nothing (#170).
    shadow_blur: Option<shadow::Blur>,
    /// The reduced-capture resolve pipelines — `None` until a frame
    /// resolves a backdrop group captured below device resolution.
    resolve: Option<resolve::Pipelines>,
    /// The capture pyramid reduce pipelines — `None` until a frame
    /// reduces a levelled capture.
    reduce: Option<reduce::Pipelines>,
    last_frame: Option<Instant>,
    origin: Option<Instant>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Core pipelines keyed by target format, blend mode and shader
    /// variant: `[format][replace][variant]` where the format index is
    /// `usize::from(format != TARGET_FORMAT)` — when surface and scratch
    /// share `Rgba16Float`, both draw through slot 0, so identical
    /// pipelines are built once (#170). A `None` cell is created at first
    /// use — preparation is measured on the frame that needs it (#170 B3).
    /// `[space][format][ShaderVariant]` — a fourth variant slot holds
    /// the union-field pipelines for backdrop-union members. Slot 3 is
    /// never built at init: the variant-3 module and pipeline are
    /// created on the first union or outer-band member (on wasm, by
    /// `prepare_union_pipelines` before the frame that needs them).
    pipelines: [[[Option<wgpu::RenderPipeline>; 4]; 2]; 2],
    /// `[opaque, partial]` coverage pipelines, same cell policy as the
    /// core set: built at init under `core_pipelines_eager()` (always,
    /// on wasm), else created at the first coverage pass (#170 B3, #49).
    coverage_pipelines: [Option<wgpu::RenderPipeline>; 2],
    /// The pipeline layout every core and backdrop pipeline shares.
    pipeline_layout: wgpu::PipelineLayout,
    #[cfg(target_vendor = "apple")]
    tile_executor: composite::metal::Executor,
    /// The configured pipeline cache, opened once for the whole set.
    pipeline_cache: Option<wgpu::PipelineCache>,
    /// Registered backdrop effect shaders, by raw id — see
    /// [`BackdropShaderEntry`].
    backdrop_shaders: FxHashMap<u64, BackdropShaderEntry>,
    /// The union-linked engine module, created on the first frame that
    /// draws a union or outer-band member (async preparation cannot
    /// run inside the synchronous encode loop, so the module materializes
    /// in `prepare_union_pipelines`).
    #[cfg(target_arch = "wasm32")]
    union_module: Option<wgpu::ShaderModule>,
    /// The configured isolation texture format.
    scratch_format: wgpu::TextureFormat,
    layout0: wgpu::BindGroupLayout,
    layout1: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    stops: wgpu::Buffer,
    /// `{0, 1, 2, 2, 1, 3}` — see `quad_index_buffer`.
    quad_indices: wgpu::Buffer,
    bind0: wgpu::BindGroup,
    /// The atlas generation `bind0` was built against.
    bound_atlas: u64,
    /// The buffer sizes `bind0` was built against.
    bound_instance_size: u64,
    /// The stop buffer size `bind0` was built against.
    bound_stop_size: u64,
    /// The globals buffer size `bind0` was built against.
    bound_globals_size: u64,
    atlas: Atlas,
    /// Per-commit cell writes, rebuilt in place each frame.
    commit_writes: Vec<glyph::CellWrite>,
    /// Per-surface pending origins, rebuilt in place each apply.
    pending_origins: Vec<PendingOrigin>,
    /// Replay-pin slots, reused across evicting commits (#119).
    commit_touches: Vec<u32>,
    /// A dummy 1×1 view for unused group-1 slots.
    dummy_view: wgpu::TextureView,
    /// A dummy 1×1 `u32` view for unused external-frame plane slots.
    dummy_uint_view: wgpu::TextureView,
    /// The external-frame group-1 layout, `None` until a surface first
    /// draws an external frame.
    ext_layout: Option<wgpu::BindGroupLayout>,
    /// `[format index]` external pipelines — the same slot indexing as
    /// `pipelines`; `None` until external frames are used (#170: prepared
    /// at `set_external_frame` registration, never at idle).
    external_pipes: [Option<wgpu::RenderPipeline>; 2],
    /// The Vulkan native external-frame context (issue #166): descriptors,
    /// render passes and pipelines for multiplanar and external-format
    /// frames, built when the first native frame is registered (#170).
    #[cfg(all(unix, not(target_vendor = "apple")))]
    native: Option<external::vulkan::Native>,
    /// Why the first `set_external_frame` failed to build the native
    /// context, so a later native-frame draw can report it.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    native_error: Option<String>,
    /// Serializes "stage queue waits → submit" so an unrelated submission
    /// cannot consume a staged producer semaphore wait (#166); shared with
    /// the plane realizations and the dma-buf pools, which submit with
    /// their own waits.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    submit_lock: std::sync::Arc<std::sync::Mutex<()>>,
    /// The bounded export pool behind every `DmabufTarget` surface —
    /// the Linux counterpart of `planes` (#1687). Empty on every other
    /// platform, where no exportable pool type exists.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(
            clippy::zero_sized_map_values,
            reason = "no export pool exists on this platform, so the map stays empty"
        )
    )]
    exports: FxHashMap<SurfaceId, ExportPool>,
    surfaces: FxHashMap<SurfaceId, SurfaceState>,
    /// Every live [`GpuProducer`](cherenkov::GpuProducer) by its
    /// `ProducerId`: renderer-scoped, shared by bindings on any surface.
    producers: FxHashMap<ProducerId, gpu_content::Producer>,
    /// Retired before its registration reached the stream: a handle
    /// created and dropped while the add was still queued sends its
    /// retirement through the producer's own queue, which can beat the
    /// add to this thread. The add skips a `ProducerId` found here.
    pending_retire: FxHashSet<ProducerId>,
    fonts: FxHashMap<u64, FontData>,
    /// Registered images.
    images: FxHashMap<u64, GpuImage>,
    bitmaps: FxHashMap<BitmapKey, GpuBitmap>,
    /// Bumped on every `images` insert/remove — every cached group-1
    /// bind group samples an image view, so an image change rebuilds them.
    images_gen: u64,
    /// Bumped on every image replacement: the one change to what a
    /// current local image read that arrives without a tree edit. Adding
    /// an image changes nothing drawn, and a removal only runs once no
    /// installed content draws the image (#199), so neither counts.
    image_replacements: u64,
    timestamps: bool,
    query_set: Option<wgpu::QuerySet>,
    /// First query of the active frame's independent range.
    query_base: u32,
    /// Free frame ranges: (set, first query, capacity). Sharing a set keeps
    /// many frames in flight without exhausting Metal's sample-buffer limit.
    query_pool: Vec<(wgpu::QuerySet, u32, u32)>,
    /// Frames still writing their pass-boundary samples on the GPU.
    pending_queries: VecDeque<PendingQueries>,
    /// Last draw submission of the current frame.
    frame_submission: Option<wgpu::SubmissionIndex>,
    /// Staging ring for the frame-wide instance, stop and globals uploads.
    uploads: upload::Uploads,
    query_buffer: Option<wgpu::Buffer>,
    /// Readback buffers recycled between frames; a frame's resolve owns
    /// one until its samples are read.
    query_staging: Vec<wgpu::Buffer>,
    /// Capacity of one frame's query range; pass `i` writes `base + 2i`
    /// at its start and `base + 2i + 1` at its end. GPU time runs from the
    /// first pass's start to the last pass's end: pass boundaries are the
    /// one timestamp position every backend with `TIMESTAMP_QUERY`
    /// supports (Metal on Apple GPUs samples only at stage boundaries).
    query_capacity: u32,
    /// Frames whose timestamp resolve was submitted but whose staging
    /// buffer is not mapped yet — read on a later call, in order.
    pending_timestamps: VecDeque<PendingTimestamps>,
    /// Resolved timings retained until `finish_timings`.
    timings: Vec<FrameTiming>,
    /// Passes encoded this frame, for the per-pass report.
    frame_pass_count: u32,
    /// `(name, width, height, format)` of each encoded pass this frame.
    pass_meta: Vec<PassMeta>,
    /// Window of every native GPU wait and the deadline of every browser
    /// wait; see [`GpuConfig::wait_timeout`].
    wait_timeout: Duration,
    /// The queue's retirement frontier; every native wait reads it between
    /// windows to tell a slow queue (still retiring) from a wedged one.
    tracker: Arc<SubmissionTracker>,
    max_texture: u32,
    /// The allocation-event diagnostic sink (issue #169); `None` in
    /// timed runs.
    diag: Option<diag::Sink>,
    /// Kept so registered-effect pipelines use the same pipeline cache.
    config: GpuConfig,
    /// The projective composite and mip pipelines (#84), `None` until a
    /// frame first composes a projective layer.
    projective: Option<projective::Pipelines>,
    /// Counts rendered frames; the projective cache's recency clock.
    frame_count: u64,
}

impl std::fmt::Debug for GpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuRenderer")
            .field("instance", &self.instance)
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
}

/// The renderer-wide resources a surface's lowering reads.
#[derive(Clone, Copy)]
struct GlyphResources<'a> {
    atlas: &'a Atlas,
    fonts: &'a FxHashMap<u64, FontData>,
    images: &'a FxHashMap<u64, GpuImage>,
    bitmaps: &'a FxHashMap<BitmapKey, GpuBitmap>,
    /// The producers' current frames by id — every binding samples the
    /// one its producer holds. The producers themselves stay on the
    /// render thread: a rendered producer's content is `!Sync`.
    producers: &'a FxHashMap<ProducerId, &'a external::Slot>,
}

/// Atlas origins produced by one deferred raster.
/// The batch commit's outcome for `lower_all`'s retry loop (#169 A3).
enum Commit {
    /// Every surface's pending rasters committed; a surface that could
    /// not be placed got `AtlasExhausted` in its result.
    Done,
    /// Resize the atlas once to this `(edge, pages)` and re-lower.
    Resize(u32, u32),
}

enum PendingOrigin {
    /// Cell origins — one for a glyph, one per cell for a path
    /// emission — and the shelves the admission's cells live on, so
    /// patched emissions can reference the bands (#119).
    Cells(Vec<(u32, u32)>, Vec<u32>),
    /// A clip mask's cell origin.
    Mask([f32; 2]),
    /// A COLR cache insert; no instance patch.
    None,
}

/// A frame owns its query range until its samples have been resolved and read.
/// Resolving is encoded only after the draw completion callback fires: on
/// newer Apple GPUs an earlier Metal blit resolve can see incomplete end
/// samples even with an explicit GPU fence or event.
struct PendingQueries {
    frame: FrameId,
    submission: wgpu::SubmissionIndex,
    query_set: wgpu::QuerySet,
    base: u32,
    capacity: u32,
    count: u32,
    meta: Vec<PassMeta>,
    complete: Arc<AtomicU8>,
}

/// One submitted frame's timestamp queries awaiting GPU completion.
///
/// The resolve and copy are submitted after the frame's draws complete.
/// The map and read happen on a later non-blocking poll, so rendering never
/// stalls on GPU idle. The query range cannot be reused until this readback
/// completes.
struct PendingTimestamps {
    /// Samples remain owned by this frame until the resolve copy completes.
    query_set: wgpu::QuerySet,
    query_base: u32,
    query_capacity: u32,
    /// The frame these queries measure.
    frame: FrameId,
    /// The submission carrying the resolve and the copy into `staging`.
    submission: wgpu::SubmissionIndex,
    staging: wgpu::Buffer,
    /// Queries resolved: `2 * passes`.
    count: u32,
    /// Pass metadata for the per-pass report.
    meta: Vec<PassMeta>,
    /// Whether `map_async` was requested for `staging`.
    map_requested: bool,
    /// Set by the map callback: 1 once the copy is readable, 2 on a
    /// failed map.
    ready: Arc<AtomicU8>,
}

impl PendingTimestamps {
    /// Requests the map of `staging`; the callback records the outcome
    /// in `ready`.
    fn request_map(&mut self) {
        if self.map_requested {
            return;
        }
        let flag = Arc::clone(&self.ready);
        self.staging.slice(..u64::from(self.count) * 8).map_async(
            wgpu::MapMode::Read,
            move |result| {
                flag.store(u8::from(result.is_err()) + 1, Ordering::Relaxed);
            },
        );
        self.map_requested = true;
    }
}

/// One encoded pass's report metadata.
struct PassMeta {
    name: String,
    width: u32,
    height: u32,
    format: &'static str,
}

/// Recreates a frame-wide buffer at `size` bytes. Its contents need not
/// survive: every dirty surface's slice is copied in from the upload ring
/// at the start of the frame's first submission, after all growth.
fn grow_buffer(
    device: &wgpu::Device,
    label: &'static str,
    old: &wgpu::Buffer,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    tracing::debug!(label, from = old.size(), to = size, "buffer grown");
    let new = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    });
    diag::grow(
        device,
        label,
        diag::Class::for_label(label),
        old.size(),
        size,
        0,
        false,
    );
    new
}

/// Reports `phase` to the configured [`GpuConfig::creation_probe`]; the
/// probe returning `true` aborts creation — the #170 ledger's ablation
/// stop.
fn fire_probe(
    config: &GpuConfig,
    phase: CreationPhase,
    adapter: Option<&wgpu::Adapter>,
    device: Option<&wgpu::Device>,
) -> Result<(), EngineError> {
    let Some(probe) = &config.creation_probe else {
        return Ok(());
    };
    if probe.fire(&CreationPoint {
        phase,
        adapter,
        device,
    }) {
        return Err(EngineError::Backend(format!(
            "creation probe stopped after {phase:?}"
        )));
    }
    Ok(())
}

/// Creates an adapter plus device. Fails when no adapter allows the target
/// format's required usages.
#[cfg(not(target_arch = "wasm32"))]
fn create_device(
    config: &GpuConfig,
) -> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), EngineError> {
    if let Some(shared) = &config.device {
        return Ok((
            shared.instance.clone(),
            shared.adapter.clone(),
            shared.device.clone(),
            shared.queue.clone(),
        ));
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: config.backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    fire_probe(config, CreationPhase::Instance, None, None)?;
    let adapters = pollster::block_on(instance.enumerate_adapters(config.backends));
    let adapter = adapters
        .into_iter()
        .find(|a| {
            a.get_texture_format_features(TARGET_FORMAT)
                .allowed_usages
                .contains(TARGET_USAGES)
        })
        .ok_or_else(|| EngineError::Backend("no adapter".into()))?;
    fire_probe(config, CreationPhase::Adapter, Some(&adapter), None)?;
    let supported = adapter.features();
    let info = adapter.get_info();
    tracing::info!(
        name = %info.name,
        backend = ?info.backend,
        device_type = ?info.device_type,
        driver = %info.driver,
        driver_info = %info.driver_info,
        timestamp_query = supported.contains(wgpu::Features::TIMESTAMP_QUERY),
        timestamps_inside_encoders =
            supported.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
        timestamps_inside_passes = supported.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
        "adapter"
    );
    tracing::debug!(limits = ?adapter.limits(), "adapter limits");
    let mut required = wgpu::Features::empty();
    // The fixed engine shaders are precompiled (#57): Vulkan loads SPIR-V
    // and Metal a metallib through the passthrough API; wgpu-hal advertises
    // the feature unconditionally on both backends.
    if matches!(info.backend, wgpu::Backend::Vulkan | wgpu::Backend::Metal)
        && supported.contains(wgpu::Features::PASSTHROUGH_SHADERS)
    {
        required |= wgpu::Features::PASSTHROUGH_SHADERS;
    }
    // Only pass-boundary timestamps are requested: Metal on Apple GPUs
    // advertises `TIMESTAMP_QUERY_INSIDE_ENCODERS` but samples only at
    // stage boundaries, so an encoder-level `write_timestamp` goes through
    // a dummy blit encoder that wgpu itself documents as unreliable.
    if config.timestamps && supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
        required |= wgpu::Features::TIMESTAMP_QUERY;
    }
    if config.pipeline_cache.is_some() && supported.contains(wgpu::Features::PIPELINE_CACHE) {
        required |= wgpu::Features::PIPELINE_CACHE;
    }
    let limits = wgpu::Limits::default().or_worse_values_from(&adapter.limits());
    // On Vulkan, device creation is where external-memory, semaphore and
    // YCbCr capabilities are enabled — the `open_with_callback` hook adds
    // the extensions/features wgpu does not request, preserving every
    // requirement wgpu does (#166). A non-Vulkan adapter falls through to
    // `request_device` unchanged.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    if info.backend == wgpu::Backend::Vulkan {
        let (device, queue) = create_vulkan_device(&adapter, required, &limits)?;
        fire_probe(config, CreationPhase::Device, Some(&adapter), Some(&device))?;
        tracing::info!(features = ?device.features(), "device");
        return Ok((instance, adapter, device, queue));
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("cherenkov-gpu"),
        required_features: required,
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: memory_hints(info.backend),
        trace: wgpu::Trace::Off,
    }))
    .map_err(|e| EngineError::Backend(format!("{e}")))?;
    fire_probe(config, CreationPhase::Device, Some(&adapter), Some(&device))?;
    tracing::info!(features = ?device.features(), "device");
    Ok((instance, adapter, device, queue))
}

/// `create_device`'s Vulkan branch (issue #166): opens the device through
/// `open_with_callback` so the external-frame extensions and the YCbCr
/// sampler-conversion feature bit are enabled at creation — first import
/// would be too late — then hands the opened device back through
/// `create_device_from_hal` with wgpu's requirements intact.
#[cfg(all(unix, not(target_vendor = "apple"), not(target_arch = "wasm32")))]
fn create_vulkan_device(
    adapter: &wgpu::Adapter,
    features: wgpu::Features,
    limits: &wgpu::Limits,
) -> Result<(wgpu::Device, wgpu::Queue), EngineError> {
    use external::vulkan;
    // SAFETY: `adapter` is the engine's own wgpu adapter; `as_hal`
    // borrows it for this scope and reports `None` off-Vulkan.
    let Some(hal_adapter) = (unsafe { adapter.as_hal::<wgpu::hal::vulkan::Api>() }) else {
        return Err(EngineError::Backend("adapter is not Vulkan".into()));
    };
    let caps = hal_adapter.physical_device_capabilities();
    let extensions: Vec<&'static core::ffi::CStr> = vulkan::extra_device_extensions()
        .into_iter()
        .filter(|ext| caps.supports_extension(ext))
        .collect();
    // The YCbCr conversion feature is keyed to its extension's presence.
    let ycbcr = caps.supports_extension(ash::khr::sampler_ycbcr_conversion::NAME);
    // `create_info.p_next` points at this struct until `vkCreateDevice`
    // runs inside `open_with_callback`; the `FnOnce` callback is dropped
    // before that call, so the struct is boxed in this scope instead of
    // being captured by the closure.
    let ycbcr_features = Box::new(
        ash::vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default()
            .sampler_ycbcr_conversion(true),
    );
    let callback: Option<Box<wgpu::hal::vulkan::CreateDeviceCallback<'_>>> =
        if ycbcr || !extensions.is_empty() {
            let ycbcr_ptr = core::ptr::from_ref(&*ycbcr_features).cast::<core::ffi::c_void>();
            Some(Box::new(
                move |args: wgpu::hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>| {
                    for ext in &extensions {
                        if !args.extensions.contains(ext) {
                            args.extensions.push(ext);
                        }
                    }
                    if ycbcr {
                        args.create_info.p_next = ycbcr_ptr;
                    }
                },
            ))
        } else {
            None
        };
    let hints = memory_hints(wgpu::Backend::Vulkan);
    // SAFETY: `callback` pins `ycbcr_features` in this scope, which
    // outlives the `vkCreateDevice` call inside `open_with_callback`
    // (the `FnOnce` runs there, not later).
    let opened = unsafe { hal_adapter.open_with_callback(features, limits, &hints, callback) }
        .map_err(|e| EngineError::Backend(format!("vulkan device creation: {e}")))?;
    drop(hal_adapter);
    // SAFETY: `opened` came from `hal_adapter.open_with_callback` on
    // this same adapter just above — the pairing contract.
    let (device, queue) = unsafe {
        adapter.create_device_from_hal::<wgpu::hal::vulkan::Api>(
            opened,
            &wgpu::DeviceDescriptor {
                label: Some("cherenkov-gpu"),
                required_features: features,
                required_limits: limits.clone(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: hints,
                trace: wgpu::Trace::Off,
            },
        )
    }
    .map_err(|e| EngineError::Backend(format!("vulkan device from hal: {e}")))?;
    Ok((device, queue))
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::interop::SharedDevice {
    /// Creates the adapter and device the GPU engine would use for `config`.
    ///
    /// Pass the result back through [`GpuConfig::device`] to drive the engine
    /// with this exact device.
    ///
    /// # Errors
    /// [`EngineError`] when no adapter allows the target format or device
    /// creation fails.
    pub fn create(config: &GpuConfig) -> Result<Self, EngineError> {
        let (instance, adapter, device, queue) = create_device(config)?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
impl crate::interop::SharedDevice {
    /// As the native [`SharedDevice::create`](Self::create), awaited on
    /// wasm32 so the JS thread is not blocked.
    ///
    /// # Errors
    /// [`EngineError`] when no adapter allows the target format or device
    /// creation fails.
    pub async fn create(config: &GpuConfig) -> Result<Self, EngineError> {
        let (instance, adapter, device, queue) = create_device(config).await?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn create_device(
    config: &GpuConfig,
) -> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), EngineError> {
    if let Some(shared) = &config.device {
        return Ok((
            shared.instance.clone(),
            shared.adapter.clone(),
            shared.device.clone(),
            shared.queue.clone(),
        ));
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: config.backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    fire_probe(config, CreationPhase::Instance, None, None)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: config.power_preference,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
            compatible_surface: None,
        })
        .await
        .map_err(|error| EngineError::Backend(error.to_string()))?;
    if !adapter
        .get_texture_format_features(TARGET_FORMAT)
        .allowed_usages
        .contains(TARGET_USAGES)
    {
        return Err(EngineError::Backend(
            "adapter cannot render the working-space format".into(),
        ));
    }
    fire_probe(config, CreationPhase::Adapter, Some(&adapter), None)?;
    let supported = adapter.features();
    let info = adapter.get_info();
    tracing::info!(
        name = %info.name,
        backend = ?info.backend,
        device_type = ?info.device_type,
        driver = %info.driver,
        driver_info = %info.driver_info,
        timestamp_query = supported.contains(wgpu::Features::TIMESTAMP_QUERY),
        timestamps_inside_encoders =
            supported.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
        timestamps_inside_passes = supported.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
        "adapter"
    );
    tracing::debug!(limits = ?adapter.limits(), "adapter limits");
    let mut required = wgpu::Features::empty();
    // Only pass-boundary timestamps are requested: Metal on Apple GPUs
    // advertises `TIMESTAMP_QUERY_INSIDE_ENCODERS` but samples only at
    // stage boundaries, so an encoder-level `write_timestamp` goes through
    // a dummy blit encoder that wgpu itself documents as unreliable.
    if config.timestamps && supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
        required |= wgpu::Features::TIMESTAMP_QUERY;
    }
    if config.pipeline_cache.is_some() && supported.contains(wgpu::Features::PIPELINE_CACHE) {
        required |= wgpu::Features::PIPELINE_CACHE;
    }
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("cherenkov-gpu"),
            required_features: required,
            // Clamp the portable defaults to what the adapter reports:
            // iOS Metal offers 15 inter-stage varyings (60 components)
            // where `Limits::default` asks for 16.
            required_limits: wgpu::Limits::default().or_worse_values_from(&adapter.limits()),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: memory_hints(info.backend),
            trace: wgpu::Trace::Off,
        })
        .await
        .map_err(|e| EngineError::Backend(format!("{e}")))?;
    fire_probe(config, CreationPhase::Device, Some(&adapter), Some(&device))?;
    tracing::info!(features = ?device.features(), "device");
    Ok((instance, adapter, device, queue))
}

/// Maps a layout-table entry to the wgpu descriptor. The same table feeds
/// `build.rs`'s slot assignment, so a precompiled shader cannot drift from
/// the layout built here.
pub fn layout_entries(entries: &[bindings::Entry]) -> Vec<wgpu::BindGroupLayoutEntry> {
    entries
        .iter()
        .map(|entry| wgpu::BindGroupLayoutEntry {
            binding: entry.binding,
            visibility: wgpu::ShaderStages::from_bits_retain(u32::from(entry.stages)),
            ty: match entry.kind {
                bindings::Kind::Uniform => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: entry.dynamic_offset,
                    min_binding_size: wgpu::BufferSize::new(entry.min_size),
                },
                bindings::Kind::StorageRead => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: entry.dynamic_offset,
                    min_binding_size: wgpu::BufferSize::new(entry.min_size),
                },
                bindings::Kind::Texture => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                bindings::Kind::TextureUint => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                bindings::Kind::Sampler => {
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                }
            },
            count: None,
        })
        .collect()
}

/// The bind group layouts: group 0 is engine data, group 1 the texture a
/// composite samples.
fn create_layouts(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    let layout0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("engine data"),
        entries: &layout_entries(bindings::ENGINE_GROUP0),
    });
    let layout1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("source texture"),
        // 0: composite source, 1: blend backdrop, 2: image paint,
        // 3: clip mask texture.
        entries: &layout_entries(bindings::ENGINE_GROUP1),
    });
    (layout0, layout1)
}

/// Resolves the image the canonical draw recorded.
fn image_view<'a>(
    source: &lower::ImageSource,
    images: &'a FxHashMap<u64, GpuImage>,
    bitmaps: &'a FxHashMap<BitmapKey, GpuBitmap>,
    shaders: &'a FxHashMap<std::sync::Arc<paint::Key>, paint::Texture>,
) -> &'a wgpu::TextureView {
    match source {
        lower::ImageSource::Registered(id) => {
            &images
                .get(id)
                .unwrap_or_else(|| panic!("a lowered range samples unregistered image {id}"))
                .view
        }
        lower::ImageSource::Bitmap(key) => &bitmaps.get(key).expect("prepared bitmap").image.view,
        lower::ImageSource::Shader(key) => &shaders[key].image.view,
        lower::ImageSource::Content(_) => {
            unreachable!("producer ranges bind the external pipeline")
        }
    }
}

/// Builds a group-1 bind group; `None` binds the dummy view.
fn make_bind1(
    device: &wgpu::Device,
    layout1: &wgpu::BindGroupLayout,
    dummy: &wgpu::TextureView,
    source: Option<&wgpu::TextureView>,
    backdrop: Option<&wgpu::TextureView>,
    image: Option<&wgpu::TextureView>,
    mask: Option<&wgpu::TextureView>,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("source texture"),
        layout: layout1,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source.unwrap_or(dummy)),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(backdrop.unwrap_or(dummy)),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(image.unwrap_or(dummy)),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(mask.unwrap_or(dummy)),
            },
        ],
    })
}

/// Builds the group-0 bind group over the current buffers and atlas.
fn make_bind0(
    device: &wgpu::Device,
    layout0: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    instances: &wgpu::Buffer,
    stops: &wgpu::Buffer,
    atlas: &Atlas,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("engine data"),
        layout: layout0,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                // One Globals window; the dynamic offset selects
                // the pass's slot inside the buffer.
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: globals,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<instance::Globals>() as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: stops.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(atlas.view()),
            },
        ],
    })
}

/// `view`'s `VkImageView` — the descriptor-set write operand for the
/// native operation (#166).
#[cfg(all(unix, not(target_vendor = "apple")))]
/// Finishes `encoder` into a command buffer and replaces it with a fresh
/// one: wgpu forbids mixing its encoding API with raw `as_hal_mut` access
/// on a single encoder, so native work is spliced between finished wgpu
/// buffers inside the one ordered submission.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn split_encoder(encoder: &mut wgpu::CommandEncoder, device: &wgpu::Device) -> wgpu::CommandBuffer {
    std::mem::replace(
        encoder,
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        }),
    )
    .finish()
}

/// `view`'s `VkImageView`.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn raw_vk_view(view: &wgpu::TextureView) -> ash::vk::ImageView {
    // SAFETY: `view` is a live wgpu view on the Vulkan backend —
    // asserted by `expect` — borrowed for this scope.
    let hal = unsafe { view.as_hal::<wgpu::hal::vulkan::Api>() };
    // SAFETY: `hal` is the view's own hal object, live above.
    unsafe { hal.expect("vulkan").raw_handle() }
}

/// `buffer`'s `VkBuffer`.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn raw_vk_buffer(buffer: &wgpu::Buffer) -> ash::vk::Buffer {
    // SAFETY: `buffer` is a live wgpu buffer on the Vulkan backend —
    // asserted by `expect` — borrowed for this scope.
    let hal = unsafe { buffer.as_hal::<wgpu::hal::vulkan::Api>() };
    // SAFETY: `hal` is the buffer's own hal object, live above.
    unsafe { hal.expect("vulkan").raw_handle() }
}

/// `format` as its `vk::Format`, queried through the hal adapter.
#[cfg(all(unix, not(target_vendor = "apple")))]
impl GpuRenderer {}

/// The closed pipeline set: instanced-quad pipelines from `shader.wgsl`,
/// specialised per fragment variant.
const fn variant_index(variant: ShaderVariant) -> usize {
    match variant {
        ShaderVariant::Simple => 0,
        ShaderVariant::Shadow => 1,
        ShaderVariant::Full => 2,
        ShaderVariant::Union => 3,
    }
}

/// #170's measured pipeline policy (`CoreEager`): the deduplicated core
/// set is built at creation. The Pixel A/B rejected demand-mode creation:
/// the first drawn frame's encode phase paid 41–561 ms for the pipelines
/// it missed (vs 3–16 ms steady). wasm is always eager — it has no demand
/// call site at all, since its pipeline creation is async and cannot run
/// inside the synchronous encode loop.
#[cfg(not(target_arch = "wasm32"))]
const fn core_pipelines_eager() -> bool {
    true
}

/// Allocator policy for the backend a device is opened on (#170).
///
/// `Manual { 4 MiB..16 MiB }` was measured only on Vulkan (Mali-G715,
/// Pixel 9 Pro): 10.4 MiB idle, where `Performance`'s 64 MiB first block
/// is most of the ~74 MiB idle reservation. The 16 MiB cap bounds a large
/// scene (map: five blocks for 53 MiB). Vulkan reads the hint in
/// `AllocationSizes::from_memory_hints`
/// (`wgpu-hal/src/vulkan/adapter.rs:2951`, rev `4b35d8bc`).
///
/// Every other class keeps [`wgpu::MemoryHints::Performance`], the
/// default (`wgpu-types/src/device.rs:59`). At that revision Metal
/// (`wgpu-hal/src/metal/adapter.rs:72`) and GLES
/// (`wgpu-hal/src/gles/adapter.rs:1083`) take `_memory_hints` and never
/// read it. The WebGPU backend never reads it: `request_device`
/// (`wgpu/src/backend/webgpu.rs:1772`) copies limits, features, and the
/// label, and that module has no `memory_hints` use. DX12 does read the
/// hint (`wgpu-hal/src/dx12/adapter.rs:1062`, through `device.rs:64` into
/// `AllocationSizes::from_memory_hints` at `suballocation.rs:78`) but that
/// class is unmeasured, so it stays on the default.
const fn memory_hints(backend: wgpu::Backend) -> wgpu::MemoryHints {
    const MIB: u64 = 1024 * 1024;
    match backend {
        wgpu::Backend::Vulkan => wgpu::MemoryHints::Manual {
            suballocated_device_memory_block_size: 4 * MIB..16 * MIB,
        },
        wgpu::Backend::Noop
        | wgpu::Backend::Metal
        | wgpu::Backend::Dx12
        | wgpu::Backend::Gl
        | wgpu::Backend::BrowserWebGpu => wgpu::MemoryHints::Performance,
    }
}

/// The pipeline layout every core and backdrop pipeline shares, over
/// bind groups 0 and 1 — one object serves the whole set (#170).
fn create_pipeline_layout(
    device: &wgpu::Device,
    layout0: &wgpu::BindGroupLayout,
    layout1: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cherenkov"),
        bind_group_layouts: &[Some(layout0), Some(layout1)],
        immediate_size: 0,
    })
}

/// The configured pipeline cache, opened once for the whole pipeline
/// set — it was previously re-read and re-opened per pipeline (#170).
fn open_pipeline_cache(device: &wgpu::Device, config: &GpuConfig) -> Option<wgpu::PipelineCache> {
    let path = config.pipeline_cache.as_ref()?;
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return None;
    }
    let data = std::fs::read(path).ok();
    // SAFETY: `data` is either a blob previously produced by wgpu or
    // absent; `fallback: true` keeps us off the unsafe fallback path.
    Some(unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("cherenkov"),
            data: data.as_deref(),
            fallback: true,
        })
    })
}

/// Persists the pipeline cache to its configured path, best effort:
/// future runs start closer to warm; a failed write only means the next
/// start recompiles.
fn persist_pipeline_cache(cache: Option<&wgpu::PipelineCache>, config: &GpuConfig) {
    if let (Some(cache), Some(path)) = (cache, &config.pipeline_cache)
        && let Some(data) = cache.get_data()
    {
        let _ = std::fs::write(path, data);
    }
}

/// The closed pipeline set: one instanced-quad pipeline from `shader.wgsl`
/// on the shared pipeline layout.
#[cfg(not(target_arch = "wasm32"))]
fn create_pipeline(
    device: &wgpu::Device,
    config: &GpuConfig,
    layout: &wgpu::PipelineLayout,
    cache: Option<&wgpu::PipelineCache>,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    mode: CoveragePass,
) -> Result<wgpu::RenderPipeline, EngineError> {
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cherenkov"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(mode.vertex()),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(mode.fragment()),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: mode.blend(),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: mode.topology(),
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: mode.depth(),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache,
    });
    if let Some(error) = pollster::block_on(error_scope.pop()) {
        return Err(EngineError::Backend(format!("{error}")));
    }
    persist_pipeline_cache(cache, config);
    Ok(pipeline)
}

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn create_pipeline(
    device: &wgpu::Device,
    config: &GpuConfig,
    layout: &wgpu::PipelineLayout,
    cache: Option<&wgpu::PipelineCache>,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    mode: CoveragePass,
) -> Result<wgpu::RenderPipeline, EngineError> {
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cherenkov"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(mode.vertex()),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(mode.fragment()),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: mode.blend(),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: mode.topology(),
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: mode.depth(),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache,
    });
    if let Some(error) = error_scope.pop().await {
        return Err(EngineError::Backend(format!("{error}")));
    }
    persist_pipeline_cache(cache, config);
    Ok(pipeline)
}

/// The objects a demand-created core pipeline needs — field-borrowed out
/// of `GpuRenderer` so the encode loop's surface borrow composes.
struct PipelineFactory<'a> {
    device: &'a wgpu::Device,
    config: &'a GpuConfig,
    layout: &'a wgpu::PipelineLayout,
    cache: Option<&'a wgpu::PipelineCache>,
    delivery: shaders::ShaderDelivery,
}

impl PipelineFactory<'_> {
    /// Creates the `(format, mode, variant)` core or coverage pipeline
    /// on demand: the shader module is built and released around the
    /// call — modules are not retained once pipelines exist (#170).
    #[cfg(not(target_arch = "wasm32"))]
    fn create(
        &self,
        format: wgpu::TextureFormat,
        mode: CoveragePass,
        variant: ShaderVariant,
    ) -> Result<wgpu::RenderPipeline, RenderError> {
        let module = self
            .delivery
            .engine_module(self.device, variant_index(variant));
        create_pipeline(
            self.device,
            self.config,
            self.layout,
            self.cache,
            &module,
            format,
            mode,
        )
        .map_err(|error| RenderError::Render(format!("core pipeline: {error}")))
    }

    /// wasm always prepares the core set eagerly: its pipeline creation
    /// is async and cannot run inside the synchronous encode loop.
    #[cfg(target_arch = "wasm32")]
    fn create(
        &self,
        format: wgpu::TextureFormat,
        mode: CoveragePass,
        variant: ShaderVariant,
    ) -> Result<wgpu::RenderPipeline, RenderError> {
        let _ = (
            self.device,
            self.config,
            self.layout,
            self.cache,
            self.delivery,
            format,
            mode,
            variant,
        );
        Err(RenderError::Render(
            "a core pipeline was not prepared at init".into(),
        ))
    }
}

/// The core pipeline for `(format, replace, variant)` — the format-slot
/// index `usize::from(format != TARGET_FORMAT)` shares pipelines across
/// surface and scratch when their formats coincide, and an empty cell is
/// created at first use so preparation lands inside the frame that needs
/// it (#170).
fn core_pipeline<'m>(
    pipelines: &'m mut [[[Option<wgpu::RenderPipeline>; 4]; 2]; 2],
    factory: &PipelineFactory<'_>,
    format: wgpu::TextureFormat,
    replace: bool,
    variant: ShaderVariant,
) -> Result<&'m wgpu::RenderPipeline, RenderError> {
    let cell = &mut pipelines[usize::from(format != TARGET_FORMAT)][usize::from(replace)]
        [variant_index(variant)];
    if cell.is_none() {
        *cell = Some(factory.create(format, CoveragePass::Painter(replace), variant)?);
    }
    Ok(cell.as_ref().expect("core pipeline ready"))
}

/// The `[opaque, partial]` coverage pipeline for the surface target —
/// the same cell policy as `core_pipeline`: an empty cell is created at
/// the first coverage pass that needs it (#170 B3, #49).
fn coverage_pipeline<'m>(
    pipelines: &'m mut [Option<wgpu::RenderPipeline>; 2],
    factory: &PipelineFactory<'_>,
    phase: CoveragePhase,
) -> Result<&'m wgpu::RenderPipeline, RenderError> {
    let cell = &mut pipelines[phase as usize];
    if cell.is_none() {
        *cell = Some(factory.create(
            TARGET_FORMAT,
            CoveragePass::Coverage(phase),
            ShaderVariant::Simple,
        )?);
    }
    Ok(cell.as_ref().expect("coverage pipeline ready"))
}

/// One instanced-quad pipeline from `external.wgsl` for `format`:
/// source-over only — an external frame composites like any image.
fn create_external_pipeline(
    device: &wgpu::Device,
    layout0: &wgpu::BindGroupLayout,
    layout1: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cherenkov external"),
        bind_group_layouts: &[Some(layout0), Some(layout1)],
        immediate_size: 0,
    });
    let component = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cherenkov external"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_external"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState {
                    color: component,
                    alpha: component,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A `w` × `h` texture in `format`.
fn create_target(
    device: &wgpu::Device,
    label: &'static str,
    size: (u32, u32),
    usages: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: usages,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Byte size of a `w` × `h` uncompressed texture in `format` — what the
/// diagnostic tracks for target allocations.
pub fn texel_bytes(format: wgpu::TextureFormat) -> u64 {
    u64::from(format.block_copy_size(None).unwrap_or(0))
}

/// `c`'s bytes at its format's texel size, including the `deep` deeper
/// pyramid levels of `⌈d / 2^k⌉` texels each.
fn target_bytes(c: &ScratchTarget, deep: u32) -> u64 {
    let texels: u64 = (0..=deep)
        .map(|k| u64::from(c.width).div_ceil(1 << k) * u64::from(c.height).div_ceil(1 << k))
        .sum();
    texels * texel_bytes(c.texture.format())
}

/// `{0, 1, 2, 2, 1, 3}`: the coverage partial pass's indexed draw replays
/// the painter path's ordered triangles while the vertex shader runs once
/// per corner.
fn quad_index_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad indices"),
        contents: bytemuck::cast_slice(&[0u16, 1, 2, 2, 1, 3]),
        usage: wgpu::BufferUsages::INDEX,
    })
}

/// Creates GPU state on the shared engine render thread.
///
/// # Errors
/// Returns initialization and pipeline validation errors from the backend.
#[expect(clippy::too_many_lines, reason = "moved into the render thread")]
#[cfg(not(target_arch = "wasm32"))]
pub fn init(config: GpuConfig) -> Result<(GpuRenderer, GpuInfo), EngineError> {
    let _diag_guard = diag::Guard::scope(config.alloc_diag.as_ref());
    create_device(&config).and_then(|(instance, adapter, device, queue)| {
        let info = adapter.get_info();
        let supported = adapter.features();
        let timestamp_support = if supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
            TimestampSupport::PassBoundaries
        } else {
            TimestampSupport::Unsupported
        };
        let (layout0, layout1) = create_layouts(&device);
        fire_probe(
            &config,
            CreationPhase::Layouts,
            Some(&adapter),
            Some(&device),
        )?;
        let scratch_format = scratch_wgpu(config.scratch_format);
        // Three specialised fragment shaders from one source file, compiled
        // at build time: the prepended `VARIANT` constant makes fs_main a
        // constant-folded dispatch to fs_simple/fs_shadow/fs_full. On Vulkan
        // and Metal these are the embedded passthrough binaries (#57).
        let shader_delivery = shaders::delivery(info.backend, &device)?;
        let modules = core_pipelines_eager()
            .then(|| [0usize, 1, 2].map(|v| shader_delivery.engine_module(&device, v)));
        fire_probe(
            &config,
            CreationPhase::ShaderModules,
            Some(&adapter),
            Some(&device),
        )?;
        let pipeline_layout = create_pipeline_layout(&device, &layout0, &layout1);
        let pipeline_cache = open_pipeline_cache(&device, &config);
        let mut pipelines = std::array::from_fn(|_| {
            std::array::from_fn(|_| std::array::from_fn(|_| None::<wgpu::RenderPipeline>))
        });
        let mut coverage_pipelines = [None, None];
        // The closed core set, deduplicated by format slot: when surface
        // and scratch share `Rgba16Float` every cell lands in slot 0 and
        // the later iteration's `is_none` skips its duplicates (#170).
        // CoreDemand leaves the cells empty — `core_pipeline` fills each
        // at the first draw that needs it (#170 B3). The coverage pair
        // follows the same policy via `coverage_pipeline`.
        if let Some(modules) = &modules {
            for format in [TARGET_FORMAT, scratch_format] {
                let slot = usize::from(format != TARGET_FORMAT);
                for replace in [false, true] {
                    for variant in 0usize..3 {
                        let cell = &mut pipelines[slot][usize::from(replace)][variant];
                        if cell.is_none() {
                            *cell = Some(create_pipeline(
                                &device,
                                &config,
                                &pipeline_layout,
                                pipeline_cache.as_ref(),
                                &modules[variant],
                                format,
                                CoveragePass::Painter(replace),
                            )?);
                        }
                    }
                }
            }
            for (cell, phase) in coverage_pipelines
                .iter_mut()
                .zip([CoveragePhase::Opaque, CoveragePhase::Partial])
            {
                *cell = Some(create_pipeline(
                    &device,
                    &config,
                    &pipeline_layout,
                    pipeline_cache.as_ref(),
                    &modules[0],
                    TARGET_FORMAT,
                    CoveragePass::Coverage(phase),
                )?);
            }
        }
        fire_probe(
            &config,
            CreationPhase::CorePipelines,
            Some(&adapter),
            Some(&device),
        )?;
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            // One 256-byte stride slot: a single pass's Globals entry.
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: 272 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let stops = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stops"),
            size: 32 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_indices = quad_index_buffer(&device);
        diag::create(&device, "globals", globals.size());
        diag::create(&device, "instances", instances.size());
        diag::create(&device, "stops", stops.size());
        fire_probe(
            &config,
            CreationPhase::Buffers,
            Some(&adapter),
            Some(&device),
        )?;
        let atlas = Atlas::new(&device, config.budget.gpu.0);
        fire_probe(&config, CreationPhase::Atlas, Some(&adapter), Some(&device))?;
        let bind0 = make_bind0(&device, &layout0, &globals, &instances, &stops, &atlas);
        let (_, dummy_view) = create_target(
            &device,
            "dummy source",
            (1, 1),
            wgpu::TextureUsages::TEXTURE_BINDING,
            TARGET_FORMAT,
        );
        diag::create(&device, "dummy source", 8);
        let (_, dummy_uint_view) = create_target(
            &device,
            "dummy uint source",
            (1, 1),
            wgpu::TextureUsages::TEXTURE_BINDING,
            wgpu::TextureFormat::R8Uint,
        );
        diag::create(&device, "dummy uint source", 1);
        fire_probe(
            &config,
            CreationPhase::BindGroups,
            Some(&adapter),
            Some(&device),
        )?;
        let timestamps =
            config.timestamps && device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (query_set, query_buffer) = if timestamps {
            (
                Some(device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("frame timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2 * TIMESTAMP_FRAMES_PER_SET,
                })),
                Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamp resolve"),
                    size: 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })),
            )
        } else {
            (None, None)
        };
        if let Some(buffer) = &query_buffer {
            diag::create(&device, "frame timestamps", 0);
            diag::create(&device, "timestamp resolve", buffer.size());
        }
        fire_probe(
            &config,
            CreationPhase::Timestamps,
            Some(&adapter),
            Some(&device),
        )?;
        // Two queries per frame, reserving 64 independent frame ranges.
        let query_capacity = if query_set.is_some() { 2 } else { 0 };
        let query_pool = query_set.as_ref().map_or_else(Vec::new, |set| {
            (1..TIMESTAMP_FRAMES_PER_SET)
                .map(|slot| (set.clone(), slot * 2, 2))
                .collect()
        });
        // The silhouette-blur pipeline is built at the first frame with a
        // shadow, not here (#170 — shadow-free engines pay for none).
        fire_probe(
            &config,
            CreationPhase::ShadowBlur,
            Some(&adapter),
            Some(&device),
        )?;
        // The Vulkan external-frame context is built at the first native
        // frame's registration, not here (#170 — idle engines pay for no
        // native descriptors or pipelines).
        fire_probe(
            &config,
            CreationPhase::ExternalNative,
            Some(&adapter),
            Some(&device),
        )?;
        fire_probe(
            &config,
            CreationPhase::Complete,
            Some(&adapter),
            Some(&device),
        )?;
        let renderer = GpuRenderer {
            instance,
            adapter,
            presenter: None,
            #[cfg_attr(
                not(any(target_vendor = "apple", target_os = "android")),
                expect(
                    clippy::zero_sized_map_values,
                    reason = "no plane realization exists on this platform, so the map stays empty"
                )
            )]
            planes: FxHashMap::default(),
            plane_only: Vec::new(),
            plan_scratch: planes::PlanScratch::default(),
            next_plan: planes::Plan::default(),
            candidates: FxHashMap::default(),
            candidate_frames: FxHashMap::default(),
            ready_sets: Vec::new(),
            ready: FxHashSet::default(),
            shader_delivery,
            shaders: paint::Registry::default(),
            backdrop_shaders: FxHashMap::default(),
            filters: filter::Registry::default(),
            shadow_blur: None,
            resolve: None,
            reduce: None,
            last_frame: None,
            origin: None,
            max_texture: device.limits().max_texture_dimension_2d,
            device,
            queue,
            pipelines,
            coverage_pipelines,
            pipeline_layout,
            #[cfg(target_vendor = "apple")]
            tile_executor: composite::metal::Executor::default(),
            pipeline_cache,
            scratch_format,
            layout0,
            layout1,
            globals,
            instances,
            stops,
            quad_indices,
            bind0,
            dummy_view,
            dummy_uint_view,
            ext_layout: None,
            external_pipes: [None, None],
            #[cfg(all(unix, not(target_vendor = "apple")))]
            native: None,
            #[cfg(all(unix, not(target_vendor = "apple")))]
            native_error: None,
            #[cfg(all(unix, not(target_vendor = "apple")))]
            submit_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            #[cfg_attr(
                not(target_os = "linux"),
                expect(
                    clippy::zero_sized_map_values,
                    reason = "no export pool exists on this platform, so the map stays empty"
                )
            )]
            exports: FxHashMap::default(),
            bound_atlas: 0,
            bound_instance_size: 272 * 16,
            bound_stop_size: 32 * 16,
            bound_globals_size: 256,
            atlas,
            commit_writes: Vec::new(),
            pending_origins: Vec::new(),
            commit_touches: Vec::new(),
            surfaces: FxHashMap::default(),
            producers: FxHashMap::default(),
            pending_retire: FxHashSet::default(),
            fonts: FxHashMap::default(),
            images: FxHashMap::default(),
            bitmaps: FxHashMap::default(),
            images_gen: 0,
            image_replacements: 0,
            timestamps,
            query_set,
            query_base: 0,
            query_pool,
            pending_queries: VecDeque::new(),
            frame_submission: None,
            uploads: upload::Uploads::default(),
            query_buffer,
            query_staging: Vec::new(),
            query_capacity,
            pending_timestamps: VecDeque::new(),
            timings: Vec::new(),
            frame_pass_count: 0,
            pass_meta: Vec::new(),
            wait_timeout: config.wait_timeout,
            tracker: Arc::new(SubmissionTracker::default()),
            diag: config.alloc_diag.clone(),
            config,
            projective: None,
            frame_count: 0,
        };
        Ok((
            renderer,
            GpuInfo {
                name: info.name,
                backend: format!("{:?}", info.backend),
                vendor: info.vendor,
                device: info.device,
                device_type: format!("{:?}", info.device_type),
                driver: info.driver,
                driver_info: info.driver_info,
                timestamps: timestamp_support,
            },
        ))
    })
}

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
#[expect(
    clippy::too_many_lines,
    reason = "adapter and device requests plus pipeline setup form one linear sequence"
)]
pub async fn init(config: GpuConfig) -> Result<(GpuRenderer, GpuInfo), EngineError> {
    let _diag_guard = diag::Guard::scope(config.alloc_diag.as_ref());
    let (instance, adapter, device, queue) = create_device(&config).await?;
    let info = adapter.get_info();
    let supported = adapter.features();
    let timestamp_support = if supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
        TimestampSupport::PassBoundaries
    } else {
        TimestampSupport::Unsupported
    };
    let (layout0, layout1) = create_layouts(&device);
    fire_probe(
        &config,
        CreationPhase::Layouts,
        Some(&adapter),
        Some(&device),
    )?;
    let scratch_format = scratch_wgpu(config.scratch_format);
    // Three specialised fragment shaders from one source file: the
    // prepended `VARIANT` constant makes fs_main a constant-folded
    // dispatch to fs_simple/fs_shadow/fs_full. WebGPU keeps WGSL (#57).
    let shader_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader_delivery = shaders::delivery(info.backend, &device)?;
    let modules = [0usize, 1, 2].map(|v| shader_delivery.engine_module(&device, v));
    if let Some(error) = shader_scope.pop().await {
        return Err(EngineError::Backend(error.to_string()));
    }
    fire_probe(
        &config,
        CreationPhase::ShaderModules,
        Some(&adapter),
        Some(&device),
    )?;
    let pipeline_layout = create_pipeline_layout(&device, &layout0, &layout1);
    let pipeline_cache = open_pipeline_cache(&device, &config);
    let mut pipelines = std::array::from_fn(|_| {
        std::array::from_fn(|_| std::array::from_fn(|_| None::<wgpu::RenderPipeline>))
    });
    // The closed core set, deduplicated by format slot: when surface and
    // scratch share `Rgba16Float` every cell lands in slot 0 and the
    // later iteration's `is_none` skips its duplicates (#170). wasm
    // always prepares eagerly — async creation cannot run inside the
    // synchronous encode loop, so the cells are never empty.
    for format in [TARGET_FORMAT, scratch_format] {
        let slot = usize::from(format != TARGET_FORMAT);
        for replace in [false, true] {
            for variant in 0usize..3 {
                let cell = &mut pipelines[slot][usize::from(replace)][variant];
                if cell.is_none() {
                    *cell = Some(
                        create_pipeline(
                            &device,
                            &config,
                            &pipeline_layout,
                            pipeline_cache.as_ref(),
                            &modules[variant],
                            format,
                            CoveragePass::Painter(replace),
                        )
                        .await?,
                    );
                }
            }
        }
    }
    let coverage_pipelines = <[_; 2]>::from(
        futures_util::future::try_join(
            create_pipeline(
                &device,
                &config,
                &pipeline_layout,
                pipeline_cache.as_ref(),
                &modules[0],
                TARGET_FORMAT,
                CoveragePass::Coverage(CoveragePhase::Opaque),
            ),
            create_pipeline(
                &device,
                &config,
                &pipeline_layout,
                pipeline_cache.as_ref(),
                &modules[0],
                TARGET_FORMAT,
                CoveragePass::Coverage(CoveragePhase::Partial),
            ),
        )
        .await?,
    )
    .map(Some);
    fire_probe(
        &config,
        CreationPhase::CorePipelines,
        Some(&adapter),
        Some(&device),
    )?;
    let globals = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("globals"),
        // One 256-byte stride slot: a single pass's Globals entry.
        size: 256,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let instances = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: 272 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let stops = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("stops"),
        size: 32 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let quad_indices = quad_index_buffer(&device);
    diag::create(&device, "globals", globals.size());
    diag::create(&device, "instances", instances.size());
    diag::create(&device, "stops", stops.size());
    fire_probe(
        &config,
        CreationPhase::Buffers,
        Some(&adapter),
        Some(&device),
    )?;
    let atlas = Atlas::new(&device, config.budget.gpu.0);
    fire_probe(&config, CreationPhase::Atlas, Some(&adapter), Some(&device))?;
    let bind0 = make_bind0(&device, &layout0, &globals, &instances, &stops, &atlas);
    let (_, dummy_view) = create_target(
        &device,
        "dummy source",
        (1, 1),
        wgpu::TextureUsages::TEXTURE_BINDING,
        TARGET_FORMAT,
    );
    diag::create(&device, "dummy source", 8);
    let (_, dummy_uint_view) = create_target(
        &device,
        "dummy uint source",
        (1, 1),
        wgpu::TextureUsages::TEXTURE_BINDING,
        wgpu::TextureFormat::R8Uint,
    );
    diag::create(&device, "dummy uint source", 1);
    fire_probe(
        &config,
        CreationPhase::BindGroups,
        Some(&adapter),
        Some(&device),
    )?;
    let timestamps =
        config.timestamps && device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let (query_set, query_buffer) = if timestamps {
        (
            Some(device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2 * TIMESTAMP_FRAMES_PER_SET,
            })),
            Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })),
        )
    } else {
        (None, None)
    };
    if let Some(buffer) = &query_buffer {
        diag::create(&device, "frame timestamps", 0);
        diag::create(&device, "timestamp resolve", buffer.size());
    }
    fire_probe(
        &config,
        CreationPhase::Timestamps,
        Some(&adapter),
        Some(&device),
    )?;
    // Two queries per frame, reserving 64 independent frame ranges.
    let query_capacity = if query_set.is_some() { 2 } else { 0 };
    let query_pool = query_set.as_ref().map_or_else(Vec::new, |set| {
        (1..TIMESTAMP_FRAMES_PER_SET)
            .map(|slot| (set.clone(), slot * 2, 2))
            .collect()
    });
    // The silhouette-blur pipeline is built at the first frame with a
    // shadow, not here (#170 — shadow-free engines pay for none).
    fire_probe(
        &config,
        CreationPhase::ShadowBlur,
        Some(&adapter),
        Some(&device),
    )?;
    fire_probe(
        &config,
        CreationPhase::Complete,
        Some(&adapter),
        Some(&device),
    )?;
    let renderer = GpuRenderer {
        instance,
        adapter,
        presenter: None,
        #[cfg_attr(
            not(any(target_vendor = "apple", target_os = "android")),
            expect(
                clippy::zero_sized_map_values,
                reason = "no plane realization exists on this platform, so the map stays empty"
            )
        )]
        planes: FxHashMap::default(),
        plane_only: Vec::new(),
        plan_scratch: planes::PlanScratch::default(),
        next_plan: planes::Plan::default(),
        candidates: FxHashMap::default(),
        candidate_frames: FxHashMap::default(),
        ready_sets: Vec::new(),
        ready: FxHashSet::default(),
        shader_delivery,
        shaders: paint::Registry::default(),
        backdrop_shaders: FxHashMap::default(),
        union_module: None,
        filters: filter::Registry::default(),
        shadow_blur: None,
        resolve: None,
        reduce: None,
        last_frame: None,
        origin: None,
        max_texture: device.limits().max_texture_dimension_2d,
        device,
        queue,
        pipelines,
        coverage_pipelines,
        pipeline_layout,
        pipeline_cache,
        scratch_format,
        layout0,
        layout1,
        globals,
        instances,
        stops,
        quad_indices,
        bind0,
        dummy_view,
        dummy_uint_view,
        ext_layout: None,
        external_pipes: [None, None],
        #[cfg_attr(
            not(target_os = "linux"),
            expect(
                clippy::zero_sized_map_values,
                reason = "no export pool exists on this platform, so the map stays empty"
            )
        )]
        exports: FxHashMap::default(),
        bound_atlas: 0,
        bound_instance_size: 272 * 16,
        bound_stop_size: 32 * 16,
        bound_globals_size: 256,
        atlas,
        commit_writes: Vec::new(),
        pending_origins: Vec::new(),
        commit_touches: Vec::new(),
        surfaces: FxHashMap::default(),
        producers: FxHashMap::default(),
        pending_retire: FxHashSet::default(),
        fonts: FxHashMap::default(),
        images: FxHashMap::default(),
        bitmaps: FxHashMap::default(),
        images_gen: 0,
        image_replacements: 0,
        timestamps,
        query_set,
        query_base: 0,
        query_pool,
        pending_queries: VecDeque::new(),
        frame_submission: None,
        uploads: upload::Uploads::default(),
        query_buffer,
        query_staging: Vec::new(),
        query_capacity,
        pending_timestamps: VecDeque::new(),
        timings: Vec::new(),
        frame_pass_count: 0,
        pass_meta: Vec::new(),
        wait_timeout: config.wait_timeout,
        tracker: Arc::new(SubmissionTracker::default()),
        diag: config.alloc_diag.clone(),
        config,
        projective: None,
        frame_count: 0,
    };
    Ok((
        renderer,
        GpuInfo {
            name: info.name,
            backend: format!("{:?}", info.backend),
            vendor: info.vendor,
            device: info.device,
            device_type: format!("{:?}", info.device_type),
            driver: info.driver,
            driver_info: info.driver_info,
            timestamps: timestamp_support,
        },
    ))
}

/// The stack entries the surface's committed plan promotes, each with its
/// installed frame and install generation — optionally only `updates`'s
/// layers.
fn plane_stack<'a, 'p: 'a>(
    surface: &'a SurfaceState,
    producers: &'a FxHashMap<ProducerId, &'p external::Slot>,
    updates: Option<&'a FxHashSet<LayerId>>,
) -> impl Iterator<Item = planes::Plane<'a>> + 'a {
    surface
        .plan
        .planes
        .iter()
        .filter(move |placement| updates.is_none_or(|updates| updates.contains(&placement.layer)))
        .map(|placement| {
            if let Some(hosted) = surface.hosted.get(&placement.layer) {
                return planes::Plane {
                    placement,
                    content: planes::PlaneContent::Hosted {
                        object: &hosted.object,
                        extent: hosted.extent,
                    },
                };
            }
            let content = surface
                .bindings
                .get(&placement.layer)
                .and_then(|binding| producers.get(&binding.producer()).copied())
                .map_or_else(
                    || {
                        let capture = surface.static_layers[&placement.layer]
                            .capture
                            .as_ref()
                            .expect("promoted pixels are captured");
                        planes::PlaneContent::Raster {
                            view: capture.source.as_ref().map(|(_, view)| view),
                            generation: capture.generation,
                        }
                    },
                    |slot| planes::PlaneContent::Frame {
                        frame: &slot.frame,
                        generation: slot.generation,
                    },
                );
            planes::Plane { placement, content }
        })
}

/// `surf`'s promotion candidates — its hosted layers, the producer
/// bindings whose current frame a plane can show, each at its binding's
/// output size, and its quiet recorded layers — filled into `candidates`,
/// which keeps its allocation between fills (#90).
fn plane_candidates<'a>(
    surf: &SurfaceState,
    producers: &FxHashMap<ProducerId, &external::Slot>,
    candidates: &'a mut FxHashMap<LayerId, planes::Candidate>,
) -> &'a FxHashMap<LayerId, planes::Candidate> {
    candidates.clear();
    candidates.extend(
        surf.hosted
            .iter()
            .map(|(&layer, hosted)| (layer, hosted.candidate())),
    );
    candidates.extend(surf.bindings.iter().filter_map(|(layer, binding)| {
        let slot = producers.get(&binding.producer()).copied()?;
        slot.on_plane.then_some((
            *layer,
            planes::Candidate {
                size: slot.size,
                raster: kurbo::Affine::scale_non_uniform(
                    f64::from(binding.size.0) / f64::from(slot.size.0),
                    f64::from(binding.size.1) / f64::from(slot.size.1),
                ),
                source: planes::Source::Frame,
            },
        ))
    }));
    candidates.extend(surf.static_layers.iter().filter_map(|(&layer, entry)| {
        let domain = entry.domain?;
        (entry.quiet_frames >= entry.quiet_required).then_some((
            layer,
            planes::Candidate {
                size: domain.size,
                raster: domain.raster(),
                source: planes::Source::Recorded,
            },
        ))
    }));
    candidates
}

fn plane_frames<'a>(
    surf: &SurfaceState,
    producers: &FxHashMap<ProducerId, &external::Slot>,
    frames: &'a mut FxHashMap<LayerId, (ExternalFrame, u64)>,
) -> &'a FxHashMap<LayerId, (ExternalFrame, u64)> {
    frames.clear();
    frames.extend(surf.bindings.iter().filter_map(|(layer, binding)| {
        let slot = producers.get(&binding.producer()).copied()?;
        slot.on_plane
            .then_some((*layer, (slot.frame.clone(), slot.generation)))
    }));
    frames
}

/// The frame's per-surface platform commits, in frame order: `()`s
/// where every present happens on the render thread, the window
/// surfaces' `CATransaction` payloads on Apple — opaque boxes, so the
/// public `Renderer::FrameCommit` names no platform type.
#[cfg(target_vendor = "apple")]
pub(in crate::render) type FrameCommits = Vec<Option<Box<dyn planes::apple::CommitApply>>>;
/// See the Apple variant above; no other platform commits off-thread.
#[cfg(not(target_vendor = "apple"))]
pub(in crate::render) type FrameCommits = Vec<()>;

/// One surface's platform commit; its `Default` is the empty commit a
/// surface that commits nothing off-thread produces (`None` on Apple).
type PlatformCommit = <planes::Platform as planes::SystemPlanes>::Commit;

impl Renderer for GpuRenderer {
    type Target = GpuTarget;
    type Font = PreparedFont;
    /// The frame's per-surface platform commits, in frame order.
    type FrameCommit = FrameCommits;

    /// Applies each surface's commit on the awaiting caller. Apple
    /// window surfaces land their whole `CATransaction` — layer geometry
    /// and every part's drawable present — inside this call on main, so
    /// the reply hands it back before the next frame's acquire (#2261).
    /// The marker is demanded only when the frame carried a commit: a
    /// caller off-main that awaits a frame covering an Apple window
    /// surface fails here, while frames with nothing to commit apply to
    /// nothing.
    #[cfg(all(not(target_arch = "wasm32"), target_vendor = "apple"))]
    fn apply_frame_commit(commit: Self::FrameCommit) {
        for commit in commit.into_iter().flatten() {
            let mtm = objc2::MainThreadMarker::new()
                .expect("an Apple window surface's frame must be awaited on the main thread");
            commit.apply(mtm);
        }
    }

    fn create_surface(
        &mut self,
        id: SurfaceId,
        target: GpuTarget,
        waker: cherenkov::CompletionWaker,
    ) -> Result<SurfaceInfo, SurfaceError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        diag::set_surface(Some(id.raw()));
        let size = self.target_size(&target)?;
        let (window, textures, refresh, presents) = match target {
            GpuTarget::Offscreen(offscreen) => (None, None, offscreen.refresh, false),
            GpuTarget::Texture(texture) => (None, Some(texture.textures), texture.refresh, false),
            #[cfg(target_os = "android")]
            GpuTarget::SurfaceControl(target) => {
                (None, None, self.surface_control(id, target)?, true)
            }
            #[cfg(target_os = "linux")]
            GpuTarget::Dmabuf(target) => (None, None, self.dmabuf_export(id, target)?, true),
            GpuTarget::Window(window) => {
                let refresh = window.refresh.clone();
                // Only Apple windows complete work after a render: a
                // promoted plane's attach on the main queue.
                #[cfg(target_vendor = "apple")]
                let surface = self.open_window(id, window, size, waker.clone());
                #[cfg(not(target_vendor = "apple"))]
                let surface = self.open_window(id, window, size)?;
                (surface, None, refresh, true)
            }
        };
        let promotes = self.planes.contains_key(&id);
        let (target, view) = create_target(
            &self.device,
            "surface target",
            size,
            TARGET_USAGES,
            TARGET_FORMAT,
        );
        if let Some(sender) = &textures {
            let _ = sender.send(target.clone());
        }
        diag::create(
            &self.device,
            "surface target",
            u64::from(size.0) * u64::from(size.1) * texel_bytes(TARGET_FORMAT),
        );
        self.surfaces.insert(
            id,
            SurfaceState {
                window,
                promotes,
                plan: planes::Plan::default(),
                plane_stamp: 0,
                plane_resources: (0, 0, 0),
                plane_clear: cherenkov::WorkingColor::TRANSPARENT,
                plane_size: (0, 0),
                static_layers: FxHashMap::default(),
                static_walk: Vec::new(),
                plan_dirty: false,
                parts: Vec::new(),
                textures,
                refresh,
                present_pending: false,
                display: cherenkov::Display::default(),
                size,
                target,
                view,
                scratch: FxHashMap::default(),
                coverage_depth: None,
                backdrop: [None, None],
                staging: [None, None],
                staging_bind: [None, None],
                backdrop_groups: FxHashMap::default(),
                layers: FxHashMap::default(),
                bindings: FxHashMap::default(),
                hosted: FxHashMap::default(),
                shader_textures: FxHashMap::default(),
                frame: LoweredFrame::default(),
                #[cfg(target_vendor = "apple")]
                composition: composite::plan::ExecutionPlan::default(),
                #[cfg(target_vendor = "apple")]
                composition_cache: composite::cache::Cache::default(),
                #[cfg(target_vendor = "apple")]
                tile_targets: composite::metal::Attachments::default(),
                inst_base: 0,
                globals_base: 0,
                bind_gen: 0,
                binds1: FxHashMap::default(),
                painter_replays: FxHashMap::default(),
                binds1_stamp: (u64::MAX, u64::MAX, u64::MAX),
                projective: FxHashMap::default(),
                realized: Vec::new(),
                composed: Vec::new(),
                interop: 0,
                visibility: Visibility::Visible,
                waker,
            },
        );
        diag::set_surface(None);
        // The system composites a surface with planes: the engine never
        // holds its whole image.
        Ok(SurfaceInfo {
            max_dimension: self.max_texture,
            size,
            readable: !promotes,
            presents,
        })
    }

    fn set_visibility(&mut self, id: SurfaceId, visibility: Visibility) {
        let surface = self
            .surfaces
            .get_mut(&id)
            .expect("visibility of a created surface");
        // The producers' and filters' wakes already follow the announced
        // visibility through the surface's waker; this decides what a frame
        // lists and what counts in `FrameRedraw`.
        surface.visibility = visibility;
    }

    fn resize_surface(&mut self, id: SurfaceId, size: (u32, u32)) {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        diag::set_surface(Some(id.raw()));
        let Some(state) = self.surfaces.get_mut(&id) else {
            diag::set_surface(None);
            return;
        };
        let (target, view) = create_target(
            &self.device,
            "surface target",
            size,
            TARGET_USAGES,
            TARGET_FORMAT,
        );
        if let Some(window) = &mut state.window {
            window.resize(&self.device, size);
        }
        if let Some(sender) = &state.textures {
            let _ = sender.send(target.clone());
        }
        let dropped = state.binds1.len() as u64;
        if dropped > 0 {
            diag::bind_groups_dropped(&self.device, dropped, "resize");
        }
        let scratch_bytes: u64 = state
            .scratch
            .values()
            .map(|s| u64::from(s.width) * u64::from(s.height) * texel_bytes(s.texture.format()))
            .sum();
        let backdrop_bytes: u64 = state
            .backdrop
            .iter()
            .flatten()
            .map(|s| u64::from(s.width) * u64::from(s.height) * texel_bytes(s.texture.format()))
            .sum();
        let old_target =
            u64::from(state.size.0) * u64::from(state.size.1) * texel_bytes(state.target.format());
        for (label, bytes) in [
            ("isolation scratch", scratch_bytes),
            ("blend backdrop", backdrop_bytes),
            ("backdrop staging", state.staging_bytes()),
            (
                "coverage depth",
                state
                    .coverage_depth
                    .as_ref()
                    .map_or(0, |depth| coverage_depth_bytes(&depth.texture)),
            ),
        ] {
            if bytes > 0 {
                diag::retire(
                    &self.device,
                    diag::RetireArgs {
                        label,
                        class: diag::Class::Target,
                        bytes,
                        used_in_latest_submit: true,
                        reason: "resize",
                    },
                );
            }
        }
        state.size = size;
        state.target = target;
        state.view = view;
        for part in &mut state.parts {
            *part = create_target(
                &self.device,
                "engine part",
                size,
                TARGET_USAGES,
                TARGET_FORMAT,
            );
        }
        if let Some(system) = self.planes.get_mut(&id) {
            planes::SystemPlanes::resize(system, size);
        }
        #[cfg(target_os = "linux")]
        if let Some(export) = self.exports.get_mut(&id) {
            export.resize(size);
        }
        state.coverage_depth = None;
        state.scratch.clear();
        #[cfg(target_vendor = "apple")]
        state.tile_targets.clear();
        state.backdrop = [None, None];
        state.staging = [None, None];
        state.staging_bind = [None, None];
        state.retire_binds(&self.device);
        state.painter_replays.clear();
        state.bind_gen += 1;
        diag::grow(
            &self.device,
            "surface target",
            diag::Class::Target,
            old_target,
            u64::from(size.0) * u64::from(size.1) * texel_bytes(TARGET_FORMAT),
            0,
            true,
        );
        diag::set_surface(None);
    }

    fn destroy_surface(&mut self, id: SurfaceId) {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        if let Some(state) = self.surfaces.get(&id) {
            diag::set_surface(Some(id.raw()));
            let target_bytes = u64::from(state.size.0)
                * u64::from(state.size.1)
                * texel_bytes(state.target.format())
                * (1 + state.parts.len() as u64);
            let scratch_bytes: u64 = state.scratch.values().map(ScratchTarget::bytes).sum();
            let backdrop_bytes: u64 = state
                .backdrop
                .iter()
                .flatten()
                .map(ScratchTarget::bytes)
                .sum();
            let capture_bytes = state.backdrop_bytes();
            let staging_bytes = state.staging_bytes();
            let dropped = state.binds1.len() as u64;
            if dropped > 0 {
                diag::bind_groups_dropped(&self.device, dropped, "destroy");
            }
            for (label, bytes) in [
                ("surface target", target_bytes),
                ("isolation scratch", scratch_bytes),
                ("blend backdrop", backdrop_bytes),
                ("backdrop staging", staging_bytes),
                ("backdrop capture", capture_bytes),
                ("projective image", state.projective_bytes()),
            ] {
                if bytes > 0 {
                    diag::retire(
                        &self.device,
                        diag::RetireArgs {
                            label,
                            class: diag::Class::Target,
                            bytes,
                            used_in_latest_submit: true,
                            reason: "destroy",
                        },
                    );
                }
            }
        }
        diag::set_surface(None);
        // The surface's bindings die on the render thread: a drop that
        // held its producer's last reference posts the retirement onto
        // the producer's own queue, never the channel this thread
        // consumes.
        if let Some(state) = self.surfaces.remove(&id) {
            // A backdrop group's chain is registered beside the surface,
            // keyed by it: a group handle that outlives its surface finds
            // no surface to remove it from, so the chains go here.
            for key in state
                .backdrop_groups
                .values()
                .filter_map(|group| group.filter)
            {
                self.release_backdrop_chain(key, "destroy");
            }
        }
        self.planes.remove(&id);
        self.exports.remove(&id);
        self.update_filter_activity();
        self.update_producer_wakes();
    }

    /// Validates font data and detects native colour-glyph formats.
    ///
    /// `COLR` fonts render through the colour-glyph lowering; supported
    /// sbix and CBDT/CBLC glyphs are decoded as images at realization.
    fn prepare_font(font: EngineFontData) -> Result<PreparedFont, ResourceError> {
        use skrifa::raw::TableProvider as _;
        let parsed = skrifa::FontRef::from_index(&font.data, font.index)
            .map_err(|e| ResourceError::Font(format!("{e}")))?;
        if parsed.data_for_tag(skrifa::Tag::new(b"SVG ")).is_some() {
            return Err(ResourceError::Unsupported(names::COLOR_FONT));
        }
        let has_colr = parsed.colr().is_ok();
        let bitmap = bitmap::BitmapFont::detect(&font.data, font.index)?.map(Arc::new);
        Ok(PreparedFont {
            data: font.data,
            index: font.index,
            has_colr,
            bitmap,
        })
    }

    fn add_font(&mut self, id: FontId, font: PreparedFont) {
        self.fonts.insert(id.raw(), font.into());
    }

    fn remove_font(&mut self, id: FontId) {
        self.fonts.remove(&id.raw());
        self.bitmaps.retain(|key, _| key.font != id.raw());
        self.images_gen += 1;
        for surface in self.surfaces.values_mut() {
            for content in surface.layers.values_mut() {
                content.invalidate();
            }
        }
        self.atlas.remove_font(id.raw());
    }

    fn set_content(
        &mut self,
        surface: SurfaceId,
        layer: LayerId,
        content: Option<ContentOp>,
    ) -> Option<cherenkov::Picture> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        let state = self.surfaces.get_mut(&surface)?;
        Self::unbind_producer(
            state,
            &self.device,
            self.images_gen,
            self.atlas.mask_texture_generation(),
            layer,
        );
        state.hosted.remove(&layer);
        match content {
            Some(ContentOp::Replace(list)) => {
                if let Some(content) = state.layers.get_mut(&layer) {
                    Some(content.replace(list))
                } else {
                    state.layers.insert(layer, ContentData::new(list));
                    None
                }
            }
            Some(ContentOp::Update(updates)) => {
                state
                    .layers
                    .get_mut(&layer)
                    .expect("slot update targets a layer without content")
                    .update(updates);
                None
            }
            Some(ContentOp::Picture(picture)) => state
                .layers
                .insert(layer, ContentData::picture(picture))
                .map(ContentData::into_picture),
            None => state.layers.remove(&layer).map(ContentData::into_picture),
        }
    }

    fn remove_layer(&mut self, surface: SurfaceId, layer: LayerId) {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        if let Some(state) = self.surfaces.get_mut(&surface) {
            state.layers.remove(&layer);
            state.static_layers.remove(&layer);
            state.projective.remove(&layer);
            state.composed.retain(|key| key.layer != layer);
            state.hosted.remove(&layer);
            Self::unbind_producer(
                state,
                &self.device,
                self.images_gen,
                self.atlas.mask_texture_generation(),
                layer,
            );
        }
    }

    fn image_limits(&self) -> cherenkov::ImageLimits {
        cherenkov::ImageLimits {
            max_dimension: self.max_texture,
            // The upload lands as f16 RGBA: eight bytes a texel.
            max_texels: self.config.budget.gpu.0 / 8,
        }
    }

    fn add_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        let data = image_texels_f16(&image)?;
        let image = upload_image(
            &self.device,
            &self.queue,
            (image.width, image.height),
            &data,
        );
        self.images.insert(id.raw(), image);
        self.images_gen += 1;
        Ok(())
    }

    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        let data = image_texels_f16(&image)?;
        let size = (image.width, image.height);
        let current = self
            .images
            .get(&id.raw())
            .expect("replace targets a registered image");
        // Local images that sampled the previous pixels are no longer
        // current, whichever path the upload takes.
        self.image_replacements += 1;
        if (current.width, current.height) == size {
            // The copy is queued ahead of the next submission, after every
            // frame already submitted, so no frame samples a partly
            // written texture. The view is unchanged: bind groups and
            // lowered paints stay valid.
            write_image(&self.device, &self.queue, &current.texture, size, &data);
            return Ok(());
        }
        let replacement = upload_image(&self.device, &self.queue, size, &data);
        let old = self
            .images
            .insert(id.raw(), replacement)
            .expect("replace targets a registered image");
        self.images_gen += 1;
        diag::retire(
            &self.device,
            diag::RetireArgs {
                label: "image",
                class: diag::Class::Image,
                bytes: u64::from(old.width) * u64::from(old.height) * 8,
                used_in_latest_submit: true,
                reason: "replace_image",
            },
        );
        // #169 A4: bind groups created against the replaced view are
        // stale — `Registered(id)` now binds the new texture; submitted
        // encoders keep the old one until completion. Lowering resolved
        // the old dimensions into the image's paints, so content sampling
        // it is lowered again.
        let images_gen = self.images_gen;
        let mask_gen = self.atlas.mask_texture_generation();
        for surf in self.surfaces.values_mut() {
            retire_binds1(
                surf,
                &self.device,
                images_gen,
                mask_gen,
                "image replaced",
                |key| key.2 == Some(lower::ImageSource::Registered(id.raw())),
            );
            for content in surf.layers.values_mut() {
                content.invalidate_image(id);
            }
        }
        Ok(())
    }

    fn samples(&self, surface: SurfaceId, resource: ResourceId) -> bool {
        self.surfaces.get(&surface).is_some_and(|state| {
            state
                .layers
                .values()
                .any(|content| content.references(resource))
        })
    }

    fn remove_image(&mut self, id: ImageId) {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        if let Some(image) = self.images.get(&id.raw()) {
            diag::retire(
                &self.device,
                diag::RetireArgs {
                    label: "image",
                    class: diag::Class::Image,
                    bytes: u64::from(image.width) * u64::from(image.height) * 8,
                    used_in_latest_submit: true,
                    reason: "remove_image",
                },
            );
        }
        self.images.remove(&id.raw());
        self.images_gen += 1;
        // #169 A4: drop unsubmitted bind groups holding the removed
        // image's view; submitted encoders keep it until completion.
        let images_gen = self.images_gen;
        let mask_gen = self.atlas.mask_texture_generation();
        for surf in self.surfaces.values_mut() {
            retire_binds1(
                surf,
                &self.device,
                images_gen,
                mask_gen,
                "image removed",
                |key| key.2 == Some(lower::ImageSource::Registered(id.raw())),
            );
            for content in surf.layers.values_mut() {
                content.invalidate();
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "rebuilds every surface's scratch, bindings and buffers in pressure order"
    )]
    fn trim(&mut self, pressure: Pressure) {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        diag::set_phase("trim");
        if let Some(blur) = &mut self.shadow_blur {
            blur.trim();
        }
        for surf in self.surfaces.values_mut() {
            let scratch_bytes: u64 = surf.scratch.values().map(ScratchTarget::bytes).sum();
            let backdrop_bytes: u64 = surf
                .backdrop
                .iter()
                .flatten()
                .map(ScratchTarget::bytes)
                .sum();
            let capture_bytes = surf.backdrop_bytes();
            let staging_bytes = surf.staging_bytes();
            let dropped = surf.binds1.len() as u64;
            if dropped > 0 {
                diag::bind_groups_dropped(&self.device, dropped, "trim");
            }
            for (label, bytes) in [
                ("isolation scratch", scratch_bytes),
                ("blend backdrop", backdrop_bytes),
                ("backdrop staging", staging_bytes),
                ("backdrop capture", capture_bytes),
                (
                    "coverage depth",
                    surf.coverage_depth
                        .as_ref()
                        .map_or(0, |depth| coverage_depth_bytes(&depth.texture)),
                ),
            ] {
                if bytes > 0 {
                    diag::retire(
                        &self.device,
                        diag::RetireArgs {
                            label,
                            class: diag::Class::Target,
                            bytes,
                            used_in_latest_submit: true,
                            reason: "trim",
                        },
                    );
                }
            }
            surf.scratch.clear();
            #[cfg(target_vendor = "apple")]
            {
                surf.composition = composite::plan::ExecutionPlan::default();
                surf.composition_cache = composite::cache::Cache::default();
                surf.tile_targets.clear();
            }
            surf.coverage_depth = None;
            surf.backdrop = [None, None];
            surf.staging = [None, None];
            surf.staging_bind = [None, None];
            for state in surf.backdrop_groups.values_mut() {
                state.truncate(0);
            }
            // The bind groups' views died with the textures.
            surf.binds1.clear();
            surf.painter_replays.clear();
            surf.bind_gen += 1;
            // Local images the latest frame did not compose are optional;
            // under critical pressure every one goes and is realized again
            // when next drawn.
            let critical = pressure == Pressure::Critical;
            if critical {
                surf.composed.clear();
            }
            let composed = &surf.composed;
            let mut bytes = 0;
            for (layer, entries) in &mut surf.projective {
                entries.retain(|e| {
                    let keep = composed.contains(&projective::LocalKey {
                        layer: *layer,
                        bucket: e.bucket,
                    });
                    if !keep {
                        bytes += e.bytes();
                    }
                    keep
                });
            }
            surf.projective.retain(|_, entries| !entries.is_empty());
            if bytes > 0 {
                diag::retire(
                    &self.device,
                    diag::RetireArgs {
                        label: "projective image",
                        class: diag::Class::Target,
                        bytes,
                        used_in_latest_submit: true,
                        reason: "trim",
                    },
                );
            }
        }
        let filter_bytes = self.filters.trim();
        if filter_bytes > 0 {
            diag::retire(
                &self.device,
                diag::RetireArgs {
                    label: "filter targets",
                    class: diag::Class::Target,
                    bytes: filter_bytes,
                    used_in_latest_submit: true,
                    reason: "trim",
                },
            );
        }
        if pressure != Pressure::Critical {
            return;
        }
        self.atlas.clear();
        self.bitmaps.clear();
        self.images_gen += 1;
        for font in self.fonts.values() {
            font.colr.borrow_mut().clear();
        }
        for surf in self.surfaces.values_mut() {
            for content in surf.layers.values_mut() {
                content.trim();
            }
            surf.static_layers.clear();
            surf.plan = planes::Plan::default();
            surf.plan_dirty = surf.promotes;
            surf.frame.instances.shrink_to_fit();
            surf.frame.stops.shrink_to_fit();
            surf.frame.passes.shrink_to_fit();
        }
        self.uploads = upload::Uploads::default();
        diag::grow(
            &self.device,
            "instances",
            diag::Class::Buffer,
            self.instances.size(),
            272 * 16,
            0,
            true,
        );
        diag::grow(
            &self.device,
            "stops",
            diag::Class::Buffer,
            self.stops.size(),
            32 * 16,
            0,
            true,
        );
        diag::grow(
            &self.device,
            "globals",
            diag::Class::Buffer,
            self.globals.size(),
            16,
            0,
            true,
        );
        self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: 272 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.stops = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stops"),
            size: 32 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            // One 256-byte stride slot: a single pass's Globals entry.
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bound_instance_size = self.instances.size();
        self.bound_stop_size = self.stops.size();
        self.bound_globals_size = self.globals.size();
        let atlas = &self.atlas;
        diag::bind_groups_dropped(&self.device, 1, "trim");
        self.bind0 = make_bind0(
            &self.device,
            &self.layout0,
            &self.globals,
            &self.instances,
            &self.stops,
            atlas,
        );
        self.bound_atlas = atlas.generation();
        diag::set_phase("render");
    }

    fn owned_animations(&self, surface: SurfaceId) -> &[LayerId] {
        self.planes
            .get(&surface)
            .map_or(&[], |planes| planes::SystemPlanes::owned_animations(planes))
    }

    fn memory(&self) -> MemoryUsage {
        let gpu = self.instances.size()
            + self
                .planes
                .values()
                .map(planes::SystemPlanes::captured_bytes)
                .sum::<u64>()
            + self.uploads.gpu_bytes()
            + self.stops.size()
            + self.globals.size()
            + self.atlas.gpu_bytes()
            + self.atlas.mask_texture_bytes()
            + self
                .surfaces
                .values()
                .map(SurfaceState::gpu_bytes)
                .sum::<u64>()
            + self
                .producers
                .values()
                .map(gpu_content::Producer::gpu_bytes)
                .sum::<u64>()
            + self
                .images
                .values()
                .map(|i| u64::from(i.width) * u64::from(i.height) * 8)
                .sum::<u64>()
            + self
                .bitmaps
                .values()
                .map(|bitmap| u64::from(bitmap.image.width) * u64::from(bitmap.image.height) * 8)
                .sum::<u64>();
        let captures = self
            .surfaces
            .values()
            .map(SurfaceState::backdrop_bytes)
            .sum();
        let capture_format = self
            .surfaces
            .values()
            .flat_map(|surf| surf.backdrop_groups.values())
            .flat_map(|g| &g.captures)
            .map(|c| format_name(c.target.texture.format()))
            .next();
        let cpu = self.atlas.cpu_bytes()
            + self
                .surfaces
                .values()
                .flat_map(|surface| surface.painter_replays.values())
                .map(|replay| {
                    (std::mem::size_of::<PainterReplay>()
                        + replay.ranges.capacity()
                            * std::mem::size_of::<(ShaderVariant, std::ops::Range<u32>)>())
                        as u64
                })
                .sum::<u64>();
        #[cfg(target_vendor = "apple")]
        let cpu = cpu
            + self
                .surfaces
                .values()
                .map(|surface| surface.composition.bytes() + surface.composition_cache.bytes())
                .sum::<u64>();
        MemoryUsage {
            gpu: cherenkov::Bytes(
                gpu + self.filters.gpu_bytes()
                    + self.shadow_blur.as_ref().map_or(0, shadow::Blur::gpu_bytes),
            ),
            cpu: cherenkov::Bytes(cpu),
            backdrop_captures: cherenkov::Bytes(captures),
            backdrop_capture_format: capture_format,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<(FrameRedraw, Self::FrameCommit), RenderError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        diag::set_phase("render");
        diag::frame_boundary(&self.device, frame.id.get(), true, false);
        let outcome = self.render_inner(frame, stats);
        diag::frame_boundary(&self.device, frame.id.get(), false, outcome.is_ok());
        outcome
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<(FrameRedraw, Self::FrameCommit), RenderError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        diag::set_phase("render");
        diag::frame_boundary(&self.device, frame.id.get(), true, false);
        let outcome = self.render_inner(frame, stats).await;
        diag::frame_boundary(&self.device, frame.id.get(), false, outcome.is_ok());
        outcome
    }

    /// Waits for the GPU to finish every pending frame and returns their
    /// timings, oldest first.
    #[cfg(not(target_arch = "wasm32"))]
    fn finish_timings(&mut self) -> Result<Vec<FrameTiming>, RenderError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        // Tooling may wait; the frame path only polls. First complete draws
        // so their resolves can be encoded, then complete the resolve copies.
        if let Some(last) = self.pending_queries.back().map(|p| p.submission.clone()) {
            self.wait(&last, "timestamp draws")?;
        }
        self.drain_timestamps();
        if !self.pending_queries.is_empty() {
            return Err(RenderError::Readback(
                "timestamp draws: the completion callback did not run after the wait".into(),
            ));
        }
        if let Some(last) = self.pending_timestamps.back().map(|p| p.submission.clone()) {
            for pending in &mut self.pending_timestamps {
                pending.request_map();
            }
            self.wait(&last, "timestamp resolve")?;
            self.drain_timestamps();
        }
        if self.pending_timestamps.is_empty() {
            Ok(std::mem::take(&mut self.timings))
        } else {
            Err(RenderError::Readback(
                "timestamp resolve: the map callback did not run after the wait".into(),
            ))
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn finish_timings(&mut self) -> Result<Vec<FrameTiming>, RenderError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        // Tooling may wait; the frame path only polls. First complete draws
        // so their resolves can be encoded, then complete the resolve copies.
        if let Some(last) = self.pending_queries.back().map(|p| p.submission.clone()) {
            self.wait(last, "timestamp draws").await?;
        }
        self.drain_timestamps();
        if !self.pending_queries.is_empty() {
            return Err(RenderError::Readback(
                "timestamp draws: the completion callback did not run after the wait".into(),
            ));
        }
        if let Some(last) = self.pending_timestamps.back().map(|p| p.submission.clone()) {
            for pending in &mut self.pending_timestamps {
                pending.request_map();
            }
            self.wait(last, "timestamp resolve").await?;
            self.drain_timestamps();
        }
        if self.pending_timestamps.is_empty() {
            Ok(std::mem::take(&mut self.timings))
        } else {
            Err(RenderError::Readback(
                "timestamp resolve: the map callback did not run after the wait".into(),
            ))
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn readback(&mut self, surface: SurfaceId) -> Result<Readback, RenderError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        let Some(state) = self.surfaces.get(&surface) else {
            return Err(RenderError::Readback("unknown surface".into()));
        };
        let (w, h) = state.size;
        diag::set_surface(Some(surface.raw()));
        diag::set_phase("readback");
        let bytes_per_row = (w * 8).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(bytes_per_row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        diag::create(&self.device, "readback", buf.size());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &state.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        track_submission(&self.queue, &self.tracker);
        diag::submit(&self.device, &self.queue, "readback");
        tracing::trace!(?surface, ?submission, "readback submitted");
        let slice = buf.slice(..);
        self.map_read(slice, &submission, "the pixel readback")?;
        let data = slice
            .get_mapped_range()
            .expect("buffer range is mapped and not overlapping");
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for row in 0..h {
            let start = (row * bytes_per_row) as usize;
            for px in data[start..start + (w * 8) as usize].as_chunks::<8>().0 {
                let bits: [u16; 4] = bytemuck::cast(*px);
                pixels.push([
                    half::f16::from_bits(bits[0]).to_f32(),
                    half::f16::from_bits(bits[1]).to_f32(),
                    half::f16::from_bits(bits[2]).to_f32(),
                    half::f16::from_bits(bits[3]).to_f32(),
                ]);
            }
        }
        drop(data);
        buf.unmap();
        diag::retire(
            &self.device,
            diag::RetireArgs {
                label: "readback",
                class: diag::Class::MapBuffer,
                bytes: buf.size(),
                used_in_latest_submit: true,
                reason: "readback done",
            },
        );
        diag::set_surface(None);
        diag::set_phase("render");
        Ok(Readback {
            width: w,
            height: h,
            pixels,
        })
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn readback(&mut self, surface: SurfaceId) -> Result<Readback, RenderError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        let Some(state) = self.surfaces.get(&surface) else {
            return Err(RenderError::Readback("unknown surface".into()));
        };
        let (w, h) = state.size;
        diag::set_surface(Some(surface.raw()));
        diag::set_phase("readback");
        let bytes_per_row = (w * 8).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(bytes_per_row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        diag::create(&self.device, "readback", buf.size());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &state.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        track_submission(&self.queue, &self.tracker);
        diag::submit(&self.device, &self.queue, "readback");
        tracing::trace!(?surface, ?submission, "readback submitted");
        let slice = buf.slice(..);
        self.map_read(slice, submission, "the pixel readback")
            .await?;
        let data = slice
            .get_mapped_range()
            .expect("buffer range is mapped and not overlapping");
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for row in 0..h {
            let start = (row * bytes_per_row) as usize;
            for px in data[start..start + (w * 8) as usize].as_chunks::<8>().0 {
                let bits: [u16; 4] = bytemuck::cast(*px);
                pixels.push([
                    half::f16::from_bits(bits[0]).to_f32(),
                    half::f16::from_bits(bits[1]).to_f32(),
                    half::f16::from_bits(bits[2]).to_f32(),
                    half::f16::from_bits(bits[3]).to_f32(),
                ]);
            }
        }
        drop(data);
        buf.unmap();
        diag::retire(
            &self.device,
            diag::RetireArgs {
                label: "readback",
                class: diag::Class::MapBuffer,
                bytes: buf.size(),
                used_in_latest_submit: true,
                reason: "readback done",
            },
        );
        diag::set_surface(None);
        diag::set_phase("render");
        Ok(Readback {
            width: w,
            height: h,
            pixels,
        })
    }

    /// The release queued by a retirement rides its own submission, so
    /// an idle engine still releases the frame's lease at once (#1691).
    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn submit_native_releases(&mut self) {
        self.flush_native_releases();
    }

    /// This platform keeps no native-release queue — retirements queue
    /// nothing to submit.
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    fn submit_native_releases(&mut self) {}
}

/// The queue's retirement frontier as the engine sees it. Every
/// `on_submitted_work_done` callback the engine registers retires one
/// submission in flight order, so the count and the latest recorded
/// submission describe what the GPU has provably finished — the progress
/// signal every native wait reads between windows to tell a queue that is
/// merely slow (still retiring) from one that is stuck.
#[derive(Default)]
struct SubmissionTracker {
    retired_count: AtomicU64,
}

impl SubmissionTracker {
    /// Records one retired submission. Runs inside
    /// `on_submitted_work_done`, which the queue fires in submission order.
    fn retire(&self) {
        self.retired_count.fetch_add(1, Ordering::Release);
    }

    /// Submissions the queue has provably retired so far; only native
    /// waits read the frontier.
    #[cfg(not(target_arch = "wasm32"))]
    fn retired(&self) -> u64 {
        self.retired_count.load(Ordering::Acquire)
    }
}

/// One bounded window of a native GPU wait, repeated until completion or
/// deadlock. The window is a deadline on GPU *progress*, not on the wait
/// itself: a queue that retires any submission inside a window is still
/// draining — the case of a slow adapter such as a CI runner's software
/// rasterizer — so the wait warns once and opens another window; a whole
/// window with zero retirements is a wedged queue and the wait fails as
/// [`RenderError::Timeout`]. Any other poll error means the device is gone
/// and surfaces as [`RenderError::DeviceLost`].
///
/// `poll` performs one `timeout`-bounded device wait; `completed` reads the
/// retirement frontier. `awaited` appears only in diagnostics, so the
/// decision is testable without a device.
#[cfg(not(target_arch = "wasm32"))]
fn wait_with_progress<I: std::fmt::Debug>(
    what: &'static str,
    timeout: Duration,
    awaited: &I,
    mut poll: impl FnMut() -> Result<wgpu::PollStatus, wgpu::PollError>,
    mut completed: impl FnMut() -> u64,
) -> Result<(), RenderError> {
    let start = Instant::now();
    let mut retired = completed();
    loop {
        match poll() {
            Ok(status) => {
                tracing::trace!(
                    what,
                    ?status,
                    wait_ms = start.elapsed().as_secs_f64() * 1e3,
                    "waited"
                );
                return Ok(());
            }
            Err(wgpu::PollError::Timeout) => {
                let now = completed();
                if now == retired {
                    let elapsed = start.elapsed();
                    tracing::error!(
                        what,
                        ?elapsed,
                        ?awaited,
                        "GPU wait timed out — no submission retired in the window"
                    );
                    return Err(RenderError::Timeout { what, timeout });
                }
                tracing::warn!(
                    what,
                    elapsed = ?start.elapsed(),
                    ?awaited,
                    retired_before = retired,
                    retired_after = now,
                    "GPU wait exceeds the warn window — still draining"
                );
                retired = now;
            }
            Err(e) => {
                tracing::error!(what, %e, "GPU wait failed");
                return Err(RenderError::DeviceLost);
            }
        }
    }
}

/// Registers the retirement bump for `submission` — one callback per
/// submission, fired in submission order, keeps `tracker`'s frontier
/// honest. Called for every submission the engine makes; takes the queue
/// and tracker by field so callers inside `&mut self` borrows elsewhere
/// stay field-disjoint.
fn track_submission(queue: &wgpu::Queue, tracker: &Arc<SubmissionTracker>) {
    let tracker = Arc::clone(tracker);
    queue.on_submitted_work_done(move || {
        tracker.retire();
    });
}

impl GpuRenderer {
    const fn target_size(&self, target: &GpuTarget) -> Result<(u32, u32), SurfaceError> {
        let size = match target {
            GpuTarget::Offscreen(offscreen) => offscreen.size,
            GpuTarget::Window(window) => window.size,
            GpuTarget::Texture(texture) => texture.size,
            #[cfg(target_os = "android")]
            GpuTarget::SurfaceControl(target) => target.size(),
            #[cfg(target_os = "linux")]
            GpuTarget::Dmabuf(target) => target.size(),
        };
        if size.0 == 0 || size.1 == 0 {
            return Err(SurfaceError::ZeroSize);
        }
        if size.0 > self.max_texture || size.1 > self.max_texture {
            return Err(SurfaceError::TooLarge {
                width: size.0,
                height: size.1,
                max: self.max_texture,
            });
        }
        Ok(size)
    }

    /// Realizes surface `id` under a `SurfaceControlTarget`'s parent and
    /// returns its refresh range.
    #[cfg(target_os = "android")]
    fn surface_control(
        &mut self,
        id: SurfaceId,
        target: crate::interop::android::SurfaceControlTarget,
    ) -> Result<cherenkov::RefreshRange, SurfaceError> {
        // The context is built on first need (#170); a surface-control
        // target needs it now — planes import frame buffers natively.
        self.ensure_native();
        let native = self.native.as_ref().ok_or_else(|| {
            SurfaceError::UnsupportedTarget(format!(
                "surface control: {}",
                self.native_error
                    .as_deref()
                    .unwrap_or("the device has no Vulkan external-memory support")
            ))
        })?;
        let system = surface_control::planes::Planes::new(
            native.shared.clone(),
            std::sync::Arc::clone(&self.submit_lock),
            &target,
        )?;
        self.planes.insert(id, system);
        self.presenter
            .get_or_insert_with(|| present::Presenter::new(&self.device, self.shader_delivery));
        Ok(target.refresh)
    }

    /// Realizes surface `id`'s bounded pool of exportable dma-buf images
    /// and returns its refresh range (#1687).
    #[cfg(target_os = "linux")]
    fn dmabuf_export(
        &mut self,
        id: SurfaceId,
        target: crate::interop::dmabuf::DmabufTarget,
    ) -> Result<cherenkov::RefreshRange, SurfaceError> {
        // The native context is built on first need (#170); the pool
        // allocates images and exports their memory through it.
        self.ensure_native();
        let native = self.native.as_ref().ok_or_else(|| {
            SurfaceError::UnsupportedTarget(format!(
                "dma-buf export: {}",
                self.native_error
                    .as_deref()
                    .unwrap_or("the device has no Vulkan external-memory support")
            ))
        })?;
        let pool = dmabuf_export::Pool::new(
            &native.shared,
            std::sync::Arc::clone(&self.submit_lock),
            &target,
        )?;
        self.exports.insert(id, pool);
        self.presenter
            .get_or_insert_with(|| present::Presenter::new(&self.device, self.shader_delivery));
        Ok(target.refresh)
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "the frame pipeline: lowers, uploads, encodes and presents in one pass"
    )]
    async fn render_inner(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<(FrameRedraw, FrameCommits), RenderError> {
        let origin = *self.origin.get_or_insert(frame.time.0);
        self.frame_count += 1;
        self.drain_timestamps();
        self.plane_only.clear();
        for sf in frame.surfaces {
            self.observe_static(sf)?;
        }
        let mut dirty: Vec<_> = frame
            .surfaces
            .iter()
            .filter(|sf| sf.changed || self.wants_redraw(&self.surfaces[&sf.id]))
            .collect();
        self.include_ready_planes(frame, &mut dirty);
        // A surface whose only change is new external frames on layers
        // its committed plan promotes presents them through the planes
        // alone — no lowering, no draws, no part blit (#90). Any other
        // change takes the full render path. The admission lives in
        // `self.plane_only`, rebuilt per render, so it can never outlive
        // the frame that made it.
        let mut full = 0;
        for i in 0..dirty.len() {
            let sf = dirty[i];
            if self.plane_only_frames(sf) {
                self.plane_only.push(sf.id);
                self.surfaces
                    .get_mut(&sf.id)
                    .expect("dirty surface must exist")
                    .present_pending = true;
            } else {
                dirty[full] = sf;
                full += 1;
            }
        }
        dirty.truncate(full);
        if dirty.is_empty() {
            diag::set_phase("present");
            return self.present_windows(frame);
        }
        let timing = self.filter_timing(frame, origin);
        self.frame_pass_count = 0;
        self.frame_submission = None;
        self.pass_meta.clear();
        // Produce, then plan, then lower and encode: a rendered
        // producer's new ring frame must be its current frame before
        // `ready_planes`/`lower_content` build this frame's plane
        // candidates, and one render serves every dirty surface.
        self.render_producers(&dirty, frame.time.0).await?;
        // Lower every dirty surface first: the GPU timestamp bracket must
        // start after CPU lowering (rasters, uploads) so it measures GPU
        // work only. Instances, stops and globals are appended frame-wide
        // at per-surface bases so a later surface's upload can't clobber
        // an earlier one before it is encoded.
        // Take the states out so the lowering workers own them.
        let t_lower = Instant::now();
        let inputs: Vec<projective::Inputs> =
            dirty.iter().map(|sf| self.projective_inputs(sf)).collect();
        let mut pending: Vec<SurfaceState> = dirty
            .iter()
            .map(|id| {
                self.surfaces
                    .remove(&id.id)
                    .expect("dirty surface must exist")
            })
            .collect();
        diag::set_phase("lower");
        let results = self.lower_all(&mut pending, &dirty, &inputs);
        for (id, mut surf) in dirty.iter().zip(pending) {
            surf.plane_resources = (self.images_gen, self.image_replacements, surf.interop);
            self.surfaces.insert(id.id, surf);
        }
        let mut inst_base = 0u32;
        let mut stop_base = 0u32;
        let mut globals_base = 0u32;
        let mut result = Ok(());
        for (sf, lowered) in dirty.iter().zip(results) {
            let id = sf.id;
            result = self.lower_surface(id, stats, inst_base, stop_base, globals_base, lowered);
            if result.is_err() {
                break;
            }
            if let Some(surf) = self.surfaces.get(&id) {
                inst_base += u32::try_from(surf.frame.instances.len()).unwrap_or(u32::MAX);
                stop_base += u32::try_from(surf.frame.stops.len()).unwrap_or(u32::MAX);
                globals_base += u32::try_from(surf.frame.passes.len())
                    .unwrap_or(u32::MAX)
                    .saturating_add(surf.frame.reduce_slots());
            }
        }
        let wait = stats.phases.wait_seconds;
        if result.is_ok() {
            result = self.upload_frame(&dirty, stats).await;
        }
        stats.phases.lower_seconds =
            t_lower.elapsed().as_secs_f64() - (stats.phases.wait_seconds - wait);
        tracing::debug!(
            surfaces = dirty.len(),
            lower_ms = stats.phases.lower_seconds * 1e3,
            ok = result.is_ok(),
            "frame lowered"
        );
        if result.is_ok() {
            self.update_producer_wakes();
            for sf in &dirty {
                self.render_shaders(
                    sf.id,
                    frame.time.0.saturating_duration_since(origin).as_secs_f32(),
                )?;
                self.prepare_filters(sf.id).await?;
                // wasm cannot create pipelines inside encode — the
                // variant-3 cells a union or outer-band member needs are
                // prepared here, once the frame is lowered.
                #[cfg(target_arch = "wasm32")]
                self.prepare_union_pipelines(sf.id).await?;
            }
            diag::set_phase("encode");
            let t = Instant::now();
            for sf in &dirty {
                let count = self.frame_pass_count;
                let meta = self.pass_meta.len();
                if let Err(error) = self.encode_surface(sf.id, timing, stats) {
                    self.frame_pass_count = count;
                    self.pass_meta.truncate(meta);
                    result = Err(error);
                    break;
                }
                self.finish_projective(sf.id);
                let surface = self.surfaces.get_mut(&sf.id).expect("rendered surface");
                surface.present_pending = surface.window.is_some()
                    || surface.promotes
                    || self.exports.contains_key(&sf.id);
            }
            self.evict_projective();
            // Set once the encode has consumed the filters' redraw
            // requests (`SurfaceWakes::set`).
            self.update_filter_activity();
            stats.phases.encode_seconds = t.elapsed().as_secs_f64();
            stats.frame = Some(frame.id);
            if self.timestamps && self.frame_pass_count > 0 {
                diag::set_phase("timestamps");
                let t = Instant::now();
                self.queue_timestamps(2 * self.frame_pass_count, frame.id);
                stats.phases.stamp_seconds += t.elapsed().as_secs_f64();
            }
            self.evict_dead_mask_textures();
        }
        result?;
        diag::set_phase("present");
        let (mut redraw, commits) = self.present_windows(frame)?;
        self.request_redraw(&mut redraw);
        Ok((redraw, commits))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[expect(
        clippy::too_many_lines,
        reason = "the frame pipeline: lowers, uploads, encodes and presents in one pass"
    )]
    fn render_inner(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<(FrameRedraw, FrameCommits), RenderError> {
        let origin = *self.origin.get_or_insert(frame.time.0);
        self.frame_count += 1;
        self.drain_timestamps();
        #[cfg(all(unix, not(target_vendor = "apple")))]
        self.flush_native_releases();
        self.plane_only.clear();
        for sf in frame.surfaces {
            self.observe_static(sf)?;
        }
        let mut dirty: Vec<_> = frame
            .surfaces
            .iter()
            .filter(|sf| sf.changed || self.wants_redraw(&self.surfaces[&sf.id]))
            .collect();
        self.include_ready_planes(frame, &mut dirty);
        // A surface whose only change is new external frames on layers
        // its committed plan promotes presents them through the planes
        // alone — no lowering, no draws, no part blit (#90). Any other
        // change takes the full render path. The admission lives in
        // `self.plane_only`, rebuilt per render, so it can never outlive
        // the frame that made it.
        let mut full = 0;
        for i in 0..dirty.len() {
            let sf = dirty[i];
            if self.plane_only_frames(sf) {
                self.plane_only.push(sf.id);
                self.surfaces
                    .get_mut(&sf.id)
                    .expect("dirty surface must exist")
                    .present_pending = true;
            } else {
                dirty[full] = sf;
                full += 1;
            }
        }
        dirty.truncate(full);
        if dirty.is_empty() {
            diag::set_phase("present");
            return self.present_windows(frame);
        }
        let timing = self.filter_timing(frame, origin);
        self.frame_pass_count = 0;
        self.frame_submission = None;
        self.pass_meta.clear();
        // Produce, then plan, then lower and encode: a rendered
        // producer's new ring frame must be its current frame before
        // `ready_planes`/`lower_content` build this frame's plane
        // candidates, and one render serves every dirty surface.
        self.render_producers(&dirty, frame.time.0)?;
        // Lower every dirty surface first: the GPU timestamp bracket must
        // start after CPU lowering (rasters, uploads) so it measures GPU
        // work only. Instances, stops and globals are appended frame-wide
        // at per-surface bases so a later surface's upload can't clobber
        // an earlier one before it is encoded.
        // Take the states out so the lowering workers own them.
        let t_lower = Instant::now();
        let inputs: Vec<projective::Inputs> =
            dirty.iter().map(|sf| self.projective_inputs(sf)).collect();
        let mut pending: Vec<SurfaceState> = dirty
            .iter()
            .map(|id| {
                self.surfaces
                    .remove(&id.id)
                    .expect("dirty surface must exist")
            })
            .collect();
        diag::set_phase("lower");
        let results = self.lower_all(&mut pending, &dirty, &inputs);
        for (id, mut surf) in dirty.iter().zip(pending) {
            surf.plane_resources = (self.images_gen, self.image_replacements, surf.interop);
            self.surfaces.insert(id.id, surf);
        }
        let mut inst_base = 0u32;
        let mut stop_base = 0u32;
        let mut globals_base = 0u32;
        let mut result = Ok(());
        for (sf, lowered) in dirty.iter().zip(results) {
            let id = sf.id;
            result = self.lower_surface(id, stats, inst_base, stop_base, globals_base, lowered);
            if result.is_err() {
                break;
            }
            if let Some(surf) = self.surfaces.get(&id) {
                inst_base += u32::try_from(surf.frame.instances.len()).unwrap_or(u32::MAX);
                stop_base += u32::try_from(surf.frame.stops.len()).unwrap_or(u32::MAX);
                globals_base += u32::try_from(surf.frame.passes.len())
                    .unwrap_or(u32::MAX)
                    .saturating_add(surf.frame.reduce_slots());
            }
        }
        let wait = stats.phases.wait_seconds;
        result = result.and_then(|()| self.upload_frame(&dirty, stats));
        stats.phases.lower_seconds =
            t_lower.elapsed().as_secs_f64() - (stats.phases.wait_seconds - wait);
        tracing::debug!(
            surfaces = dirty.len(),
            lower_ms = stats.phases.lower_seconds * 1e3,
            ok = result.is_ok(),
            "frame lowered"
        );
        if result.is_ok() {
            self.update_producer_wakes();
            for sf in &dirty {
                self.render_shaders(
                    sf.id,
                    frame.time.0.saturating_duration_since(origin).as_secs_f32(),
                )?;
            }
            diag::set_phase("encode");
            let t = Instant::now();
            for sf in &dirty {
                let count = self.frame_pass_count;
                let meta = self.pass_meta.len();
                if let Err(error) = self.encode_surface(sf.id, timing, stats) {
                    self.frame_pass_count = count;
                    self.pass_meta.truncate(meta);
                    result = Err(error);
                    break;
                }
                self.finish_projective(sf.id);
                let surface = self.surfaces.get_mut(&sf.id).expect("rendered surface");
                surface.present_pending = surface.window.is_some()
                    || surface.promotes
                    || self.exports.contains_key(&sf.id);
            }
            self.evict_projective();
            // Set once the encode has consumed the filters' redraw
            // requests (`SurfaceWakes::set`).
            self.update_filter_activity();
            stats.phases.encode_seconds = t.elapsed().as_secs_f64();
            stats.frame = Some(frame.id);
            if self.timestamps && self.frame_pass_count > 0 {
                diag::set_phase("timestamps");
                let t = Instant::now();
                self.queue_timestamps(2 * self.frame_pass_count, frame.id);
                stats.phases.stamp_seconds += t.elapsed().as_secs_f64();
            }
            self.evict_dead_mask_textures();
        }
        result?;
        diag::set_phase("present");
        let (mut redraw, commits) = self.present_windows(frame)?;
        self.request_redraw(&mut redraw);
        Ok((redraw, commits))
    }

    fn filter_timing(&mut self, frame: &Frame<'_>, origin: Instant) -> filtrate::EffectFrameTiming {
        let timing = filtrate::EffectFrameTiming::new(
            frame.time.0.saturating_duration_since(origin),
            self.last_frame.map_or(std::time::Duration::ZERO, |last| {
                frame.time.0.saturating_duration_since(last)
            }),
            frame.id.get(),
        );
        self.last_frame = Some(frame.time.0);
        timing
    }

    /// Sets each producer's redraw wakes to the surfaces whose frames drew
    /// it: a request wakes the host of each, through the surface's own
    /// waker — the same frame-membership model
    /// [`Self::update_filter_activity`] applies to filters, so a binding
    /// whose layer was detached (and may return) wakes nothing until it is
    /// drawn again.
    fn update_producer_wakes(&mut self) {
        let mut uses: FxHashMap<ProducerId, Vec<cherenkov::CompletionWaker>> = FxHashMap::default();
        for surface in self.surfaces.values() {
            for (id, _) in &surface.frame.content {
                let surfaces = uses.entry(*id).or_default();
                if surfaces.last() != Some(&surface.waker) {
                    surfaces.push(surface.waker.clone());
                }
            }
        }
        for (id, producer) in &mut self.producers {
            producer.set_wakes(uses.get(id).map_or(&[][..], Vec::as_slice));
        }
    }

    fn update_filter_activity(&self) {
        // A retained local image keeps the filters its realization ran
        // active: an animating one must make it stale. Each filter's wakes
        // are the surfaces that run it, so a hidden surface's filters wake
        // no host; the frame that shows it runs them.
        let mut uses: FxHashMap<filter::FilterKey, Vec<cherenkov::CompletionWaker>> =
            FxHashMap::default();
        for surface in self.surfaces.values() {
            for key in surface.frame.filters.iter().map(|(_, id)| *id).chain(
                surface.composed.iter().flat_map(|key| {
                    projective_entry(&surface.projective, *key)
                        .deps
                        .filters
                        .iter()
                        .copied()
                }),
            ) {
                let surfaces = uses.entry(key).or_default();
                // Listed surface by surface: one entry per surface.
                if surfaces.last() != Some(&surface.waker) {
                    surfaces.push(surface.waker.clone());
                }
            }
        }
        self.filters.set_surfaces(&uses);
    }
    pub fn add_filter(&mut self, id: cherenkov::FilterId, source: Box<dyn filter::Source>) {
        self.filters.add(filter::FilterKey::Layer(id.raw()), source);
    }
    pub fn remove_filter(&mut self, id: cherenkov::FilterId) {
        self.filters.remove(filter::FilterKey::Layer(id.raw()));
    }
    pub fn add_backdrop_group(
        &mut self,
        surface: SurfaceId,
        id: cherenkov::BackdropId,
        source: Option<Box<dyn filter::Source>>,
        spec: cherenkov::BackdropSpec,
    ) {
        let Some(surf) = self.surfaces.get_mut(&surface) else {
            return;
        };
        let filter = source.map(|source| {
            let key = filter::FilterKey::Backdrop {
                surface: surface.raw(),
                group: id.raw(),
            };
            self.filters.add(key, source);
            key
        });
        surf.backdrop_groups.insert(
            id.raw(),
            BackdropGroupState {
                filter,
                spec,
                captures: Vec::new(),
                resolves: Vec::new(),
            },
        );
    }
    pub fn remove_backdrop_group(&mut self, surface: SurfaceId, id: cherenkov::BackdropId) {
        let Some(surf) = self.surfaces.get_mut(&surface) else {
            return;
        };
        if let Some(state) = surf.backdrop_groups.remove(&id.raw()) {
            if !state.captures.is_empty() {
                surf.bind_gen += 1;
                retire_binds1(
                    surf,
                    &self.device,
                    self.images_gen,
                    self.atlas.mask_texture_generation(),
                    "backdrop group removed",
                    |key| matches!(key.0, Some(Source::Backdrop { group, .. }) if group == id.raw()),
                );
            }
            if let Some(key) = state.filter {
                self.release_backdrop_chain(key, "backdrop group removed");
            }
        }
    }
    /// Unregisters a backdrop group's capture chain and retires its targets.
    fn release_backdrop_chain(&mut self, key: filter::FilterKey, reason: &'static str) {
        let bytes = self.filters.remove(key);
        if bytes > 0 {
            diag::retire(
                &self.device,
                diag::RetireArgs {
                    label: "filter targets",
                    class: diag::Class::Target,
                    bytes,
                    used_in_latest_submit: true,
                    reason,
                },
            );
        }
    }
    /// Validates a user shader paint on the caller thread.
    pub(crate) fn validate_shader(source: &cherenkov::ShaderSource) -> Result<(), ResourceError> {
        paint::validate(source)
    }

    /// Validates a backdrop effect shader on the caller thread: the stock
    /// module with the user's `backdrop_effect` parses and validates.
    pub(crate) fn validate_backdrop_shader(
        source: &cherenkov::BackdropShaderSource,
    ) -> Result<(), ResourceError> {
        // The member may draw under a union: both module shapes validate.
        shaders::validate_wgsl(&backdrop_effect_text(&source.source, false))?;
        shaders::validate_wgsl(&backdrop_effect_text(&source.source, true)).map(drop)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_shader(
        &mut self,
        id: cherenkov::ShaderId,
        source: &cherenkov::ShaderSource,
    ) -> Result<(), ResourceError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        self.shaders.add(&self.device, id.raw(), source)
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn add_shader(
        &mut self,
        id: cherenkov::ShaderId,
        source: &cherenkov::ShaderSource,
    ) -> Result<(), ResourceError> {
        self.shaders.add(&self.device, id.raw(), source).await
    }

    /// Compiles `source` into a backdrop effect pipeline per target
    /// format. The module text is the stock shader with the stub
    /// `backdrop_effect` removed and the user source appended — the
    /// stub sits between two `// backdrop-effect-stub` marker lines, so
    /// removal is a plain string split.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_backdrop_shader(
        &mut self,
        id: cherenkov::BackdropShaderId,
        source: &cherenkov::BackdropShaderSource,
    ) -> Result<(), ResourceError> {
        // The scope covers module creation: invalid WGSL reports at
        // module use, and `create_pipeline` scopes only itself.
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        // The union-shaped text validates now — invalid WGSL still
        // fails fast — but its module builds on the first union use.
        let union_source: std::sync::Arc<str> = backdrop_effect_text(&source.source, true).into();
        shaders::validate_wgsl(&union_source)?;
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("backdrop effect"),
                source: wgpu::ShaderSource::Wgsl(backdrop_effect_text(&source.source, false)),
            });
        let make =
            |module: &wgpu::ShaderModule, format| -> Result<wgpu::RenderPipeline, ResourceError> {
                create_pipeline(
                    &self.device,
                    &self.config,
                    &self.pipeline_layout,
                    self.pipeline_cache.as_ref(),
                    module,
                    format,
                    CoveragePass::Painter(false),
                )
                .map_err(|e| ResourceError::Shader(e.to_string()))
            };
        // One pipeline per distinct target format on the plain variant
        // — equal formats share the slot-0 pipeline (#170).
        let mut pipelines = [None, None];
        for format in [TARGET_FORMAT, self.scratch_format] {
            let slot = usize::from(format != TARGET_FORMAT);
            if pipelines[slot].is_none() {
                pipelines[slot] = Some(make(&module, format));
            }
        }
        let scope_error = pollster::block_on(scope.pop());
        // An invalid module reports through the scope and fails the
        // pipelines only as a consequence: surface it first.
        if let Some(error) = scope_error {
            return Err(ResourceError::Shader(format!("{error}")));
        }
        let [a, b] = pipelines;
        self.backdrop_shaders.insert(
            id.raw(),
            BackdropShaderEntry {
                union_source,
                union_module: None,
                plain: [a.transpose()?, b.transpose()?],
                union: [None, None],
            },
        );
        Ok(())
    }

    /// The wasm variant of [`GpuRenderer::add_backdrop_shader`].
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn add_backdrop_shader(
        &mut self,
        id: cherenkov::BackdropShaderId,
        source: &cherenkov::BackdropShaderSource,
    ) -> Result<(), ResourceError> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        // The union-shaped text validates now — invalid WGSL still
        // fails fast — but its module builds on the first union use in
        // `prepare_union_pipelines`.
        let union_source: std::sync::Arc<str> = backdrop_effect_text(&source.source, true).into();
        shaders::validate_wgsl(&union_source)?;
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("backdrop effect"),
                source: wgpu::ShaderSource::Wgsl(backdrop_effect_text(&source.source, false)),
            });
        // One pipeline per distinct target format on the plain variant
        // — equal formats share the slot-0 pipeline (#170).
        let mut pipelines = [None, None];
        for format in [TARGET_FORMAT, self.scratch_format] {
            let slot = usize::from(format != TARGET_FORMAT);
            if pipelines[slot].is_none() {
                pipelines[slot] = Some(
                    create_pipeline(
                        &self.device,
                        &self.config,
                        &self.pipeline_layout,
                        self.pipeline_cache.as_ref(),
                        &module,
                        format,
                        CoveragePass::Painter(false),
                    )
                    .await
                    .map_err(|e| ResourceError::Shader(e.to_string())),
                );
            }
        }
        // The scope is popped before `?` propagates: an early return must
        // not leak an unbalanced error scope. An invalid module reports
        // through the scope and fails the pipelines only as a
        // consequence: surface it first.
        let scope_error = scope.pop().await;
        if let Some(error) = scope_error {
            return Err(ResourceError::Shader(format!("{error}")));
        }
        let [a, b] = pipelines;
        self.backdrop_shaders.insert(
            id.raw(),
            BackdropShaderEntry {
                union_source,
                union_module: None,
                plain: [a.transpose()?, b.transpose()?],
                union: [None, None],
            },
        );
        Ok(())
    }

    /// Frees a backdrop effect shader's pipelines. A member that still
    /// samples it fails at encode with a render error.
    pub fn remove_backdrop_shader(&mut self, id: cherenkov::BackdropShaderId) {
        self.backdrop_shaders.remove(&id.raw());
    }

    pub fn remove_shader(&mut self, id: cherenkov::ShaderId) {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        self.shaders.remove(id.raw());
        for surface in self.surfaces.values_mut() {
            surface
                .shader_textures
                .retain(|key, _| key.shader != id.raw());
            surface.binds1.clear();
            // A retained local image never names a released shader. The
            // release runs once no installed content draws the shader, so
            // an image that sampled it is one no frame composes again.
            retire_projective(surface, &self.device, "shader released", |_, entry| {
                entry.deps.images.iter().any(|image| {
                    matches!(image, lower::ImageSource::Shader(key) if key.shader == id.raw())
                })
            });
        }
    }
    fn render_shaders(&mut self, id: SurfaceId, elapsed: f32) -> Result<(), RenderError> {
        let surface = self.surfaces.get_mut(&id).expect("registered surface");
        if self.shaders.has_registrations() {
            let keys: FxHashSet<_> = surface
                .frame
                .passes
                .iter()
                .flat_map(|pass| &pass.ranges)
                .filter_map(|range| match &range.image {
                    Some(lower::ImageSource::Shader(key)) => Some(std::sync::Arc::clone(key)),
                    _ => None,
                })
                .collect();
            let count = surface.shader_textures.len();
            surface.shader_textures.retain(|key, _| keys.contains(key));
            if count != surface.shader_textures.len() {
                surface.binds1.clear();
            }
            for key in keys {
                self.shaders.render(
                    &self.device,
                    &self.queue,
                    &key,
                    &mut surface.shader_textures,
                    elapsed,
                )?;
            }
        }
        Ok(())
    }

    /// Renders every producer a drawn binding asked for this frame,
    /// before planning reads its current frame: the wanted set is each
    /// surface's bindings on the layers the frame's tree draws — a
    /// producer whose bound layers are all outside the tree renders
    /// nothing — folded into one output attachment at the componentwise
    /// largest requested size and scale, rendered at most once per
    /// engine frame.
    #[cfg(not(target_arch = "wasm32"))]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "validated display scale fits f32"
    )]
    fn render_producers(
        &mut self,
        dirty: &[&SurfaceFrame<'_>],
        time: Instant,
    ) -> Result<(), RenderError> {
        let mut wanted: FxHashMap<ProducerId, ((u32, u32), f32)> = FxHashMap::default();
        for sf in dirty {
            let surface = self.surfaces.get(&sf.id).expect("registered surface");
            if surface.bindings.is_empty() {
                continue;
            }
            for (layer, _) in sf.tree.layers() {
                let Some(binding) = surface.bindings.get(&layer) else {
                    continue;
                };
                let (wanted_size, wanted_scale) = wanted
                    .entry(binding.producer())
                    .or_insert((binding.size, sf.display.scale as f32));
                wanted_size.0 = wanted_size.0.max(binding.size.0);
                wanted_size.1 = wanted_size.1.max(binding.size.1);
                *wanted_scale = wanted_scale.max(sf.display.scale as f32);
            }
        }
        for (id, (size, scale)) in wanted {
            self.producers
                .get_mut(&id)
                .expect("composed GPU producer")
                .render(&self.adapter, &self.device, &self.queue, time, size, scale)?;
        }
        Ok(())
    }

    /// [`render_producers`](Self::render_producers), on the browser
    /// executor.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "validated display scale fits f32"
    )]
    async fn render_producers(
        &mut self,
        dirty: &[&SurfaceFrame<'_>],
        time: Instant,
    ) -> Result<(), RenderError> {
        let mut wanted: FxHashMap<ProducerId, ((u32, u32), f32)> = FxHashMap::default();
        for sf in dirty {
            let surface = self.surfaces.get(&sf.id).expect("registered surface");
            if surface.bindings.is_empty() {
                continue;
            }
            for (layer, _) in sf.tree.layers() {
                let Some(binding) = surface.bindings.get(&layer) else {
                    continue;
                };
                let (wanted_size, wanted_scale) = wanted
                    .entry(binding.producer())
                    .or_insert((binding.size, sf.display.scale as f32));
                wanted_size.0 = wanted_size.0.max(binding.size.0);
                wanted_size.1 = wanted_size.1.max(binding.size.1);
                *wanted_scale = wanted_scale.max(sf.display.scale as f32);
            }
        }
        for (id, (size, scale)) in wanted {
            self.producers
                .get_mut(&id)
                .expect("composed GPU producer")
                .render(&self.adapter, &self.device, &self.queue, time, size, scale)
                .await?;
        }
        Ok(())
    }

    /// Every visible surface that wants the next frame asks for it on
    /// its own entry: a filter, animated shader or producer source
    /// names exactly the surface it draws into — one animating surface
    /// never marks every other surface's deadline.
    fn request_redraw(&self, redraw: &mut FrameRedraw) {
        for (id, surface) in &self.surfaces {
            if surface.visibility != Visibility::Visible {
                continue;
            }
            if surface
                .frame
                .filters
                .iter()
                .any(|(_, id)| self.filters.wants_redraw(*id))
                || surface.content_wants_redraw(&self.producers)
                || surface
                    .shader_textures
                    .keys()
                    .any(|key| self.shaders.animated(key))
            {
                redraw.request(*id, surface.refresh.clone());
            }
        }
    }

    /// Opens a window target. Apple windows expose the view's layer: the
    /// engine builds its planes under it and presents its parts there, so
    /// no single swapchain exists. A plane's main-queue attach wakes the
    /// host through the surface's `waker` when it lands, pulling the frame
    /// that promotes the born candidate.
    #[cfg(target_vendor = "apple")]
    fn open_window(
        &mut self,
        id: SurfaceId,
        window: crate::WindowTarget,
        size: (u32, u32),
        waker: cherenkov::CompletionWaker,
    ) -> Option<present::WindowSurface> {
        let system = planes::apple::LayerPlanes::new(
            &self.instance,
            &self.adapter,
            &self.device,
            window.parent,
            size,
            window.output,
            window.probe,
            waker,
        );
        self.planes.insert(id, system);
        self.presenter
            .get_or_insert_with(|| present::Presenter::new(&self.device, self.shader_delivery));
        None
    }

    /// Opens a window target: one swapchain the surface's target is blitted
    /// onto.
    #[cfg(not(target_vendor = "apple"))]
    fn open_window(
        &mut self,
        _id: SurfaceId,
        window: crate::WindowTarget,
        size: (u32, u32),
    ) -> Result<Option<present::WindowSurface>, SurfaceError> {
        let surface = present::WindowSurface::new(
            &self.instance,
            &self.adapter,
            &self.device,
            window.handle,
            size,
            window.output,
            window.probe,
        )?;
        self.presenter
            .get_or_insert_with(|| present::Presenter::new(&self.device, self.shader_delivery));
        Ok(Some(surface))
    }

    /// Registers a rendered producer's content; the first drawn binding
    /// runs its `setup` (`cherenkov::GpuContent`). A handle created and
    /// dropped while this add was still queued already retired the
    /// producer: its retirement can beat the add on the producer's own
    /// queue, so a `pending_retire` id lands already retired.
    pub fn add_gpu_producer(&mut self, id: ProducerId, content: crate::interop::GpuContentBox) {
        if self.pending_retire.remove(&id) {
            return;
        }
        self.producers
            .insert(id, gpu_content::Producer::rendered(content));
    }

    /// Registers a submitted-frame producer (`cherenkov::GpuContent`). A
    /// frame producer has no setup: the first [`Self::submit_frame`]
    /// supplies its frame, and the render loop wakes the surfaces the
    /// frame lands on.
    pub fn add_frame_producer(&mut self, id: ProducerId) {
        if self.pending_retire.remove(&id) {
            return;
        }
        self.producers
            .insert(id, gpu_content::Producer::submitted());
    }

    /// Retires a producer: its last handle dropped, so no binding of it
    /// remains — only its device resources are still registered. A
    /// retirement that beat its producer's registration to this thread
    /// is remembered: the add lands already retired.
    pub fn retire_gpu_producer(&mut self, id: ProducerId) {
        if self.producers.remove(&id).is_none() {
            self.pending_retire.insert(id);
        }
    }

    /// Binds a layer on `surface` to `producer` at the size the layer
    /// needs (`cherenkov::GpuContent`). A layer has one content kind at a
    /// time; a size change is a new binding. Returns the current frame's
    /// declared alpha for the layer's alpha contract — `None` before the
    /// producer's first frame.
    pub fn bind_gpu_producer(
        &mut self,
        surface: SurfaceId,
        layer: LayerId,
        producer: &cherenkov::GpuProducer<crate::Gpu>,
        size: (u32, u32),
    ) -> Option<bool> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        let state = self.surfaces.get_mut(&surface).expect("GPU surface exists");
        state.layers.remove(&layer);
        state.hosted.remove(&layer);
        Self::unbind_producer(
            state,
            &self.device,
            self.images_gen,
            self.atlas.mask_texture_generation(),
            layer,
        );
        state
            .bindings
            .insert(layer, gpu_content::Binding::new(producer.clone(), size));
        state.interop += 1;
        let producer = self.producers.get(&producer.id());
        assert!(producer.is_some(), "GPU producer registered");
        producer
            .and_then(gpu_content::Producer::current)
            .map(|slot| slot.frame.alpha() == crate::interop::RgbAlpha::Opaque)
    }

    /// Binds a layer on `surface` to a hosted system layer at `extent` in
    /// its content coordinates (`cherenkov::HostedLayers`). A layer has one
    /// content kind at a time. The object shows in one place: a binding of
    /// it on another layer — of this surface or another — is released, and
    /// that surface lowers again so its planes let it go.
    #[cfg(any(target_vendor = "apple", target_os = "android"))]
    pub fn bind_hosted(
        &mut self,
        surface: SurfaceId,
        layer: LayerId,
        object: planes::Hosted,
        extent: kurbo::Size,
    ) {
        for (&id, state) in &mut self.surfaces {
            let before = state.hosted.len();
            state
                .hosted
                .retain(|&at, bound| (id, at) == (surface, layer) || !bound.object.is(&object));
            if state.hosted.len() != before {
                state.plan_dirty = true;
            }
        }
        let state = self.surfaces.get_mut(&surface).expect("GPU surface exists");
        state.layers.remove(&layer);
        Self::unbind_producer(
            state,
            &self.device,
            self.images_gen,
            self.atlas.mask_texture_generation(),
            layer,
        );
        state
            .hosted
            .insert(layer, planes::HostedBinding { object, extent });
        state.interop += 1;
    }

    /// Releases `layer`'s binding, if any: the bind groups sampling the
    /// producer's current frame retire. A drop that held the producer's
    /// last reference posts its retirement onto the producer's own
    /// queue — it never blocks this thread on the channel it drains.
    fn unbind_producer(
        state: &mut SurfaceState,
        device: &wgpu::Device,
        images_gen: u64,
        mask_gen: u64,
        layer: LayerId,
    ) {
        let Some(binding) = state.bindings.remove(&layer) else {
            return;
        };
        let id = binding.producer();
        state.interop += 1;
        // #169 A4: only bind groups referencing this producer's
        // attachment went stale — drop them, keep the rest.
        retire_binds1(
            state,
            device,
            images_gen,
            mask_gen,
            "producer unbound",
            |key| key.2 == Some(lower::ImageSource::Content(id)),
        );
    }

    /// Detaches every live producer and drops its device resources —
    /// the device-replacement contract: a rendered producer's content
    /// comes back for the next renderer to register again (its first
    /// drawn binding runs `setup` on the new device); a frame
    /// producer's frame dropped with the device — its sink's next
    /// submit supplies one.
    pub fn drain_gpu_producers(
        &mut self,
    ) -> Vec<(ProducerId, cherenkov::DrainedProducer<crate::Gpu>)> {
        for surface in self.surfaces.values_mut() {
            if !surface.bindings.is_empty() {
                surface.interop += 1;
                surface.bindings.clear();
            }
        }
        self.producers
            .drain()
            .map(|(id, producer)| {
                (
                    id,
                    producer.into_content().map_or_else(
                        || cherenkov::DrainedProducer::Frame,
                        cherenkov::DrainedProducer::Rendered,
                    ),
                )
            })
            .collect()
    }

    /// Installs a submitted frame as `producer`'s current frame
    /// (`cherenkov::GpuContent`): every binding of the producer shows it,
    /// sampled where the frame lands — never copied or rasterized — and
    /// the layers bound on any surface come back for the surface's plane
    /// bookkeeping.
    pub fn submit_frame(
        &mut self,
        id: ProducerId,
        frame: crate::interop::ExternalFrame,
    ) -> Vec<(SurfaceId, LayerId)> {
        // Registration prepares the external-frame family — the layout
        // and pipelines, and for a native frame the Vulkan context — so
        // a cold frame's encode does not pay their creation (#170, #165).
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let needs_native = matches!(&frame.planes, crate::interop::FramePlanes::Native(_));
        let mut bound = Vec::new();
        for (sid, state) in &mut self.surfaces {
            let mut draws = false;
            for (layer, binding) in &state.bindings {
                if binding.producer() == id {
                    bound.push((*sid, *layer));
                    draws = true;
                }
            }
            if draws {
                // A new frame on a bound layer is the layer's own frame
                // swap: retained binds of the producer's slot re-lower,
                // and the layer's plane hands the system a new buffer.
                state.interop += 1;
                #[cfg(target_os = "android")]
                if state.promotes && !<planes::Platform as planes::Compositor>::shows(&frame) {
                    let reason = match &frame.planes {
                        crate::interop::FramePlanes::Native(native) => {
                            surface_control::planes::ineligible(native)
                                .unwrap_or(surface_control::planes::Ineligible::NotABuffer)
                        }
                        _ => surface_control::planes::Ineligible::NotABuffer,
                    };
                    tracing::debug!(target: "cherenkov::planes", producer = ?id, reason = ?reason, "frame producer eligibility");
                }
            }
        }
        if let Some(producer) = self.producers.get_mut(&id) {
            producer.submit(&self.device, &self.queue, frame);
        }
        if let Err(error) = self.ensure_external() {
            tracing::warn!(%error, "external-frame preparation failed at submission");
        }
        #[cfg(all(unix, not(target_vendor = "apple")))]
        if needs_native {
            self.ensure_native();
        }
        bound
    }

    /// Builds the Vulkan native external-frame context on first need — a
    /// `FramePlanes::Native` registration or a surface-control target
    /// (#170). A failure is recorded once; [`Self::native_missing`]
    /// reports it from then on.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn ensure_native(&mut self) {
        if self.native.is_some() || self.native_error.is_some() {
            return;
        }
        let shared = crate::interop::SharedDevice {
            instance: self.instance.clone(),
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
        };
        match external::vulkan::shared_for(&shared).and_then(external::vulkan::Native::new) {
            Ok(native) => self.native = Some(native),
            Err(error) => {
                tracing::warn!(%error, "vulkan external-frame context unavailable");
                self.native_error = Some(error.to_string());
            }
        }
    }

    /// Builds the external-frame group-1 layout and both format pipelines
    /// on the first surface draw that samples an external slot.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::unnecessary_wraps,
            reason = "browser WebGPU reports pipeline errors asynchronously, so only the native error scope can fail here"
        )
    )]
    fn ensure_external(&mut self) -> Result<(), RenderError> {
        if self.ext_layout.is_some() {
            return Ok(());
        }
        let ext_layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cherenkov external 1"),
                entries: &layout_entries(bindings::EXTERNAL_GROUP1),
            });
        let module = self.shader_delivery.external_module(&self.device);
        // Error scopes resolve asynchronously; only the native path pops
        // synchronously. A failure here is an engine bug, so the wasm path
        // reports through the uncaptured-error handler instead.
        #[cfg(not(target_arch = "wasm32"))]
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        // One pipeline per distinct target format — equal formats share
        // the slot-0 pipeline (#170).
        for format in [TARGET_FORMAT, self.scratch_format] {
            let slot = usize::from(format != TARGET_FORMAT);
            if self.external_pipes[slot].is_none() {
                self.external_pipes[slot] = Some(create_external_pipeline(
                    &self.device,
                    &self.layout0,
                    &ext_layout,
                    &module,
                    format,
                ));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(RenderError::Render(format!("external pipeline: {error}")));
        }
        self.ext_layout = Some(ext_layout);
        Ok(())
    }

    /// The error a native-frame draw reports when the Vulkan context is
    /// absent — carrying the registration-time failure when one was
    /// recorded (#166, #170).
    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn native_missing(&self) -> RenderError {
        RenderError::Render(self.native_error.as_deref().map_or_else(
            || {
                "a native external frame is installed but the Vulkan context is unavailable"
                    .to_string()
            },
            |error| format!("vulkan external-frame context unavailable: {error}"),
        ))
    }

    /// Builds the projective composite and mip pipelines on the first
    /// frame that composes a projective layer.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::unnecessary_wraps,
            reason = "browser WebGPU reports pipeline errors asynchronously, so only the native error scope can fail here"
        )
    )]
    fn ensure_projective(&mut self) -> Result<(), RenderError> {
        if self.projective.is_some() {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = projective::Pipelines::new(
            &self.device,
            self.shader_delivery,
            &self.layout0,
            self.scratch_format,
        );
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(RenderError::Render(format!("projective pipeline: {error}")));
        }
        self.projective = Some(pipelines);
        Ok(())
    }

    /// Bytes of every retained local image.
    fn projective_bytes(&self) -> u64 {
        self.surfaces
            .values()
            .map(SurfaceState::projective_bytes)
            .sum()
    }

    /// The bytes local images may hold: the GPU budget less every other
    /// resident resource.
    fn projective_available(&self) -> u64 {
        let others = self.memory().gpu.0 - self.projective_bytes();
        self.config.budget.gpu.0.saturating_sub(others)
    }

    /// What surface `sf`'s lowering needs to plan and reuse local images.
    /// First drops every retained image no frame can compose again: its
    /// layer is gone or affine, its content stamp moved on, or an image
    /// was replaced since. A resource released under #199's deferred
    /// release is drawn by no installed content, so every image that read
    /// it is among these: none outlives the frame of its release.
    fn projective_inputs(&mut self, sf: &SurfaceFrame<'_>) -> projective::Inputs {
        let replacements = self.image_replacements;
        let tree = sf.tree;
        retire_projective(
            self.surfaces.get_mut(&sf.id).expect("registered surface"),
            &self.device,
            "projective content changed",
            |layer, entry| {
                tree.projective_pose(layer).is_none()
                    || !entry
                        .key
                        .is_some_and(|key| key.is_current(tree.content_stamp(layer), replacements))
            },
        );
        let surf = &self.surfaces[&sf.id];
        projective::Inputs {
            limits: cherenkov::lowering::projective::Limits {
                max_dimension: self.max_texture,
                max_bytes: self.projective_available(),
            },
            replacements,
            interop: surf.interop,
            stale: surf.stale_projective(&self.filters, &self.shaders, &self.producers),
        }
    }

    /// Admits and allocates surface `id`'s local images for this frame:
    /// every image it composes must fit the budget together, and every
    /// image it realizes gets a texture of its size, reusing the bucket's
    /// texture when the size is unchanged.
    fn allocate_projective(
        &mut self,
        id: SurfaceId,
        realize: Vec<projective::Realize>,
        composed: Vec<projective::LocalKey>,
    ) -> Result<(), RenderError> {
        if !composed.is_empty() {
            self.ensure_projective()?;
        }
        let available = self.projective_available();
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        let mut required = 0_u64;
        for key in &composed {
            required += realize.iter().find(|r| r.key == *key).map_or_else(
                || projective_entry(&surf.projective, *key).bytes(),
                |r| {
                    r.levels
                        .iter()
                        .map(|&(w, h)| 8 * u64::from(w) * u64::from(h))
                        .sum()
                },
            );
            if required > available {
                return Err(RenderError::ProjectiveUnsupported {
                    layer: key.layer,
                    reason: format!(
                        "the frame's projective images need {required} bytes; the GPU budget admits {available}"
                    ),
                });
            }
        }
        for r in &realize {
            let entries = surf.projective.entry(r.key.layer).or_default();
            let slot = entries.iter().position(|e| e.bucket == r.key.bucket);
            if let Some(entry) = slot.map(|i| &mut entries[i])
                && entry.fits(r.levels[0])
            {
                entry.key = None;
                continue;
            }
            let pipelines = self.projective.as_ref().expect("built above");
            let entry = projective::Entry::new(&self.device, pipelines, r.key.bucket, &r.levels);
            let bytes = entry.bytes();
            if let Some(i) = slot {
                let old = std::mem::replace(&mut entries[i], entry);
                diag::grow(
                    &self.device,
                    "projective image",
                    diag::Class::Target,
                    old.bytes(),
                    bytes,
                    0,
                    true,
                );
            } else {
                entries.push(entry);
                diag::create(&self.device, "projective image", bytes);
            }
        }
        surf.realized = realize;
        surf.composed = composed;
        Ok(())
    }

    /// After surface `id`'s frame encoded: its realized images are current,
    /// and every image it composed was used this frame.
    fn finish_projective(&mut self, id: SurfaceId) {
        let frame = self.frame_count;
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return;
        };
        for r in std::mem::take(&mut surf.realized) {
            let entry = projective_entry_mut(&mut surf.projective, r.key);
            entry.key = Some(r.cache);
            entry.deps = r.deps;
        }
        for key in &surf.composed {
            projective_entry_mut(&mut surf.projective, *key).last_used = frame;
        }
        for capture in surf
            .static_layers
            .values_mut()
            .filter_map(|entry| entry.capture.as_mut())
        {
            capture.dirty = false;
        }
    }

    /// Evicts least-recently-used local images until the retained set fits
    /// the budget; the images each surface's latest frame composed are
    /// required and never evicted.
    fn evict_projective(&mut self) {
        let available = self.projective_available();
        let mut total = self.projective_bytes();
        if total <= available {
            return;
        }
        let mut optional: Vec<(u64, SurfaceId, projective::LocalKey)> = self
            .surfaces
            .iter()
            .flat_map(|(sid, surf)| {
                surf.projective.iter().flat_map(move |(layer, entries)| {
                    entries
                        .iter()
                        .map(move |e| {
                            (
                                e.last_used,
                                *sid,
                                projective::LocalKey {
                                    layer: *layer,
                                    bucket: e.bucket,
                                },
                            )
                        })
                        .filter(|(_, _, key)| !surf.composed.contains(key))
                })
            })
            .collect();
        optional.sort_unstable_by_key(|(last, ..)| *last);
        for (_, sid, key) in optional {
            if total <= available {
                break;
            }
            let Some(entries) = self
                .surfaces
                .get_mut(&sid)
                .and_then(|surf| surf.projective.get_mut(&key.layer))
            else {
                continue;
            };
            if let Some(i) = entries.iter().position(|e| e.bucket == key.bucket) {
                let entry = entries.swap_remove(i);
                total -= entry.bytes();
                diag::retire(
                    &self.device,
                    diag::RetireArgs {
                        label: "projective image",
                        class: diag::Class::Target,
                        bytes: entry.bytes(),
                        used_in_latest_submit: false,
                        reason: "projective eviction",
                    },
                );
            }
        }
    }

    /// Presents every window surface with a pending present, returning
    /// the requests of the surfaces whose present must be retried.
    fn present_windows(
        &mut self,
        frame: &Frame<'_>,
    ) -> Result<(FrameRedraw, FrameCommits), RenderError> {
        let mut redraw = FrameRedraw::default();
        let mut commits = FrameCommits::default();
        if self.presenter.is_none() {
            return Ok((redraw, commits));
        }
        let currents: FxHashMap<ProducerId, &external::Slot> = self
            .producers
            .iter()
            .filter_map(|(id, producer)| producer.current().map(|slot| (*id, slot)))
            .collect();
        for sf in frame.surfaces {
            let surface = self.surfaces.get_mut(&sf.id).expect("registered surface");
            // A headroom-only frame asks for a present without lowering
            // new content (#98). The front end marks `present_pending`
            // only on surfaces it reported as presenting
            // (`SurfaceInfo::presents`), so a pending present implies a
            // window here.
            surface.present_pending |= sf.present_pending;
            // A display move or a scale change re-runs the window's
            // output negotiation; it reconfigures only when the selected
            // pair moves. A headroom-only update never re-enumerates —
            // the host announces a move with `Surface::display_moved`,
            // since a move to a numerically identical display is
            // invisible in `Display`'s values (#98).
            let renegotiate =
                sf.display_moved || sf.display.scale.to_bits() != surface.display.scale.to_bits();
            surface.display = sf.display;
            if renegotiate {
                Self::reselect_surface(
                    self.planes.get_mut(&sf.id),
                    &self.adapter,
                    &self.device,
                    surface,
                )?;
            }
            if surface.present_pending {
                // A dma-buf surface presents a fresh pool image per
                // presented frame; `Retry` keeps `present_pending` for the
                // next frame (#1687).
                #[cfg(target_os = "linux")]
                let exported = self
                    .exports
                    .get_mut(&sf.id)
                    .map(|export| {
                        export.present(
                            &self.device,
                            &self.queue,
                            self.presenter.as_mut().expect("checked above"),
                            &surface.view,
                            sf.display.headroom,
                        )
                    })
                    .transpose()?;
                #[cfg(not(target_os = "linux"))]
                let exported = None;
                let (presentation, commit) = match exported {
                    Some(presentation) => (presentation, PlatformCommit::default()),
                    None => Self::present_surface(
                        self.planes.get_mut(&sf.id),
                        self.plane_only.contains(&sf.id),
                        (
                            &self.device,
                            &self.queue,
                            self.presenter.as_mut().expect("checked above"),
                        ),
                        surface,
                        sf,
                        &currents,
                    )?,
                };
                commits.push(commit);
                surface.present_pending = presentation != planes::Presentation::Presented;
                if presentation == planes::Presentation::Presented {
                    for capture in surface
                        .static_layers
                        .values_mut()
                        .filter_map(|entry| entry.capture.as_mut())
                    {
                        capture.source = None;
                    }
                }
                if surface.present_pending
                    && let Some(system) = self.planes.get_mut(&sf.id)
                {
                    planes::SystemPlanes::withdraw_animations(system);
                }
                if presentation == planes::Presentation::Retry {
                    redraw.request(sf.id, surface.refresh.clone());
                }
            }
            if !surface.present_pending
                && let Some(system) = self.planes.get_mut(&sf.id)
            {
                planes::SystemPlanes::animate(system, sf.tree, &surface.plan);
            }
        }
        Ok((redraw, commits))
    }

    /// A display move or a scale change re-runs the window's output
    /// negotiation — the window's and the plane system's — which
    /// reconfigures only when the selected pair moves, and asks for a
    /// present either way.
    fn reselect_surface(
        system: Option<&mut planes::Platform>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        surface: &mut SurfaceState,
    ) -> Result<(), RenderError> {
        if let Some(window) = &mut surface.window {
            window
                .reselect(adapter, device)
                .map_err(|error| RenderError::Render(error.to_string()))?;
            surface.present_pending = true;
        }
        if let Some(system) = system {
            planes::SystemPlanes::reselect(system, adapter, device)
                .map_err(|error| RenderError::Render(error.to_string()))?;
            surface.present_pending = true;
        }
        Ok(())
    }

    /// Presents `sf`'s surface: through its plane system when it has one —
    /// the frame-swap refresh when this render admitted frames only, a
    /// full compose otherwise — and through the window presenter when it
    /// has none. A dma-buf surface presents through its export pool
    /// before this is reached.
    fn present_surface(
        system: Option<&mut planes::Platform>,
        frames_only: bool,
        present: (&wgpu::Device, &wgpu::Queue, &mut present::Presenter),
        surface: &SurfaceState,
        sf: &SurfaceFrame<'_>,
        currents: &FxHashMap<ProducerId, &external::Slot>,
    ) -> Result<(planes::Presentation, PlatformCommit), RenderError> {
        let (device, queue, presenter) = present;
        match system {
            Some(system) => {
                // The frame's own `plane_frames` carries the update set —
                // admitted by this render's `plane_only`, so it cannot be
                // a stale frame's.
                if frames_only {
                    // The frame's only change is new frames on these
                    // promoted layers: present them alone, leaving every
                    // part's shown buffer in place (#90).
                    planes::SystemPlanes::refresh(
                        system,
                        plane_stack(surface, currents, sf.plane_frames),
                    )?;
                    Ok((planes::Presentation::Presented, PlatformCommit::default()))
                } else {
                    let parts: Vec<_> = (0..surface.plan.parts())
                        .map(|n| planes::Part {
                            view: match n {
                                0 => &surface.view,
                                n => &surface.parts[n - 1].1,
                            },
                        })
                        .collect();
                    let stack: Vec<_> = plane_stack(surface, currents, None).collect();
                    planes::SystemPlanes::compose(
                        system,
                        planes::Composition {
                            device,
                            queue,
                            presenter,
                            size: surface.size,
                            display: sf.display,
                            parts: &parts,
                            planes: &stack,
                        },
                    )
                }
            }
            None => Ok((
                if presenter.present(
                    device,
                    queue,
                    surface.window.as_ref().expect("pending window"),
                    &surface.view,
                    sf.display.headroom,
                )? {
                    planes::Presentation::Presented
                } else {
                    planes::Presentation::Retry
                },
                PlatformCommit::default(),
            )),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn lower_all(
        &mut self,
        pending: &mut [SurfaceState],
        frames: &[&SurfaceFrame<'_>],
        inputs: &[projective::Inputs],
    ) -> Vec<Result<Lowered, RenderError>> {
        let mut grew = false;
        loop {
            let group_maps: Vec<FxHashMap<u64, BackdropGroupInfo>> = pending
                .iter()
                .map(|surf| surf.backdrop_info(&mut self.filters))
                .collect();
            self.ready_planes(frames, pending);
            // The workers share the producers' current frames, not the
            // producers: a rendered producer's content is render-thread
            // state (`!Sync`), and a frame slot is all the lowering
            // reads anyway.
            let currents: FxHashMap<ProducerId, &external::Slot> = self
                .producers
                .iter()
                .filter_map(|(id, producer)| producer.current().map(|slot| (*id, slot)))
                .collect();
            let producers = &currents;
            let mut results: Vec<Result<Lowered, RenderError>> = if pending.len() > 1 {
                let (atlas, images, bitmaps) = (&self.atlas, &self.images, &self.bitmaps);
                // `FontData`'s COLR cache is a `RefCell` — !Sync — so
                // each worker moves in its own snapshot built here.
                let snapshots: Vec<FxHashMap<u64, FontData>> = pending
                    .iter()
                    .map(|_| {
                        self.fonts
                            .iter()
                            .map(|(id, f)| (*id, f.snapshot()))
                            .collect()
                    })
                    .collect();
                std::thread::scope(|s| {
                    pending
                        .iter_mut()
                        .zip(snapshots)
                        .zip(frames)
                        .zip(group_maps.iter().zip(inputs).zip(self.ready_sets.iter()))
                        .map(|(((surf, fonts), frame), ((groups, inputs), ready))| {
                            s.spawn(move || {
                                Self::lower_content(
                                    surf,
                                    frame,
                                    GlyphResources {
                                        atlas,
                                        fonts: &fonts,
                                        images,
                                        bitmaps,
                                        producers,
                                    },
                                    groups,
                                    inputs,
                                    ready,
                                )
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                        .collect()
                })
            } else {
                pending
                    .iter_mut()
                    .zip(frames)
                    .zip(group_maps.iter().zip(inputs).zip(self.ready_sets.iter()))
                    .map(|((surf, frame), ((groups, inputs), ready))| {
                        Self::lower_content(
                            surf,
                            frame,
                            GlyphResources {
                                atlas: &self.atlas,
                                fonts: &self.fonts,
                                images: &self.images,
                                bitmaps: &self.bitmaps,
                                producers,
                            },
                            groups,
                            inputs,
                            ready,
                        )
                    })
                    .collect()
            };
            // The batch's pending rasters commit transactionally
            // (#169 A3): a dry run decides fit / grow once / recycle
            // before any placement or upload happens, so a failed
            // placement never enqueues uploads into an atlas the same
            // preparation abandons.
            match self.commit_rasters(pending, &mut results, grew) {
                Commit::Done => break results,
                Commit::Resize(size, pages) => {
                    self.atlas.resize_to(&self.device, size, pages);
                    grew = true;
                    tracing::debug!(
                        size = self.atlas.size(),
                        pages = self.atlas.pages(),
                        generation = self.atlas.generation(),
                        "atlas resized"
                    );
                }
            }
            // Resizing emptied the atlas: every hit any lowering took is
            // now a miss, so lower the whole batch again.
            diag::event(
                &self.device,
                diag::EventKind::Phase {
                    name: "atlas retry",
                },
            );
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn lower_all(
        &mut self,
        pending: &mut [SurfaceState],
        frames: &[&SurfaceFrame<'_>],
        inputs: &[projective::Inputs],
    ) -> Vec<Result<Lowered, RenderError>> {
        let mut grew = false;
        loop {
            let group_maps: Vec<FxHashMap<u64, BackdropGroupInfo>> = pending
                .iter()
                .map(|surf| surf.backdrop_info(&mut self.filters))
                .collect();
            self.ready_planes(frames, pending);
            let currents: FxHashMap<ProducerId, &external::Slot> = self
                .producers
                .iter()
                .filter_map(|(id, producer)| producer.current().map(|slot| (*id, slot)))
                .collect();
            let producers = &currents;
            let mut results: Vec<Result<Lowered, RenderError>> = pending
                .iter_mut()
                .zip(frames)
                .zip(group_maps.iter().zip(inputs).zip(self.ready_sets.iter()))
                .map(|((surf, frame), ((groups, inputs), ready))| {
                    Self::lower_content(
                        surf,
                        frame,
                        GlyphResources {
                            atlas: &self.atlas,
                            fonts: &self.fonts,
                            images: &self.images,
                            bitmaps: &self.bitmaps,
                            producers,
                        },
                        groups,
                        inputs,
                        ready,
                    )
                })
                .collect();
            // The batch's pending rasters commit transactionally
            // (#169 A3): a dry run decides fit / grow once / recycle
            // before any placement or upload happens, so a failed
            // placement never enqueues uploads into an atlas the same
            // preparation abandons.
            match self.commit_rasters(pending, &mut results, grew) {
                Commit::Done => break results,
                Commit::Resize(size, pages) => {
                    self.atlas.resize_to(&self.device, size, pages);
                    grew = true;
                    tracing::debug!(
                        size = self.atlas.size(),
                        pages = self.atlas.pages(),
                        generation = self.atlas.generation(),
                        "atlas resized"
                    );
                }
            }
            // Growing emptied the atlas: every hit any lowering took is
            // now a miss, so lower the whole batch again.
            diag::event(
                &self.device,
                diag::EventKind::Phase {
                    name: "atlas retry",
                },
            );
        }
    }

    // `#[inline(never)]` keeps a symbol for the Callgrind gate's root lookup
    // (`bench/scripts/ir_gate.py`).
    /// Whether a surface needs a render for reasons outside the frame's
    /// commits: an active filter, a producer's new content, a stale
    /// projective image, or an animated shader texture.
    fn wants_redraw(&self, surface: &SurfaceState) -> bool {
        surface
            .frame
            .filters
            .iter()
            .any(|(_, id)| self.filters.wants_redraw(*id))
            || surface.content_wants_redraw(&self.producers)
            || surface.projective_wants_redraw(&self.filters, &self.shaders, &self.producers)
            || surface
                .shader_textures
                .keys()
                .any(|key| self.shaders.animated(key))
    }

    /// Observe content lifetimes without outlining glyphs until admission.
    /// Periodic changes must outlive their previous quiet interval before
    /// another capture is admitted.
    fn observe_static(&mut self, sf: &SurfaceFrame<'_>) -> Result<(), RenderError> {
        let surf = self.surfaces.get_mut(&sf.id).expect("registered surface");
        if !surf.promotes {
            return Ok(());
        }
        let output_changed = sf.display.headroom.to_bits() != surf.display.headroom.to_bits()
            || sf.display.scale.to_bits() != surf.display.scale.to_bits();
        if output_changed && !surf.static_layers.is_empty() {
            for entry in surf.static_layers.values_mut() {
                entry.capture = None;
            }
            surf.plan_dirty = true;
        }
        if !sf.changed && !output_changed {
            return Ok(());
        }
        let resources = (self.images_gen, self.image_replacements);
        surf.static_layers
            .retain(|layer, _| surf.layers.contains_key(layer));
        surf.static_walk.clear();
        surf.static_walk
            .push((sf.tree.root(), kurbo::Affine::IDENTITY));
        while let Some((layer, parent)) = surf.static_walk.pop() {
            let node = sf.tree.layer(layer);
            if sf.tree.projective_pose(layer).is_some() {
                continue;
            }
            let space = parent * node.content_transform();
            surf.static_walk
                .extend(node.children.iter().map(|&child| (child, space)));
            if !node.children.is_empty() {
                surf.static_layers.remove(&layer);
                continue;
            }
            let Some(content) = surf.layers.get(&layer) else {
                continue;
            };
            let stamp = sf.tree.content_stamp(layer);
            // The transformed unit circle's major radius is the largest
            // singular value. Capture at the actual device density.
            let radii = kurbo::Ellipse::from_affine(space).radii();
            let density = radii.x.max(radii.y);
            // A replacement is unprepared until this frame lowers it. The
            // lifetime still has to be recorded: deleting the observation
            // admits the following interval from the initial threshold.
            let Some((ops, source)) = content.retained.current() else {
                let forget = if let Some(entry) = surf.static_layers.get_mut(&layer) {
                    if entry.stamp != stamp || entry.resources != resources {
                        entry.change(stamp, resources);
                        entry.density = density;
                        false
                    } else {
                        true
                    }
                } else {
                    false
                };
                if forget {
                    surf.static_layers.remove(&layer);
                }
                continue;
            };
            let entry = surf
                .static_layers
                .entry(layer)
                .or_insert_with(|| planes::static_layer::Observation::new(stamp, resources));
            if entry.observe(stamp, resources, density) {
                entry.domain = planes::static_layer::domain(
                    ops,
                    source,
                    &self.fonts,
                    density,
                    self.max_texture,
                )?;
            }
        }
        Ok(())
    }

    /// Admit external-frame or promoted-pose updates only when the native
    /// stack and all engine-composited content stay unchanged.
    fn plane_only_frames(&mut self, sf: &SurfaceFrame<'_>) -> bool {
        if sf.present_pending || sf.display_moved {
            return false;
        }
        let Some(surface) = self.surfaces.get(&sf.id) else {
            return false;
        };
        if surface.plan_dirty {
            return false;
        }
        if !surface.promotes || surface.present_pending || self.wants_redraw(surface) {
            return false;
        }
        let Some(system) = self.planes.get_mut(&sf.id) else {
            return false;
        };
        // The committed plan judged only the candidates the platform
        // reported ready; the check must see the same offered set — a
        // pending candidate is no plan change, a newly ready one is.
        let currents: FxHashMap<ProducerId, &external::Slot> = self
            .producers
            .iter()
            .filter_map(|(id, producer)| producer.current().map(|slot| (*id, slot)))
            .collect();
        let candidates = plane_candidates(surface, &currents, &mut self.candidates);
        let installed = plane_frames(surface, &currents, &mut self.candidate_frames);
        planes::SystemPlanes::prepare(system, candidates, installed, &mut self.ready);
        if let Some(frames) = sf.plane_frames {
            return planes::frames_only::<planes::Platform>(
                &surface.plan,
                sf.tree,
                &self.candidates,
                &self.ready,
                frames,
                &mut self.plan_scratch,
            );
        }
        if surface.plan.planes.is_empty()
            || (surface.plane_clear, surface.plane_size) != (sf.clear, sf.size)
            || surface.display.scale.to_bits() != sf.display.scale.to_bits()
            || surface.plan.planes.iter().any(|plane| {
                !self.candidates.contains_key(&plane.layer)
                    || !sf.tree.layer(plane.layer).children.is_empty()
            })
            || surface.plane_resources
                != (self.images_gen, self.image_replacements, surface.interop)
            || sf.tree.composition_stamp(|layer| {
                surface.plan.planes.iter().any(|plane| plane.layer == layer)
            }) != surface.plane_stamp
        {
            return false;
        }
        planes::plan_with::<planes::Platform>(
            sf.tree,
            &self.candidates,
            &self.ready,
            &mut self.plan_scratch,
            &mut self.next_plan,
        );
        let next = &self.next_plan;
        if next.trailing != surface.plan.trailing
            || next.planes.len() != surface.plan.planes.len()
            || !next.planes.iter().zip(&surface.plan.planes).all(|(a, b)| {
                (a.layer, a.size, a.raster, a.source) == (b.layer, b.size, b.raster, b.source)
            })
        {
            return false;
        }
        std::mem::swap(
            &mut self
                .surfaces
                .get_mut(&sf.id)
                .expect("registered surface")
                .plan,
            &mut self.next_plan,
        );
        true
    }

    fn include_ready_planes<'a>(
        &mut self,
        frame: &Frame<'a>,
        dirty: &mut Vec<&'a SurfaceFrame<'a>>,
    ) {
        for sf in frame.surfaces {
            if dirty.iter().any(|seen| seen.id == sf.id) {
                continue;
            }
            let Some(surface) = self.surfaces.get(&sf.id) else {
                continue;
            };
            if !surface.promotes {
                // A hosted layer the surface could not place keeps failing
                // its renders until it is gone.
                if surface.plan_dirty {
                    dirty.push(sf);
                }
                continue;
            }
            let Some(system) = self.planes.get_mut(&sf.id) else {
                continue;
            };
            let currents: FxHashMap<ProducerId, &external::Slot> = self
                .producers
                .iter()
                .filter_map(|(id, producer)| producer.current().map(|slot| (*id, slot)))
                .collect();
            let candidates = plane_candidates(surface, &currents, &mut self.candidates);
            let frames = plane_frames(surface, &currents, &mut self.candidate_frames);
            planes::SystemPlanes::groom_with_frames(system, candidates, frames);
            if surface.plan_dirty || planes::SystemPlanes::wants_plan(system) {
                dirty.push(sf);
            }
        }
    }

    /// Per-surface the candidates whose plane can show a frame now,
    /// filled into `self.ready_sets` (kept between renders, so the sets
    /// allocate nothing steady-state): `SystemPlanes::prepare` queues
    /// each new candidate's realization and reports it ready once done;
    /// a candidate it does not report keeps compositing in-engine this
    /// frame.
    fn ready_planes(&mut self, frames: &[&SurfaceFrame<'_>], pending: &[SurfaceState]) {
        let currents: FxHashMap<ProducerId, &external::Slot> = self
            .producers
            .iter()
            .filter_map(|(id, producer)| producer.current().map(|slot| (*id, slot)))
            .collect();
        let (ready_sets, planes, candidates, candidate_frames) = (
            &mut self.ready_sets,
            &mut self.planes,
            &mut self.candidates,
            &mut self.candidate_frames,
        );
        let producers = &currents;
        for (i, (sf, surf)) in frames.iter().zip(pending.iter()).enumerate() {
            if ready_sets.len() == i {
                ready_sets.push(FxHashSet::default());
            }
            let candidates = plane_candidates(surf, producers, candidates);
            let frames = plane_frames(surf, producers, candidate_frames);
            if let Some(system) = planes.get_mut(&sf.id) {
                planes::SystemPlanes::prepare(system, candidates, frames, &mut ready_sets[i]);
            } else {
                let ready = &mut ready_sets[i];
                ready.clear();
                ready.extend(candidates.keys().copied());
            }
        }
    }

    /// The first hosted layer `surf` cannot place this frame, with the
    /// rule it fails. Content shown only on a plane is never composited
    /// instead: such a layer fails the render, and every render after it
    /// until the layer is placeable or gone.
    fn unplaced(surf: &SurfaceState, tree: &cherenkov::SurfaceTree) -> Option<(LayerId, String)> {
        if surf.promotes {
            surf.plan
                .unplaced
                .first()
                .map(|(layer, cause)| (*layer, cause.to_string()))
        } else if surf.hosted.is_empty() {
            None
        } else {
            planes::first_in_paint_order(tree, |layer| surf.hosted.contains_key(&layer)).map(
                |layer| {
                    (
                        layer,
                        "the surface has no system-compositor parent".to_owned(),
                    )
                },
            )
        }
    }

    #[inline(never)]
    fn lower_content(
        surf: &mut SurfaceState,
        frame: &SurfaceFrame<'_>,
        resources: GlyphResources<'_>,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        inputs: &projective::Inputs,
        ready: &FxHashSet<LayerId>,
    ) -> Result<Lowered, RenderError> {
        surf.frame.reset();
        surf.plan_dirty = false;
        let mut candidates = FxHashMap::default();
        surf.plan = if surf.promotes {
            plane_candidates(surf, resources.producers, &mut candidates);
            planes::plan::<planes::Platform>(frame.tree, &candidates, ready)
        } else {
            planes::Plan::default()
        };
        if let Some((layer, reason)) = Self::unplaced(surf, frame.tree) {
            surf.plan_dirty = true;
            return Err(RenderError::Unplaceable { layer, reason });
        }
        if surf.promotes {
            for plane in &surf.plan.planes {
                tracing::debug!(target: "cherenkov::planes", layer = ?plane.layer, decision = "promoted", "plane decision");
            }
            for (layer, why) in &surf.plan.rejected {
                tracing::debug!(target: "cherenkov::planes", layer = ?layer, decision = ?why, "plane decision");
            }
        }
        surf.plane_stamp = frame
            .tree
            .composition_stamp(|layer| surf.plan.planes.iter().any(|plane| plane.layer == layer));
        surf.plane_clear = frame.clear;
        surf.plane_size = frame.size;
        // Lowering borrows `layers` immutably while mutating `frame`;
        // taking the map out keeps the two borrows disjoint.
        let mut layers = std::mem::take(&mut surf.layers);
        let mut lowered = Lowered::default();
        let result = {
            let glyphs = GlyphContext {
                atlas: resources.atlas,
                live_stamp: resources.atlas.live_stamp(),
                fonts: resources.fonts,
                images: resources.images,
                bitmaps: resources.bitmaps,
                content: &surf.bindings,
            };
            let mut lowering = Lowering::new(&mut surf.frame, surf.size);
            let result = lowering.prepare(&mut layers, &glyphs).and_then(|()| {
                for placement in &surf.plan.planes {
                    if let Some(entry) = surf.static_layers.get(&placement.layer)
                        && entry.capture.as_ref().is_none_or(|capture| capture.dirty)
                    {
                        lowering.run_plane(
                            frame.tree,
                            &mut layers,
                            &glyphs,
                            groups,
                            placement.layer,
                            entry.domain.expect("a static candidate has a domain"),
                        )?;
                    }
                }
                let placed = Self::lower_projective(
                    &mut lowering,
                    frame,
                    &mut layers,
                    (&glyphs, groups),
                    (&surf.projective, inputs, surf.size),
                    &mut lowered,
                )?;
                lowering.run(
                    frame.tree,
                    &mut layers,
                    frame.clear,
                    &glyphs,
                    groups,
                    placed,
                    &surf.plan,
                )
            });
            lowered.commands = lowering.commands_lowered;
            lowered.layers = lowering.layers_composed;
            lowered.glyphs = lowering.glyphs_rasterized();
            lowered.paths = lowering.paths_rasterized();
            lowered.cell_patches = std::mem::take(&mut lowering.cell_patches);
            lowered.emission_patches = std::mem::take(&mut lowering.emission_patches);
            lowered.mask_patches = std::mem::take(&mut lowering.mask_patches);
            lowered.pending = std::mem::take(&mut lowering.pending);
            result
        };
        surf.layers = layers;
        surf.frame.content.sort_unstable_by_key(|(id, _)| id.raw());
        surf.frame.external.sort_unstable_by_key(|id| id.raw());
        surf.frame.external.dedup();
        result.map(|()| lowered)
    }

    /// Plans the surface's projective layers and lowers the local image of
    /// every visible one without a current retained image, innermost
    /// first, into the frame ahead of the surface walk. Returns the
    /// placements composed directly into the surface; `lowered` records
    /// the images realized and composed.
    fn lower_projective(
        lowering: &mut Lowering<'_>,
        frame: &SurfaceFrame<'_>,
        layers: &mut FxHashMap<LayerId, ContentData>,
        (glyphs, groups): (&GlyphContext<'_>, &FxHashMap<u64, BackdropGroupInfo>),
        (retained, inputs, size): (
            &FxHashMap<LayerId, Vec<projective::Entry>>,
            &projective::Inputs,
            (u32, u32),
        ),
        lowered: &mut Lowered,
    ) -> Result<FxHashMap<LayerId, projective::Placement>, RenderError> {
        let tree = frame.tree;
        let mut placed: FxHashMap<Option<LayerId>, FxHashMap<LayerId, projective::Placement>> =
            FxHashMap::default();
        for plan in cherenkov::lowering::projective::plan(tree, size, inputs.limits)? {
            let Some(image) = plan.image else { continue };
            let nested = placed.remove(&Some(plan.layer)).unwrap_or_default();
            let key = projective::LocalKey {
                layer: plan.layer,
                bucket: projective::bucket(image.density),
            };
            let cache =
                projective::Key::new(&image, tree.content_stamp(plan.layer), inputs.replacements);
            let current = !inputs.stale.contains(&key)
                && retained.get(&plan.layer).is_some_and(|entries| {
                    entries
                        .iter()
                        .any(|e| e.bucket == key.bucket && e.key == Some(cache))
                });
            if !current {
                let (filters, passes) = (
                    lowering.frame().filters.len(),
                    lowering.frame().passes.len(),
                );
                lowering.run_local(
                    tree,
                    layers,
                    glyphs,
                    groups,
                    (plan.layer, image.local_to_texel, key),
                    image.size,
                    nested,
                )?;
                let lowered_frame = lowering.frame();
                let last = lowered_frame.passes.len() - 1;
                let deps = projective::Deps::of(lowered_frame, filters, passes, inputs.interop);
                lowering.push_mips(last, key);
                lowered.realize.push(projective::Realize {
                    key,
                    cache,
                    levels: image.levels.clone(),
                    deps,
                });
            }
            lowered.composed.push(key);
            placed
                .entry(plan.parent)
                .or_default()
                .insert(plan.layer, projective::Placement::new(key, &image));
        }
        Ok(placed.remove(&None).unwrap_or_default())
    }

    /// Over budget, drops every mask texture no retained frame references:
    /// bind groups and frames holding one keep it alive by key.
    fn evict_dead_mask_textures(&mut self) {
        if !self.atlas.mask_textures_over_budget() {
            return;
        }
        let live: FxHashSet<u64> = self
            .surfaces
            .values()
            .flat_map(|surf| surf.frame.passes.iter())
            .flat_map(|pass| pass.ranges.iter())
            .filter_map(|range| range.mask)
            .collect();
        let gen_before = self.atlas.mask_texture_generation();
        self.atlas
            .evict_mask_textures(&self.device, |key| live.contains(&key));
        if self.atlas.mask_texture_generation() == gen_before {
            return;
        }
        // #169 A4: unsubmitted bind groups holding an evicted view are
        // obsolete — drop them now rather than at the next encode.
        let mask_gen = self.atlas.mask_texture_generation();
        let images_gen = self.images_gen;
        for surf in self.surfaces.values_mut() {
            retire_binds1(
                surf,
                &self.device,
                images_gen,
                mask_gen,
                "mask texture evict",
                |key| key.3.is_some_and(|mask| !live.contains(&mask)),
            );
        }
    }

    /// Commits every surface's pending rasters transactionally
    /// (#169 A3). The shelves this lowering's hits live on are marked
    /// first so no placement can evict them; [`Atlas::plan`] then
    /// dry-runs all placements against shelf metadata alone; only a
    /// `Fits` verdict — or the last attempt after a grow — commits for
    /// real. When even an emptied atlas cannot hold the batch the
    /// commit runs evicting instead of clearing: cold shelves are
    /// reclaimed in place, so surviving entries — and the retained
    /// emissions referencing them — stay valid (#119). The upload
    /// batches the committed cells into one `write_texture` per newly
    /// allocated shelf region.
    // `never` so Callgrind can attribute the commit path's inclusive
    // cost — the one-per-frame call boundary is free evidence (#119).
    #[inline(never)]
    fn commit_rasters(
        &mut self,
        pending: &mut [SurfaceState],
        results: &mut [Result<Lowered, RenderError>],
        grew: bool,
    ) -> Commit {
        let rasters: Vec<&glyph::PendingRaster> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .flat_map(|l| l.pending.iter())
            .collect();
        let n_cells: usize = rasters.iter().map(|r| r.cell_count()).sum();
        // A frame whose batch places strictly needs no replay pins and
        // no eviction, so no emission scan and no touch decoding runs —
        // the commit's work scales with what changed, not what stays
        // retained (#119).
        let mut touches_len = 0usize;
        // `plan` also decides whether a paged atlas may release pages
        // (#211), so only a single-page atlas takes the strict path.
        let plan_dbg = if self.atlas.pages() == 1 && self.atlas.fits_strict(&rasters) {
            self.atlas.begin_commit(&[]);
            "fits"
        } else {
            let mut touches = std::mem::take(&mut self.commit_touches);
            touches.clear();
            touches_len = self.replay_pins(pending, &mut touches);
            self.atlas.begin_commit(&touches);
            touches.clear();
            self.commit_touches = touches;
            match self.atlas.plan(&rasters) {
                glyph::AtlasPlan::Fits | glyph::AtlasPlan::FitsEviction => "fits-eviction",
                glyph::AtlasPlan::Resize(size, pages) if !grew => {
                    // The resize discards the atlas and re-lowers, so
                    // every retained emission is dead anyway — but the
                    // pending cells recorded this round index a raster
                    // list that never applied, and must not survive
                    // into the retry's hits (#119).
                    for surf in pending.iter_mut() {
                        Self::discard_surface(surf);
                    }
                    return Commit::Resize(size, pages);
                }
                glyph::AtlasPlan::Resize(..) | glyph::AtlasPlan::Recycle => {
                    // Bounded in-place eviction makes room instead of a
                    // wholesale clear: the commit below reclaims shelves
                    // nothing touched until the batch places or nothing
                    // untouchable remains, then exhausts the first surface
                    // whose raster still does not fit (#119).
                    "exhaust-candidate"
                }
            }
        };
        if plan_dbg != "fits" {
            self.atlas.enable_evicting();
        }
        let mut writes = std::mem::take(&mut self.commit_writes);
        writes.clear();
        let mut failed = None;
        for (i, (surf, result)) in pending.iter_mut().zip(results.iter_mut()).enumerate() {
            match result {
                // A failed lowering leaves whatever retained mutations
                // its leaves already made — pending cells indexing a
                // raster list that never applies. Those emissions must
                // not survive to hit next frame (#119).
                Err(_) => Self::discard_surface(surf),
                Ok(lowered) => match self.apply_pending(surf, lowered, &mut writes) {
                    Ok(()) => {}
                    Err(e) => {
                        *result = Err(match e {
                            RenderError::AtlasFull => RenderError::AtlasExhausted,
                            other => other,
                        });
                        failed = Some(i);
                        break;
                    }
                },
            }
        }
        // An apply that stopped midway leaves the pending cells of
        // every unapplied surface pointing at rasters that never
        // landed — drop their retained emissions wholesale so the
        // next frame re-lowers rather than composes unplaced cells
        // (#119).
        if let Some(i) = failed {
            Self::discard_unapplied(&mut pending[i..], &mut results[i..]);
        }
        let evicted = self.atlas.take_evicted();
        tracing::debug!(
            evictions = evicted.len(),
            evicted_bytes = evicted.iter().map(|e| e.0).sum::<u64>(),
            plan = ?plan_dbg,
            pending_cells = n_cells,
            touches = touches_len,
            occupancy = ?self.atlas.occupancy(),
            "atlas commit"
        );
        for (bytes, used_in_latest_submit) in evicted {
            diag::retire(
                &self.device,
                diag::RetireArgs {
                    label: "glyph atlas",
                    class: diag::Class::Atlas,
                    bytes,
                    used_in_latest_submit,
                    reason: "atlas evict",
                },
            );
        }
        self.atlas
            .upload_committed(&self.device, &self.queue, &writes);
        self.commit_writes = writes;
        Commit::Done
    }

    /// The shelf slots the commit must pin: the shelves each new
    /// emission's glyph instances sample plus the clip masks bound
    /// through `uv[2..3]` — all recovered from stored UVs, so the
    /// lowering records no slot list (#119).
    fn replay_pins(&self, pending: &mut [SurfaceState], touches: &mut Vec<u32>) -> usize {
        for surf in pending.iter_mut() {
            for content in surf.layers.values_mut() {
                let (_, emissions) = content.retained.prepared();
                for emission in emissions.iter().filter_map(|e| e.data.as_ref()) {
                    Self::for_each_glyph_shelf(
                        &self.atlas,
                        &content.storage.instances[emission.instances.clone()],
                        |slot| touches.push(slot),
                    );
                }
            }
            // Clip masks bound through `uv[2..3]` pin the same way: the
            // frame's masked instances recover their shelf from the
            // stored atlas coordinates (#119).
            for inst in &surf.frame.instances {
                let flags = inst.meta[3] >> 24;
                if flags & instance::FLAG_HAS_MASK != 0
                    && flags & instance::FLAG_MASK_TEXTURE == 0
                    && let Some(slot) = self.atlas.shelf_at(inst.uv[2], inst.uv[3])
                {
                    touches.push(slot);
                }
            }
        }
        // Pins stay frame-scoped: only the shelves this frame's
        // emissions and masks sample are marked. Bands a retained
        // emission sampled in an earlier frame are deliberately left
        // unpinned: evicting them is what keeps the atlas bounded, and
        // the emission's stale check re-lowers it if it displays again
        // (#119).
        touches.len()
    }

    /// The shelves `instances`' glyph quads sample, recovered from their
    /// stored UVs — the leaf records no per-cell slot at emit time, so
    /// the commit derives them here instead (#119).
    fn for_each_glyph_shelf(
        atlas: &Atlas,
        instances: &[lower::RetainedInstance],
        mut f: impl FnMut(u32),
    ) {
        let mut hint = None;
        for inst in instances {
            if matches!(inst.kind, instance::KIND_GLYPH | instance::KIND_REGION)
                && let Some(slot) = atlas.shelf_at_hint(inst.uv[0], inst.uv[1], &mut hint)
            {
                f(slot);
            }
        }
    }

    /// Drops one surface's retained emissions: unapplied pending cells
    /// index a raster list that never landed, so nothing they recorded
    /// may hit next frame (#119).
    fn discard_surface(surf: &mut SurfaceState) {
        for content in surf.layers.values_mut() {
            content.invalidate();
        }
    }

    /// [`Self::discard_surface`] for every surface at `from` onward and
    /// marks the still-`Ok` results [`RenderError::AtlasExhausted`]
    /// (#119).
    fn discard_unapplied(
        pending: &mut [SurfaceState],
        results: &mut [Result<Lowered, RenderError>],
    ) {
        for (surf, result) in pending.iter_mut().zip(results.iter_mut()) {
            Self::discard_surface(surf);
            if result.is_ok() {
                *result = Err(RenderError::AtlasExhausted);
            }
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "atlas coordinates fit exactly in f32"
    )]
    fn apply_pending(
        &mut self,
        surf: &mut SurfaceState,
        lowered: &mut Lowered,
        writes: &mut Vec<glyph::CellWrite>,
    ) -> Result<(), RenderError> {
        let pending = std::mem::take(&mut lowered.pending);
        let mut origins = std::mem::take(&mut self.pending_origins);
        origins.clear();
        for raster in pending {
            origins.push(self.apply_raster(raster, writes)?);
        }
        let cell_origin = |p: u32, c: u32| {
            let PendingOrigin::Cells(cells, _) = &origins[p as usize] else {
                unreachable!("cell patch must reference cell raster");
            };
            let (x, y) = cells[c as usize];
            [x as f32, y as f32]
        };
        for content in surf.layers.values_mut() {
            let (_, emissions) = content.retained.prepared();
            for emission in emissions.iter_mut().filter_map(|e| e.data.as_mut()) {
                // The emission's atlas references are the shelves it
                // touched while lowering plus the bands its deferred
                // rasters resolved to — recorded as `(slot, band epoch)`
                // pairs contiguous at the storage tail. Duplicates are
                // harmless (epoch checks are idempotent and pin marks
                // dedupe themselves), so the fold writes directly — no
                // per-emission set allocation (#119).
                // Deferred `refs` resolve here: the frame's touched
                // slots and each placed cell's bands join as one
                // contiguous `(slot, epoch)` extent at the storage
                // tail (#119).
                if emission.refs & lower::DEFERRED_REFS != 0 || !emission.pending_cells_empty() {
                    let first = content.storage.refs.len();
                    for i in emission.pending_cells() {
                        let (inst, p, c) = lowered.emission_patches[i];
                        let local = inst - emission.cell_inst_base();
                        content.storage.instances[emission.instances.start + local as usize].uv
                            [..2]
                            .copy_from_slice(&cell_origin(p, c));
                        if let PendingOrigin::Cells(_, bands) = &origins[p as usize] {
                            content.storage.refs.extend(
                                bands
                                    .iter()
                                    .map(|&slot| (slot, self.atlas.shelf_epoch(slot))),
                            );
                        }
                    }
                    // Glyph cells record no slot at emit time: the shelf
                    // each stored UV samples is recovered here instead
                    // (#119).
                    Self::for_each_glyph_shelf(
                        &self.atlas,
                        &content.storage.instances[emission.instances.clone()],
                        |slot| {
                            content
                                .storage
                                .refs
                                .push((slot, self.atlas.shelf_epoch(slot)));
                        },
                    );
                    emission.clear_pending_cells();
                    emission.refs =
                        lower::Emission::pack_refs(first, content.storage.refs.len() - first);
                }
                // Restamp only when every reference survived this
                // commit's evictions; a stale emission must keep an
                // older stamp so its next hit check walks the refs
                // and re-lowers (#119).
                let live = content.storage.refs[emission.refs_range()]
                    .iter()
                    .all(|&(s, ep)| self.atlas.shelf_epoch(s) == ep);
                if live {
                    emission.live_stamp = self.atlas.live_stamp();
                }
            }
        }
        for (inst, p, c) in lowered.cell_patches.drain(..) {
            let [x, y] = cell_origin(p, c);
            surf.frame.instances[inst as usize].uv[..2].copy_from_slice(&[x, y]);
        }
        for (inst, p) in lowered.mask_patches.drain(..) {
            let PendingOrigin::Mask(origin) = &origins[p as usize] else {
                unreachable!("mask patch must reference mask raster");
            };
            surf.frame.instances[inst as usize].uv[2..].copy_from_slice(origin);
        }
        self.pending_origins = origins;
        Ok(())
    }

    fn apply_raster(
        &mut self,
        raster: PendingRaster,
        writes: &mut Vec<glyph::CellWrite>,
    ) -> Result<PendingOrigin, RenderError> {
        match raster {
            PendingRaster::Glyph {
                key,
                left,
                top,
                w,
                h,
                texels,
            } => {
                let hit = self.atlas.get(&key).is_some();
                let out = self
                    .atlas
                    .place_glyph(key, left, top, w, h, texels, writes)
                    .map(|(x, y)| {
                        PendingOrigin::Cells(
                            vec![(x, y)],
                            vec![self.atlas.get(&key).expect("just stored").slot],
                        )
                    })
                    .ok_or(RenderError::AtlasFull)?;
                if !hit && w == 0 {
                    diag::atlas_cell(&self.device, (0, 0, 0, 0));
                }
                Ok(out)
            }
            PendingRaster::Path { key, emit, cells } => {
                self.atlas
                    .place_path(key, emit, cells, writes)
                    .ok_or(RenderError::AtlasFull)?;
                Ok(PendingOrigin::Cells(
                    self.atlas.path_origins(key).expect("just stored"),
                    self.atlas
                        .emit_slot_arena(self.atlas.path(key).expect("just stored").slots.clone())
                        .to_vec(),
                ))
            }
            PendingRaster::Mask {
                key,
                mask,
                w,
                h,
                texels,
            } => {
                self.atlas
                    .place_mask(key, mask, w, h, texels, writes)
                    .ok_or(RenderError::AtlasFull)?;
                Ok(PendingOrigin::Mask(
                    self.atlas.mask_origin(key).expect("just stored"),
                ))
            }
            PendingRaster::MaskTexture {
                key,
                mask,
                w,
                h,
                texels,
            } => {
                self.atlas
                    .store_mask_texture(&self.device, &self.queue, key, mask, w, h, &texels);
                Ok(PendingOrigin::None)
            }
            PendingRaster::Colr { font, key, picture } => {
                if let Some(font) = self.fonts.get_mut(&font) {
                    font.colr.borrow_mut().entry(key).or_insert(picture);
                }
                Ok(PendingOrigin::None)
            }
            PendingRaster::Bitmap {
                key,
                em,
                width,
                height,
                texels,
            } => {
                if !self.bitmaps.contains_key(&key) {
                    let image = create_gpu_image(
                        &self.device,
                        &self.queue,
                        "bitmap glyph",
                        width,
                        height,
                        &texels,
                    );
                    self.bitmaps.insert(key, GpuBitmap { image, em });
                }
                Ok(PendingOrigin::None)
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one surface's frame application and buffer growth"
    )]
    fn lower_surface(
        &mut self,
        id: SurfaceId,
        stats: &mut FrameStats,
        inst_base: u32,
        stop_base: u32,
        globals_base: u32,
        lowered: Result<Lowered, RenderError>,
    ) -> Result<(), RenderError> {
        diag::set_surface(Some(id.raw()));
        let Lowered {
            glyphs,
            paths,
            commands,
            layers,
            realize,
            composed,
            ..
        } = lowered?;
        stats.commands_lowered += commands;
        stats.layers_composed += layers;
        stats.glyphs_rasterized += glyphs;
        stats.paths_rasterized += paths;
        stats.projective_realized += u32::try_from(realize.len()).unwrap_or(u32::MAX);
        stats.projective_composed += u32::try_from(composed.len()).unwrap_or(u32::MAX);
        self.allocate_projective(id, realize, composed)?;
        // Scratch textures for the frame's deepest isolation level.
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        for (&layer, entry) in &mut surf.static_layers {
            if !surf.plan.planes.iter().any(|plane| plane.layer == layer) {
                entry.capture = None;
                continue;
            }
            if entry.capture.is_none() {
                let domain = entry.domain.expect("a static candidate has a domain");
                let (texture, view) = create_target(
                    &self.device,
                    "static plane capture",
                    domain.size,
                    TARGET_USAGES,
                    TARGET_FORMAT,
                );
                entry.capture = Some(planes::static_layer::Capture {
                    domain,
                    source: Some((texture, view)),
                    generation: self.frame_count,
                    dirty: true,
                });
            }
        }
        // One engine texture per part above the first, as the plan split
        // the surface.
        let parts = surf.plan.parts() - 1;
        let part_bytes =
            u64::from(surf.size.0) * u64::from(surf.size.1) * texel_bytes(TARGET_FORMAT);
        while surf.parts.len() < parts {
            surf.parts.push(create_target(
                &self.device,
                "engine part",
                surf.size,
                TARGET_USAGES,
                TARGET_FORMAT,
            ));
            diag::create(&self.device, "engine part", part_bytes);
        }
        if surf.parts.len() > parts {
            diag::retire(
                &self.device,
                diag::RetireArgs {
                    label: "engine part",
                    class: diag::Class::Target,
                    bytes: part_bytes * (surf.parts.len() - parts) as u64,
                    used_in_latest_submit: true,
                    reason: "plan",
                },
            );
            // A cached direct-resolve bind on a retired part would keep
            // its texture alive past the retire above.
            for (_, view) in surf.parts.split_off(parts) {
                surf.drop_resolve_binds_on(&self.device, &view, "backdrop resolve");
            }
        }
        #[cfg(target_vendor = "apple")]
        {
            let tile_slots = if self.shader_delivery == shaders::ShaderDelivery::Metallib
                && self.scratch_format == TARGET_FORMAT
            {
                2
            } else {
                0
            };
            surf.composition_cache.update(
                &mut surf.composition,
                &surf.frame,
                self.scratch_format,
                tile_slots,
            );
        }
        let max_scratch = surf
            .frame
            .passes
            .iter()
            .filter_map(|p| match p.target {
                Target::Scratch(i) => Some(i + 1),
                Target::Part(_)
                | Target::Backdrop { .. }
                | Target::Projected(_)
                | Target::Plane(_) => None,
            })
            .max()
            .unwrap_or(0);
        // The largest region each isolation depth must hold this frame.
        let mut region_max = vec![(0u32, 0u32); max_scratch];
        let passes = surf.frame.passes.iter();
        #[cfg(target_vendor = "apple")]
        let passes = passes
            .enumerate()
            .filter_map(|(index, pass)| surf.composition.materialized_pass(index).then_some(pass));
        for pass in passes {
            if let Target::Scratch(i) = pass.target {
                region_max[i].0 = region_max[i].0.max(pass.region[2]);
                region_max[i].1 = region_max[i].1.max(pass.region[3]);
            }
        }
        // Grow each scratch to its needed size; never shrink.
        let mut retired_depths = Vec::new();
        surf.scratch.retain(|depth, texture| {
            let keep = region_max.get(*depth).is_some_and(|size| *size != (0, 0));
            if !keep {
                diag::retire(
                    &self.device,
                    diag::RetireArgs {
                        label: "isolation scratch",
                        class: diag::Class::Target,
                        bytes: u64::from(texture.width)
                            * u64::from(texture.height)
                            * texel_bytes(texture.texture.format()),
                        used_in_latest_submit: true,
                        reason: "tile lifetime",
                    },
                );
                retired_depths.push(*depth);
            }
            keep
        });
        if !retired_depths.is_empty() {
            surf.bind_gen += 1;
            retire_binds1(
                surf,
                &self.device,
                self.images_gen,
                self.atlas.mask_texture_generation(),
                "tile lifetime",
                |key| matches!(key.0, Some(Source::Scratch(i)) if retired_depths.contains(&i)),
            );
        }
        for (i, &(w, h)) in region_max.iter().enumerate() {
            if w == 0 || h == 0 {
                continue;
            }
            let (nw, nh) = (
                w.max(surf.scratch.get(&i).map_or(0, |s| s.width)),
                h.max(surf.scratch.get(&i).map_or(0, |s| s.height)),
            );
            if surf
                .scratch
                .get(&i)
                .is_some_and(|s| s.width >= w && s.height >= h)
            {
                continue;
            }
            if nw > self.device.limits().max_texture_dimension_2d
                || nh > self.device.limits().max_texture_dimension_2d
            {
                return Err(RenderError::Render(
                    "isolation capture exceeds device texture extent".into(),
                ));
            }
            let old_scratch = surf.scratch.get(&i).map_or(0, |s| {
                u64::from(s.width) * u64::from(s.height) * texel_bytes(s.texture.format())
            });
            let (texture, view) = create_target(
                &self.device,
                "isolation scratch",
                (nw, nh),
                TARGET_USAGES,
                self.scratch_format,
            );
            let target = ScratchTarget {
                texture,
                view,
                width: nw,
                height: nh,
            };
            diag::grow(
                &self.device,
                "isolation scratch",
                diag::Class::Target,
                old_scratch,
                u64::from(nw) * u64::from(nh) * texel_bytes(self.scratch_format),
                0,
                true,
            );
            let replaced_view = surf.scratch.get(&i).map(|s| s.view.clone());
            surf.scratch.insert(i, target);
            surf.bind_gen += 1;
            // #169 A4: unsubmitted group-1 bind groups referencing the
            // replaced view keep its predecessor alive — drop them now.
            retire_binds1(
                surf,
                &self.device,
                self.images_gen,
                self.atlas.mask_texture_generation(),
                "scratch regen",
                |key| key.0 == Some(Source::Scratch(i)),
            );
            // A cached direct-resolve bind holds its source view the
            // same way: drop every group's entry that reads the
            // replaced scratch's view so its texture frees with it.
            if let Some(replaced_view) = replaced_view {
                surf.drop_resolve_binds_on(&self.device, &replaced_view, "backdrop resolve");
            }
        }
        // Backdrop-group captures are exactly their pass's region, in the
        // format of the target the capture copies from, and sampled by
        // later passes. The shared staging needs one allocation per
        // format, so the loop also accumulates the largest staged
        // device rect of each.
        let mut staged_max = [None, None];
        for pass in &surf.frame.passes {
            let Some(capture) = pass.capture else {
                continue;
            };
            let (w, h) = (pass.region[2], pass.region[3]);
            let format = capture_format(capture.copy_from, self.scratch_format)?;
            let Some(group_state) = surf.backdrop_groups.get_mut(&capture.group) else {
                return Err(RenderError::Render(format!(
                    "backdrop group {} was not registered",
                    capture.group
                )));
            };
            let r = capture.region as usize;
            // #169 A4: like scratch, a capture is never shrunk or
            // regrown around an animated region size — it grows only
            // when the frame needs more, and `trim` releases it outside
            // the hot path.
            // A levelled group captures into a mipmapped texture whose
            // level-0 size is the region aligned up to the deepest
            // level's grid; the tail past the spec extent is never
            // written or read.
            let levels = group_state.spec.levels().get();
            let grid = 1 << (levels - 1);
            let (nw, nh) = (
                w.max(group_state.captures.get(r).map_or(0, |c| c.target.width))
                    .next_multiple_of(grid),
                h.max(group_state.captures.get(r).map_or(0, |c| c.target.height))
                    .next_multiple_of(grid),
            );
            if group_state.captures.get(r).is_none_or(|c| {
                c.target.width < nw || c.target.height < nh || c.target.texture.format() != format
            }) {
                let old_capture = group_state
                    .captures
                    .get(r)
                    .map_or(0, |c| target_bytes(&c.target, levels - 1));
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("backdrop capture"),
                    size: wgpu::Extent3d {
                        width: nw,
                        height: nh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: levels,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: TARGET_USAGES | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    mip_level_count: Some(1),
                    ..wgpu::TextureViewDescriptor::default()
                });
                let source = (levels > 1)
                    .then(|| texture.create_view(&wgpu::TextureViewDescriptor::default()));
                let levels_views: Vec<wgpu::TextureView> = (1..levels)
                    .map(|level| {
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            base_mip_level: level,
                            mip_level_count: Some(1),
                            ..wgpu::TextureViewDescriptor::default()
                        })
                    })
                    .collect();
                let target = Capture {
                    target: ScratchTarget {
                        view,
                        texture,
                        width: nw,
                        height: nh,
                    },
                    source,
                    levels: levels_views,
                    reduces: (1..levels).map(|_| None).collect(),
                };
                let new = target_bytes(&target.target, levels - 1);
                if r < group_state.captures.len() {
                    group_state.captures[r] = target;
                } else {
                    group_state.captures.resize_with(r, || Capture {
                        target: ScratchTarget {
                            texture: surf.target.clone(),
                            view: surf.view.clone(),
                            width: 0,
                            height: 0,
                        },
                        source: None,
                        levels: Vec::new(),
                        reduces: Vec::new(),
                    });
                    group_state.captures.push(target);
                }
                diag::grow(
                    &self.device,
                    "backdrop capture",
                    diag::Class::Target,
                    old_capture,
                    new,
                    0,
                    true,
                );
                surf.bind_gen += 1;
                // #169 A4: unsubmitted group-1 bind groups referencing
                // the replaced view keep its predecessor alive — drop
                // them now.
                let before = surf.binds1.len();
                surf.binds1
                    .retain(|key, _| {
                        !matches!(key.0, Some(Source::Backdrop { group, .. }) if group == capture.group)
                    });
                let dropped = before - surf.binds1.len();
                if dropped > 0 {
                    diag::bind_groups_dropped(&self.device, dropped as u64, "capture regen");
                }
                surf.binds1_stamp = (
                    surf.bind_gen,
                    self.images_gen,
                    self.atlas.mask_texture_generation(),
                );
            }
            if let Some(resolve) = resolve::of(pass).filter(|_| resolve::staged(pass)) {
                let entry = staged_max[staging_slot(format)].get_or_insert((format, 0, 0));
                entry.1 = entry.1.max(resolve.device[2]);
                entry.2 = entry.2.max(resolve.device[3]);
            }
        }
        // A staged resolve's 1:1 device copy, one per format, shared by
        // every staged resolve on the surface and grown like a capture:
        // allocated once for the frame's largest staged device rect.
        for (i, entry) in staged_max.into_iter().enumerate() {
            let Some((format, dw, dh)) = entry else {
                continue;
            };
            let slot = &mut surf.staging[i];
            if slot
                .as_ref()
                .is_none_or(|c| c.width < dw || c.height < dh || c.texture.format() != format)
            {
                let (old, nw, nh) = slot.as_ref().map_or((0, dw, dh), |c| {
                    (c.bytes(), dw.max(c.width), dh.max(c.height))
                });
                let (texture, view) = create_target(
                    &self.device,
                    "backdrop staging",
                    (nw, nh),
                    TARGET_USAGES,
                    format,
                );
                let target = ScratchTarget {
                    texture,
                    view,
                    width: nw,
                    height: nh,
                };
                let new = target.bytes();
                *slot = Some(target);
                // The slot's staged-resolve bind reads the replaced view:
                // drop it with the texture so the predecessor is freed.
                surf.staging_bind[i] = None;
                diag::grow(
                    &self.device,
                    "backdrop staging",
                    diag::Class::Target,
                    old,
                    new,
                    0,
                    true,
                );
            }
        }
        // Regions dropped between frames drop their textures too.
        let needed: FxHashMap<u64, u32> = surf
            .frame
            .passes
            .iter()
            .filter_map(|p| p.capture.map(|c| (c.group, c.region + 1)))
            .fold(FxHashMap::default(), |mut m, (g, n)| {
                m.entry(g).and_modify(|e| *e = (*e).max(n)).or_insert(n);
                m
            });
        for (gid, n) in needed {
            if let Some(state) = surf.backdrop_groups.get_mut(&gid)
                && state.captures.len() > n as usize
            {
                state.truncate(n as usize);
                surf.bind_gen += 1;
                let before = surf.binds1.len();
                surf.binds1.retain(
                    |key, _| !matches!(key.0, Some(Source::Backdrop { group, .. }) if group == gid),
                );
                let dropped = before - surf.binds1.len();
                if dropped > 0 {
                    diag::bind_groups_dropped(&self.device, dropped as u64, "capture trim");
                }
                surf.binds1_stamp = (
                    surf.bind_gen,
                    self.images_gen,
                    self.atlas.mask_texture_generation(),
                );
            }
        }
        // Backdrop textures for blend composites, sized like the scratch
        // pool to the largest region copied this frame.
        let mut backdrop_max = [(0u32, 0u32); 2];
        let passes = surf.frame.passes.iter();
        #[cfg(target_vendor = "apple")]
        let passes = passes.enumerate().filter_map(|(index, pass)| {
            (!surf.composition.contains_native_pass(index)).then_some(pass)
        });
        for pass in passes {
            if let Some(r) = pass.backdrop_copy {
                let slot = match pass.target {
                    Target::Part(_) | Target::Projected(_) | Target::Plane(_) => 0,
                    Target::Scratch(_) => 1,
                    Target::Backdrop { .. } => {
                        return Err(RenderError::Render(
                            "a blend backdrop copy on a capture pass".into(),
                        ));
                    }
                };
                backdrop_max[slot].0 = backdrop_max[slot].0.max(r[2]);
                backdrop_max[slot].1 = backdrop_max[slot].1.max(r[3]);
            }
        }
        for (slot, &(w, h)) in backdrop_max.iter().enumerate() {
            if w == 0 || h == 0 {
                continue;
            }
            if surf.backdrop[slot]
                .as_ref()
                .is_some_and(|b| b.width >= w && b.height >= h)
            {
                continue;
            }
            let (nw, nh) = (
                w.max(surf.backdrop[slot].as_ref().map_or(0, |b| b.width)),
                h.max(surf.backdrop[slot].as_ref().map_or(0, |b| b.height)),
            );
            let format = if slot == 0 {
                TARGET_FORMAT
            } else {
                self.scratch_format
            };
            let old_backdrop = surf.backdrop[slot].as_ref().map_or(0, |b| {
                u64::from(b.width) * u64::from(b.height) * texel_bytes(b.texture.format())
            });
            let (texture, view) = create_target(
                &self.device,
                "blend backdrop",
                (nw, nh),
                TARGET_USAGES | wgpu::TextureUsages::COPY_DST,
                format,
            );
            surf.backdrop[slot] = Some(ScratchTarget {
                texture,
                view,
                width: nw,
                height: nh,
            });
            diag::grow(
                &self.device,
                "blend backdrop",
                diag::Class::Target,
                old_backdrop,
                u64::from(nw) * u64::from(nh) * texel_bytes(format),
                0,
                true,
            );
            surf.bind_gen += 1;
            retire_binds1(
                surf,
                &self.device,
                self.images_gen,
                self.atlas.mask_texture_generation(),
                "backdrop regen",
                |key| key.1,
            );
        }
        // Gradient instances index stops absolutely; shift each instance's
        // first-stop index by this surface's stop base. Only gradient
        // paints read `meta.z`, so bumping it unconditionally is safe.
        if stop_base != 0 {
            for inst in &mut surf.frame.instances {
                inst.meta[2] += stop_base;
                // A union member's `uv[0]` bitcast holds its record run's
                // base `Stop` index inside the surface's stops section;
                // shift it into the shared frame buffer's coordinates.
                if inst.meta[3] >> 24 & instance::FLAG_UNION != 0 {
                    inst.uv[0] = f32::from_bits(inst.uv[0].to_bits() + stop_base);
                }
            }
        }
        surf.inst_base = inst_base;
        surf.globals_base = globals_base;
        {
            let atlas = &self.atlas;
            if atlas.generation() != self.bound_atlas {
                diag::bind_groups_dropped(&self.device, 1, "atlas generation");
                self.bind0 = make_bind0(
                    &self.device,
                    &self.layout0,
                    &self.globals,
                    &self.instances,
                    &self.stops,
                    atlas,
                );
                self.bound_atlas = atlas.generation();
                self.invalidate_painter_replays();
            }
        }
        // Buffers grown above leave `bind0` stale; rebuild when capacity
        // changed since the bind group was built.
        if self.instances.size() > self.bound_instance_size
            || self.stops.size() > self.bound_stop_size
            || self.globals.size() > self.bound_globals_size
        {
            diag::bind_groups_dropped(&self.device, 1, "buffer growth");
            let atlas = &self.atlas;
            self.bind0 = make_bind0(
                &self.device,
                &self.layout0,
                &self.globals,
                &self.instances,
                &self.stops,
                atlas,
            );
            self.bound_atlas = atlas.generation();
            self.invalidate_painter_replays();
            self.bound_instance_size = self.instances.size();
            self.bound_stop_size = self.stops.size();
            self.bound_globals_size = self.globals.size();
        }

        Ok(())
    }

    fn invalidate_painter_replays(&mut self) {
        for surface in self.surfaces.values_mut() {
            surface.painter_replays.clear();
        }
    }

    /// Grows each frame-wide buffer once for the frame's whole upload
    /// range, before the frame's first submission — never per surface
    /// mid-frame (#169 A2 on the staging ring).
    fn grow_frame_buffers(&mut self, copies: &[upload::Copy]) {
        let mut passes = 0u64;
        for copy in copies {
            let (label, usage, buffer) = match copy.dest {
                upload::Dest::Instances => (
                    "instances",
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    &mut self.instances,
                ),
                upload::Dest::Stops => (
                    "stops",
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    &mut self.stops,
                ),
                upload::Dest::Globals => (
                    "globals",
                    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    &mut self.globals,
                ),
            };
            let needed = copy.dst + copy.size;
            if needed > buffer.size() {
                *buffer = grow_buffer(
                    &self.device,
                    label,
                    buffer,
                    needed.next_power_of_two(),
                    usage,
                );
            }
            if matches!(copy.dest, upload::Dest::Globals) {
                passes = copy.size / 256;
            }
        }
        // The timestamp query set and resolve buffer grow once for the
        // frame's whole pass count; never mid-encoder.
        if self.timestamps && passes > 0 {
            self.ensure_query_capacity(2 * u32::try_from(passes).unwrap_or(u32::MAX));
        }
    }

    /// The frame's upload size and copies: every dirty surface's instances,
    /// stops and 256-byte globals entries, laid out in the staging slot in
    /// the order they occupy the frame-wide buffers from offset 0.
    fn upload_layout(&self, dirty: &[&SurfaceFrame<'_>]) -> (u64, Vec<upload::Copy>) {
        let (mut instances, mut stops, mut passes) = (0u64, 0u64, 0u64);
        for surf in dirty.iter().filter_map(|sf| self.surfaces.get(&sf.id)) {
            instances += surf.frame.instances.len() as u64;
            stops += surf.frame.stops.len() as u64;
            passes += surf.frame.passes.len() as u64 + u64::from(surf.frame.reduce_slots());
        }
        let sizes = [
            (
                upload::Dest::Instances,
                instances * std::mem::size_of::<instance::Instance>() as u64,
            ),
            (
                upload::Dest::Stops,
                stops * std::mem::size_of::<instance::Stop>() as u64,
            ),
            (upload::Dest::Globals, passes * 256),
        ];
        let mut src = 0;
        let mut copies = Vec::new();
        for (dest, size) in sizes {
            if size > 0 {
                copies.push(upload::Copy {
                    dest,
                    src,
                    dst: 0,
                    size,
                });
                src += size;
            }
        }
        (src, copies)
    }

    /// Fills the acquired staging slot with this frame's uploads.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel sizes are well within f32"
    )]
    fn write_uploads(&mut self, dirty: &[&SurfaceFrame<'_>], size: u64, copies: &[upload::Copy]) {
        diag::upload(&self.device, "frame staging", size, None);
        let surfaces: Vec<&SurfaceState> = dirty
            .iter()
            .filter_map(|sf| self.surfaces.get(&sf.id))
            .collect();
        self.uploads.write(size, copies, |bytes| {
            let mut at = 0;
            let mut put = |data: &[u8]| {
                bytes.slice(at..at + data.len()).copy_from_slice(data);
                at += data.len();
            };
            for surf in &surfaces {
                put(bytemuck::cast_slice(&surf.frame.instances));
            }
            for surf in &surfaces {
                put(bytemuck::cast_slice(&surf.frame.stops));
            }
            let mut entry = [0u8; 256];
            for surf in &surfaces {
                // A staged resolve's draws land in device space; its
                // parameters follow the globals in the same slot.
                let globals = surf.frame.passes.iter().map(|pass| {
                    let region = resolve::draw_region(pass);
                    (
                        lower::globals(
                            [region[2] as f32, region[3] as f32],
                            [region[0] as f32, region[1] as f32],
                            pass.space,
                        ),
                        resolve::params(pass),
                    )
                });
                #[cfg(target_vendor = "apple")]
                let globals = globals.enumerate().map(|(index, (mut g, params))| {
                    if let Some(origin) = surf.composition.attachment_origin(index) {
                        g.attachment_origin = origin.map(|value| value as f32);
                    }
                    (g, params)
                });
                for (g, params) in globals {
                    let g = bytemuck::bytes_of(&g);
                    let params = bytemuck::bytes_of(&params);
                    entry[..g.len()].copy_from_slice(g);
                    entry[g.len()..g.len() + params.len()].copy_from_slice(params);
                    put(&entry);
                }
                // The pyramid steps' slots follow the pass slots: one
                // per reduce, in pass and level order, reading the
                // level above's spec extent.
                for &(index, levels) in &surf.frame.reduces {
                    let pass = &surf.frame.passes[index];
                    for k in 1..levels {
                        let dst = (
                            pass.region[2].div_ceil(1 << k),
                            pass.region[3].div_ceil(1 << k),
                        );
                        let src = (
                            pass.region[2].div_ceil(1 << (k - 1)),
                            pass.region[3].div_ceil(1 << (k - 1)),
                        );
                        let g = lower::globals([dst.0 as f32, dst.1 as f32], [0.0; 2], pass.space);
                        let params = reduce::params(src);
                        let g = bytemuck::bytes_of(&g);
                        let params = bytemuck::bytes_of(&params);
                        entry[..g.len()].copy_from_slice(g);
                        entry[g.len()..g.len() + params.len()].copy_from_slice(params);
                        put(&entry);
                    }
                }
            }
        });
    }

    /// Stages the frame's uploads; time spent waiting for a staging slot
    /// the GPU has not finished copying out of goes to `wait_seconds`.
    #[cfg(not(target_arch = "wasm32"))]
    fn upload_frame(
        &mut self,
        dirty: &[&SurfaceFrame<'_>],
        stats: &mut FrameStats,
    ) -> Result<(), RenderError> {
        let (size, copies) = self.upload_layout(dirty);
        if size == 0 {
            return Ok(());
        }
        self.grow_frame_buffers(&copies);
        if let upload::Acquire::Wait(submission) = self.uploads.acquire(&self.device, size)? {
            let start = Instant::now();
            self.wait(&submission, "upload staging")?;
            stats.phases.wait_seconds += start.elapsed().as_secs_f64();
            self.uploads.check_mapped()?;
        }
        self.write_uploads(dirty, size, &copies);
        Ok(())
    }

    /// Stages the frame's uploads; time spent waiting for a staging slot
    /// the GPU has not finished copying out of goes to `wait_seconds`.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn upload_frame(
        &mut self,
        dirty: &[&SurfaceFrame<'_>],
        stats: &mut FrameStats,
    ) -> Result<(), RenderError> {
        let (size, copies) = self.upload_layout(dirty);
        if size == 0 {
            return Ok(());
        }
        self.grow_frame_buffers(&copies);
        if let upload::Acquire::Wait(submission) = self.uploads.acquire(&self.device, size)? {
            tracing::trace!(?submission, "awaiting the upload slot's map");
            let start = Instant::now();
            if let Some(mapped) = self.uploads.take_mapped() {
                browser_wait(mapped, self.wait_timeout, "upload staging").await?;
            }
            stats.phases.wait_seconds += start.elapsed().as_secs_f64();
            self.uploads.check_mapped()?;
        }
        self.write_uploads(dirty, size, &copies);
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        reason = "pixel sizes are well within f32"
    )]
    fn encode_surface(
        &mut self,
        id: SurfaceId,
        timing: filtrate::EffectFrameTiming,
        stats: &mut FrameStats,
    ) -> Result<(), RenderError> {
        // External pipelines stay lazy until a surface first samples an
        // external frame; the scan stays on the (unchanged) surface state.
        // A previous failed encode may have left acquisitions staged;
        // drop them so this encode's submission can't consume them.
        #[cfg(all(unix, not(target_vendor = "apple")))]
        if let Some(native) = self.native.as_mut() {
            external::vulkan::cancel_staged(native);
        }
        let needs_external = self.surfaces.get(&id).is_some_and(|surf| {
            surf.frame
                .passes
                .iter()
                .flat_map(|pass| &pass.ranges)
                .any(|range| matches!(range.image, Some(lower::ImageSource::Content(_))))
        });
        if needs_external {
            self.ensure_external()?;
        }
        if self
            .surfaces
            .get(&id)
            .is_some_and(|surf| !surf.composed.is_empty())
        {
            self.ensure_projective()?;
        }
        // The hal adapter handle is bound before `surf` borrows the
        // renderer: the native pass maps target formats through it inside
        // the encode loop (#166).
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let hal_adapter = self
            .native
            .is_some()
            // SAFETY: `self.adapter` is the engine's own wgpu adapter,
            // live for the borrow; Vulkan is asserted by `expect`.
            .then(|| unsafe { self.adapter.as_hal::<wgpu::hal::vulkan::Api>() }.expect("vulkan"));
        // Buffers grown during lowering leave `bind0` stale; rebuild when
        // capacity changed since the bind group was built.
        if self.instances.size() > self.bound_instance_size
            || self.stops.size() > self.bound_stop_size
            || self.globals.size() > self.bound_globals_size
        {
            let atlas = &self.atlas;
            diag::bind_groups_dropped(&self.device, 1, "buffer growth");
            self.bind0 = make_bind0(
                &self.device,
                &self.layout0,
                &self.globals,
                &self.instances,
                &self.stops,
                atlas,
            );
            self.bound_atlas = atlas.generation();
            self.bound_instance_size = self.instances.size();
            self.bound_stop_size = self.stops.size();
            self.bound_globals_size = self.globals.size();
            self.invalidate_painter_replays();
        }
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        diag::set_surface(Some(id.raw()));
        let inst_base = surf.inst_base;
        // wgpu forbids mixing its encoding API with raw `as_hal_mut`
        // access on one encoder, so every native command buffer — the
        // first-use acquire barriers, the external-frame composition op
        // and the release barriers — is recorded on a dedicated raw
        // encoder and spliced between finished wgpu buffers; queue order
        // inside the single submission preserves the intended sequence.
        let mut buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        // The frame's first submission carries every dirty surface's
        // uploads on its own encoder, queued ahead of the passes that
        // consume them: the staging slot's map — and the wait the next
        // acquire takes — then covers only the copy work itself, never
        // this frame's rendering.
        let uploads = self.uploads.take_copies();
        let upload_buffer = uploads.map(|(staging, copies)| {
            let mut upload_encoder =
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("upload staging"),
                    });
            for copy in &copies {
                let dest = match copy.dest {
                    upload::Dest::Instances => &self.instances,
                    upload::Dest::Stops => &self.stops,
                    upload::Dest::Globals => &self.globals,
                };
                upload_encoder.copy_buffer_to_buffer(&staging, copy.src, dest, copy.dst, copy.size);
            }
            upload_encoder.finish()
        });
        // Group-1 bind groups persist across frames, keyed by
        // (source scratch, backdrop-needed, image, mask texture); the
        // stamp rebuilds them when a scratch/backdrop texture, the image
        // set, or the mask textures changed.
        let stamp = (
            surf.bind_gen,
            self.images_gen,
            self.atlas.mask_texture_generation(),
        );
        if surf.binds1_stamp != stamp {
            let dropped = surf.binds1.len() as u64;
            if dropped > 0 {
                diag::bind_groups_dropped(&self.device, dropped, "stamp change");
            }
            surf.binds1.clear();
            surf.painter_replays.clear();
            surf.binds1_stamp = stamp;
        }
        let factory = PipelineFactory {
            device: &self.device,
            config: &self.config,
            layout: &self.pipeline_layout,
            cache: self.pipeline_cache.as_ref(),
            delivery: self.shader_delivery,
        };
        surf.painter_replays
            .retain(|index, _| *index < surf.frame.passes.len());
        let mask_gen = self.atlas.mask_texture_generation();
        let mut i = 0;
        // The pyramid steps' globals-slot index within this surface:
        // `write_uploads` lays them out in pass and level order after the
        // pass slots.
        let mut reduce_slot = 0u32;
        while i < surf.frame.passes.len() {
            #[cfg(target_vendor = "apple")]
            if let Some(epoch) = surf.composition.epoch_at(i) {
                let end = epoch.passes.end;
                let pass_index = self.frame_pass_count;
                self.frame_pass_count += 1;
                if self.timestamps {
                    self.pass_meta.push(PassMeta {
                        name: "tile composition".into(),
                        width: epoch.region[2],
                        height: epoch.region[3],
                        format: "rgba16float",
                    });
                }
                composite::metal::encode(
                    &mut self.tile_executor,
                    &composite::metal::Resources {
                        device: &self.device,
                        layout: &self.pipeline_layout,
                        layout1: &self.layout1,
                        dummy: &self.dummy_view,
                        bind0: &self.bind0,
                        images: &self.images,
                        bitmaps: &self.bitmaps,
                        atlas: &self.atlas,
                    },
                    surf,
                    i,
                    &mut encoder,
                    self.query_set
                        .as_ref()
                        .map(|query_set| wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: Some(self.query_base + 2 * pass_index),
                            end_of_pass_write_index: Some(self.query_base + 2 * pass_index + 1),
                        }),
                    stats,
                );
                stats.passes += 1;
                i = end;
                continue;
            }
            // First reduced capture of the engine's life builds the
            // resolve pipelines here, like the shadow blur (#170).
            let resolve_draw = if resolve::of(&surf.frame.passes[i]).is_some() {
                let pipelines = self.resolve.get_or_insert_with(|| {
                    resolve::Pipelines::new(&self.device, self.shader_delivery)
                });
                prepare_resolve(
                    &self.device,
                    pipelines,
                    &self.globals,
                    surf,
                    i,
                    self.scratch_format,
                )?
            } else {
                None
            };
            let pass = &surf.frame.passes[i];
            let coverage_order = matches!(pass.target, Target::Part(_))
                && !pass.ranges.is_empty()
                && pass.ranges.iter().all(|range| {
                    range.pipeline == PipelineKind::SrcOver
                        && range.variant == ShaderVariant::Simple
                        && range.source.is_none()
                        && range.image.is_none()
                        && range.mask.is_none()
                });
            let replayable = !coverage_order
                && !pass.ranges.is_empty()
                && pass.ranges.iter().all(|range| {
                    range.pipeline == PipelineKind::SrcOver
                        && matches!(range.variant, ShaderVariant::Simple | ShaderVariant::Shadow)
                        && range.source.is_none()
                        && range.image.is_none()
                        && range.mask.is_none()
                });
            if !replayable {
                surf.painter_replays.remove(&i);
            }
            if coverage_order
                && surf
                    .coverage_depth
                    .as_ref()
                    .is_none_or(|depth| (depth.width, depth.height) != surf.size)
            {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("coverage depth"),
                    size: wgpu::Extent3d {
                        width: surf.size.0,
                        height: surf.size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TRANSIENT_ATTACHMENT,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                tracing::debug!(
                    nominal_bytes = u64::from(surf.size.0) * u64::from(surf.size.1) * 4,
                    resident_bytes = coverage_depth_bytes(&texture),
                    "transient coverage depth allocated"
                );
                surf.coverage_depth = Some(ScratchTarget {
                    texture,
                    view,
                    width: surf.size.0,
                    height: surf.size.1,
                });
            }
            let (view, texture) = match pass.target {
                Target::Plane(layer) => {
                    let capture = surf.static_layers[&layer]
                        .capture
                        .as_ref()
                        .expect("capture allocated before encode");
                    let (texture, view) = capture
                        .source
                        .as_ref()
                        .expect("capture source allocated before encode");
                    (view, texture)
                }
                Target::Part(n) => surf.part(n),
                Target::Scratch(i) => (&surf.scratch[&i].view, &surf.scratch[&i].texture),
                // A staged resolve's draws land in the staging texture.
                Target::Backdrop { group, region } => {
                    let group_state = &surf.backdrop_groups[&group];
                    let capture = if resolve::of(pass).is_some() && resolve::staged(pass) {
                        let slot = staging_slot(capture_format(
                            pass.capture
                                .expect("a staged resolve implies a capture")
                                .copy_from,
                            self.scratch_format,
                        )?);
                        surf.staging[slot]
                            .as_ref()
                            .expect("staging allocated before encode")
                    } else {
                        &group_state.captures[region as usize].target
                    };
                    (&capture.view, &capture.texture)
                }
                Target::Projected(key) => {
                    let entry = projective_entry(&surf.projective, key);
                    (&entry.levels[0], &entry.texture)
                }
            };
            // A backdrop-group capture first copies its draw region out
            // of its `copy_from` target into the group's capture texture —
            // into the staging texture for a staged resolve. A direct
            // resolve reads `copy_from` in the pass instead.
            let draw_region = resolve::draw_region(pass);
            if let Some(capture) = pass.capture
                && (capture.resolve.is_none() || resolve::staged(pass))
            {
                let src = match capture.copy_from {
                    Target::Plane(layer) => {
                        &surf.static_layers[&layer]
                            .capture
                            .as_ref()
                            .expect("allocated capture")
                            .source
                            .as_ref()
                            .expect("capture source allocated before encode")
                            .0
                    }
                    Target::Part(n) => surf.part(n).1,
                    Target::Projected(key) => &projective_entry(&surf.projective, key).texture,
                    Target::Scratch(k) => &surf.scratch[&k].texture,
                    Target::Backdrop { group, region } => {
                        &surf.backdrop_groups[&group].captures[region as usize]
                            .target
                            .texture
                    }
                };
                debug_assert!(draw_region[0] + draw_region[2] <= src.width());
                debug_assert!(draw_region[1] + draw_region[3] <= src.height());
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: src,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: draw_region[0],
                            y: draw_region[1],
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: draw_region[2],
                        height: draw_region[3],
                        depth_or_array_layers: 1,
                    },
                );
            }
            // A blend pass reads the target's prior contents from a copy;
            // the copy must complete before the pass starts.
            if let Some([bx, by, bw, bh]) = pass.backdrop_copy {
                let slot = match pass.target {
                    Target::Part(_) | Target::Projected(_) | Target::Plane(_) => 0,
                    Target::Scratch(_) => 1,
                    Target::Backdrop { .. } => {
                        return Err(RenderError::Render(
                            "a blend backdrop copy on a capture pass".into(),
                        ));
                    }
                };
                let backdrop = surf.backdrop[slot].as_ref().expect("grown above");
                // The copy region is recorded in device space; a scratch
                // target stores its contents offset by its pass region.
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: bx.saturating_sub(pass.region[0]),
                            y: by.saturating_sub(pass.region[1]),
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &backdrop.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: bw,
                        height: bh,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let load = match pass.clear {
                Some([red, green, blue, alpha]) => wgpu::LoadOp::Clear(wgpu::Color {
                    r: f64::from(red),
                    g: f64::from(green),
                    b: f64::from(blue),
                    a: f64::from(alpha),
                }),
                None => wgpu::LoadOp::Load,
            };
            let pass_index = self.frame_pass_count;
            self.frame_pass_count += 1;
            // A pass containing native external ops splits at each op; its
            // beginning sample lands on the first segment and its end
            // sample on the last (reopened) segment, so the recorded GPU
            // time spans the native composition too.
            #[cfg(all(unix, not(target_vendor = "apple")))]
            let native_run = self.native.is_some()
                && pass.ranges.iter().any(|range| {
                    matches!(&range.image, Some(lower::ImageSource::Content(id))
                        if self
                            .producers
                            .get(id)
                            .and_then(gpu_content::Producer::current)
                            .and_then(|slot| slot.native_frame())
                            .is_some())
                });
            #[cfg(not(all(unix, not(target_vendor = "apple"))))]
            let native_run = false;
            let timestamp_writes =
                self.query_set
                    .as_ref()
                    .map(|qs| wgpu::RenderPassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(self.query_base + 2 * pass_index),
                        end_of_pass_write_index: (!native_run)
                            .then_some(self.query_base + 2 * pass_index + 1),
                    });
            // Only the timestamp path reads `pass_meta`; skip the
            // allocation when timing is off.
            if self.timestamps {
                self.pass_meta.push(PassMeta {
                    name: match pass.target {
                        Target::Plane(layer) => format!("plane{}", layer.raw()),
                        Target::Part(0) => "surface".to_string(),
                        Target::Part(n) => format!("part{n}"),
                        Target::Scratch(i) => format!("scratch{i}"),
                        Target::Backdrop { group, region } => {
                            format!("backdrop{group}.{region}")
                        }
                        Target::Projected(key) => {
                            format!("projected{}.{}", key.layer.raw(), key.bucket)
                        }
                    },
                    width: pass.region[2],
                    height: pass.region[3],
                    format: format_name(texture.format()),
                });
            }
            let scratch_backdrop = pass.backdrop_copy.is_some();
            // First-use acquisition (#166): for every native generation this
            // pass samples — `Planes`/`ExternalFormat` bound in the native
            // op and `Rgb` bound through wgpu — record the acquire barrier
            // and stage its wait before the pass opens. `stage_acquire`
            // deduplicates generations already staged or owned. The native
            // context is built at frame registration; a missing one is an
            // error, not a skip (#170).
            #[cfg(all(unix, not(target_vendor = "apple")))]
            {
                let gens: Vec<std::sync::Arc<external::vulkan::Generation>> = pass
                    .ranges
                    .iter()
                    .filter_map(|range| match &range.image {
                        Some(lower::ImageSource::Content(id)) => self
                            .producers
                            .get(id)
                            .and_then(gpu_content::Producer::current)
                            .and_then(|slot| {
                                slot.vulkan_frame().map(|frame| frame.generation.clone())
                            }),
                        _ => None,
                    })
                    .collect();
                if !gens.is_empty() {
                    let Some(native) = self.native.as_mut() else {
                        return Err(self.native_missing());
                    };
                    buffers.push(split_encoder(&mut encoder, &self.device));
                    let mut acquire =
                        self.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("acquire"),
                            });
                    // SAFETY: `acquire` is a live Vulkan encoder; the
                    // closure's `cb` is recording, which is
                    // `stage_acquire`'s contract.
                    unsafe {
                        acquire.as_hal_mut::<wgpu::hal::vulkan::Api, _, _>(|hal| {
                            let hal = hal.expect("vulkan encoder");
                            let cb = hal.raw_handle();
                            for generation in &gens {
                                if let Some(pending) =
                                    external::vulkan::stage_acquire(generation, cb)?
                                {
                                    native.staged.push(pending);
                                }
                            }
                            Ok::<(), external::vulkan::NativeError>(())
                        })
                    }
                    .map_err(|e| RenderError::Render(e.to_string()))?;
                    buffers.push(acquire.finish());
                }
            }
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: coverage_order.then(|| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &surf
                            .coverage_depth
                            .as_ref()
                            .expect("coverage depth allocated")
                            .view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }
                }),
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let format_i = usize::from(texture.format() != TARGET_FORMAT);
            // `view`/`texture` can borrow `surf.projective` (a projected
            // pass target); the loop's projective draws need
            // `surf.projective` mutably, so the native op, the reopened
            // pass and demand pipeline creation use these owned handles
            // and copies instead.
            let target_format = texture.format();
            #[cfg(all(unix, not(target_vendor = "apple")))]
            let target_view = view.clone();
            #[cfg(all(unix, not(target_vendor = "apple")))]
            let target_size = (texture.width(), texture.height());
            if coverage_order {
                render_pass.set_pipeline(coverage_pipeline(
                    &mut self.coverage_pipelines,
                    &factory,
                    CoveragePhase::Opaque,
                )?);
                render_pass.set_bind_group(
                    0,
                    &self.bind0,
                    &[(surf.globals_base + u32::try_from(i).unwrap()) * 256],
                );
                let bind = surf
                    .binds1
                    .entry((None, false, None, None))
                    .or_insert_with(|| {
                        stats.bind_groups_created += 1;
                        make_bind1(
                            &self.device,
                            &self.layout1,
                            &self.dummy_view,
                            None,
                            None,
                            None,
                            None,
                        )
                    });
                render_pass.set_bind_group(1, &*bind, &[]);
                for range in &pass.ranges {
                    render_pass.draw(
                        0..8,
                        (inst_base + range.instances.start)..(inst_base + range.instances.end),
                    );
                    stats.draws += 1;
                }
                render_pass.set_pipeline(coverage_pipeline(
                    &mut self.coverage_pipelines,
                    &factory,
                    CoveragePhase::Partial,
                )?);
                render_pass
                    .set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint16);
                stats.pipeline_switches += 2;
            } else {
                render_pass.set_pipeline(core_pipeline(
                    &mut self.pipelines,
                    &factory,
                    target_format,
                    false,
                    ShaderVariant::Simple,
                )?);
            }
            // Region-targeted passes cover only their region; the surface
            // pass the whole target. `in.device` stays in true device
            // space via the per-pass Globals origin.
            if !matches!(pass.target, Target::Part(_)) {
                render_pass.set_viewport(
                    0.0,
                    0.0,
                    draw_region[2] as f32,
                    draw_region[3] as f32,
                    0.0,
                    1.0,
                );
                render_pass.set_scissor_rect(0, 0, draw_region[2], draw_region[3]);
            }
            // The uniform slot written for this pass above (256-byte
            // stride), which matches `surf.frame.passes` ordering.
            let offset = (surf.globals_base + u32::try_from(i).unwrap()) * 256;
            // A direct resolve is the pass's only draw.
            if let Some(draw) = resolve_draw.as_ref().filter(|draw| draw.staged.is_none()) {
                resolve::draw(
                    &mut render_pass,
                    &draw.pipeline,
                    &draw.bind,
                    offset,
                    (pass.region[2], pass.region[3]),
                );
            }
            render_pass.set_bind_group(0, &self.bind0, &[offset]);
            // External ranges bind a slot's group-1 over the external
            // pipeline; an engine range after one must rebind its pipeline.
            let mut pipeline = Bound::Engine(PipelineKind::SrcOver, ShaderVariant::Simple);
            let backdrop_slot = scratch_backdrop.then_some(match pass.target {
                Target::Part(_) | Target::Projected(_) | Target::Plane(_) => 0,
                Target::Scratch(_) | Target::Backdrop { .. } => 1,
            });
            if replayable {
                let bind1 = surf
                    .binds1
                    .entry((None, false, None, None))
                    .or_insert_with(|| {
                        stats.bind_groups_created += 1;
                        make_bind1(
                            &self.device,
                            &self.layout1,
                            &self.dummy_view,
                            None,
                            None,
                            None,
                            None,
                        )
                    });
                let current = surf.painter_replays.get(&i).is_some_and(|replay| {
                    replay.bind0 == self.bind0
                        && replay.bind1 == *bind1
                        && replay.offset == offset
                        && replay.base == inst_base
                        && replay.format == target_format
                        && replay.ranges.len() == pass.ranges.len()
                        && replay.ranges.iter().zip(&pass.ranges).all(
                            |((variant, instances), range)| {
                                *variant == range.variant && *instances == range.instances
                            },
                        )
                });
                if !current {
                    for range in &pass.ranges {
                        core_pipeline(
                            &mut self.pipelines,
                            &factory,
                            target_format,
                            false,
                            range.variant,
                        )?;
                    }
                    let mut bundle = self.device.create_render_bundle_encoder(
                        &wgpu::RenderBundleEncoderDescriptor {
                            label: Some("retained painter"),
                            color_formats: &[Some(target_format)],
                            depth_stencil: None,
                            sample_count: 1,
                            multiview: None,
                        },
                    );
                    bundle.set_bind_group(0, &self.bind0, &[offset]);
                    bundle.set_bind_group(1, &*bind1, &[]);
                    let mut variant = None;
                    for range in &pass.ranges {
                        if variant != Some(range.variant) {
                            bundle.set_pipeline(
                                self.pipelines[format_i][0][variant_index(range.variant)]
                                    .as_ref()
                                    .expect("painter pipeline prepared"),
                            );
                            variant = Some(range.variant);
                        }
                        bundle.draw(
                            0..6,
                            (inst_base + range.instances.start)..(inst_base + range.instances.end),
                        );
                    }
                    surf.painter_replays.insert(
                        i,
                        PainterReplay {
                            bind0: self.bind0.clone(),
                            bind1: bind1.clone(),
                            offset,
                            base: inst_base,
                            format: target_format,
                            ranges: pass
                                .ranges
                                .iter()
                                .map(|range| (range.variant, range.instances.clone()))
                                .collect(),
                            bundle: bundle.finish(&wgpu::RenderBundleDescriptor {
                                label: Some("retained painter"),
                            }),
                        },
                    );
                }
                render_pass.execute_bundles([&surf.painter_replays[&i].bundle]);
                stats.draws +=
                    u32::try_from(pass.ranges.len()).expect("draw ranges fit the instance limit");
                stats.pipeline_switches += u32::try_from(
                    pass.ranges
                        .windows(2)
                        .filter(|pair| pair[0].variant != pair[1].variant)
                        .count(),
                )
                .expect("pipeline switches fit the instance limit")
                    + 1;
            }
            let mut ri = if replayable { pass.ranges.len() } else { 0 };
            while ri < pass.ranges.len() {
                let range = &pass.ranges[ri];
                stats.draws += 1;
                if let PipelineKind::Projective { replace } = range.pipeline {
                    let pipes = self
                        .projective
                        .as_ref()
                        .expect("projective pipelines are built before encoding");
                    if pipeline != Bound::Projective(replace) {
                        pipeline = Bound::Projective(replace);
                        stats.pipeline_switches += 1;
                        render_pass.set_pipeline(&pipes.composite[format_i][usize::from(replace)]);
                    }
                    let Some(Source::Projected(key)) = range.source else {
                        unreachable!("a projective range samples a local image")
                    };
                    let backdrop = backdrop_slot
                        .map(|slot| &surf.backdrop[slot].as_ref().expect("grown above").view);
                    let mask = range.mask.map(|k| {
                        self.atlas
                            .mask_texture_view(k)
                            .expect("mask texture stored before encode")
                    });
                    let entry = projective_entry_mut(&mut surf.projective, key);
                    let bind = entry.bind(
                        &self.device,
                        pipes,
                        (backdrop_slot, backdrop),
                        (range.mask, mask),
                        (surf.bind_gen, mask_gen),
                        &self.dummy_view,
                    );
                    render_pass.set_bind_group(1, bind, &[]);
                    render_pass.draw(
                        0..6,
                        (inst_base + range.instances.start)..(inst_base + range.instances.end),
                    );
                    ri += 1;
                    continue;
                }
                // A `Planes`/`ExternalFormat` generation draws in the Vulkan
                // native operation: end the wgpu pass, record the native
                // composition into the same target, then reopen the wgpu
                // pass with a Load (#166). `Rgb` generations stay on the
                // ordinary external path below.
                #[cfg(all(unix, not(target_vendor = "apple")))]
                if let Some(lower::ImageSource::Content(id)) = &range.image
                    && self.native.is_some()
                    && self
                        .producers
                        .get(id)
                        .and_then(gpu_content::Producer::current)
                        .and_then(|slot| slot.native_frame())
                        .is_some()
                {
                    drop(render_pass);
                    let mut draws = Vec::new();
                    let mut end = ri;
                    while end < pass.ranges.len() {
                        let Some(lower::ImageSource::Content(pid)) = &pass.ranges[end].image else {
                            break;
                        };
                        let Some(frame) = self
                            .producers
                            .get(pid)
                            .and_then(gpu_content::Producer::current)
                            .and_then(|slot| slot.native_frame())
                        else {
                            break;
                        };
                        let slot = self
                            .producers
                            .get(pid)
                            .and_then(gpu_content::Producer::current)
                            .expect("slot checked");
                        draws.push(external::vulkan::OpDraw {
                            generation: frame.generation.clone(),
                            first_instance: inst_base + pass.ranges[end].instances.start,
                            instance_count: pass.ranges[end].instances.end
                                - pass.ranges[end].instances.start,
                            mask: pass.ranges[end].mask.map(|key| {
                                raw_vk_view(
                                    self.atlas
                                        .mask_texture_view(key)
                                        .expect("mask texture stored before encode"),
                                )
                            }),
                            mask_gen: self.atlas.mask_texture_generation(),
                            params: raw_vk_buffer(slot.params_buffer()),
                        });
                        end += 1;
                    }
                    stats.draws += u32::try_from(draws.len() - 1).unwrap_or(u32::MAX);
                    stats.passes += 1;
                    let vk_format = hal_adapter
                        .as_ref()
                        .expect("native run implies vulkan")
                        .texture_format_as_raw(target_format);
                    let native = self.native.as_mut().expect("checked");
                    let set0 = native
                        .set0_set(
                            raw_vk_buffer(&self.globals),
                            raw_vk_buffer(&self.instances),
                            raw_vk_buffer(&self.stops),
                            raw_vk_view(self.atlas.view()),
                        )
                        .map_err(|e| RenderError::Render(e.to_string()))?;
                    buffers.push(split_encoder(&mut encoder, &self.device));
                    let mut op =
                        self.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("external"),
                            });
                    // SAFETY: `op` is a live Vulkan encoder; the
                    // closure's `cb` is recording and every argument
                    // (`target_view`, sets, `native`'s tables) is live.
                    unsafe {
                        op.as_hal_mut::<wgpu::hal::vulkan::Api, _, _>(|hal| {
                            let hal = hal.expect("vulkan encoder");
                            let cb = hal.raw_handle();
                            native.record(
                                cb,
                                raw_vk_view(&target_view),
                                target_size,
                                vk_format,
                                set0,
                                &draws,
                                offset,
                                surf.bind_gen,
                            )
                        })
                    }
                    .map_err(|e| RenderError::Render(e.to_string()))?;
                    buffers.push(op.finish());
                    // The end sample belongs on the LAST reopened segment:
                    // a pass with a later native op still coming must not
                    // write it here.
                    let more_native = pass.ranges[end..].iter().any(|range| {
                        matches!(&range.image, Some(lower::ImageSource::Content(id))
                            if self
                                .producers
                                .get(id)
                                .and_then(gpu_content::Producer::current)
                                .and_then(|slot| slot.native_frame())
                                .is_some())
                    });
                    let reopen_writes =
                        self.query_set
                            .as_ref()
                            .map(|qs| wgpu::RenderPassTimestampWrites {
                                query_set: qs,
                                beginning_of_pass_write_index: None,
                                end_of_pass_write_index: (!more_native)
                                    .then_some(self.query_base + 2 * pass_index + 1),
                            });
                    render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: reopen_writes,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    render_pass.set_pipeline(core_pipeline(
                        &mut self.pipelines,
                        &factory,
                        target_format,
                        false,
                        ShaderVariant::Simple,
                    )?);
                    if !matches!(pass.target, Target::Part(_)) {
                        render_pass.set_viewport(
                            0.0,
                            0.0,
                            pass.region[2] as f32,
                            pass.region[3] as f32,
                            0.0,
                            1.0,
                        );
                        render_pass.set_scissor_rect(0, 0, pass.region[2], pass.region[3]);
                    }
                    render_pass.set_bind_group(0, &self.bind0, &[offset]);
                    pipeline = Bound::Engine(PipelineKind::SrcOver, ShaderVariant::Simple);
                    ri = end;
                    continue;
                }
                if let Some(lower::ImageSource::Content(id)) = &range.image {
                    // The binding draws nothing while its producer has
                    // no current frame — a submitted frame that has not
                    // landed, or a render the full ring skipped.
                    let Some(slot) = self
                        .producers
                        .get_mut(id)
                        .and_then(gpu_content::Producer::current_mut)
                    else {
                        ri += 1;
                        continue;
                    };
                    if pipeline != Bound::External {
                        pipeline = Bound::External;
                        stats.pipeline_switches += 1;
                        let Some(pipe) = &self.external_pipes[format_i] else {
                            return Err(RenderError::Render("external pipeline unbuilt".into()));
                        };
                        render_pass.set_pipeline(pipe);
                    }
                    let Some(ext_layout) = &self.ext_layout else {
                        return Err(RenderError::Render("external layout unbuilt".into()));
                    };
                    let bind = slot.bind(
                        &self.device,
                        ext_layout,
                        external::MaskBinding {
                            key: range.mask,
                            view: range.mask.map(|key| {
                                self.atlas
                                    .mask_texture_view(key)
                                    .expect("mask texture stored before encode")
                            }),
                            generation: self.atlas.mask_texture_generation(),
                        },
                        &self.dummy_view,
                        &self.dummy_uint_view,
                    );
                    render_pass.set_bind_group(1, bind, &[]);
                    render_pass.draw(
                        0..6,
                        (inst_base + range.instances.start)..(inst_base + range.instances.end),
                    );
                    ri += 1;
                    continue;
                }
                let want = Bound::Engine(range.pipeline, range.variant);
                if want != pipeline {
                    pipeline = want;
                    stats.pipeline_switches += 1;
                    let Bound::Engine(kind, variant) = pipeline else {
                        unreachable!("engine want")
                    };
                    let pipe = match kind {
                        PipelineKind::Effect(id) => {
                            let entry = self.backdrop_shaders.get_mut(&id).ok_or_else(|| {
                                RenderError::Render(format!(
                                    "backdrop shader {id} is not registered"
                                ))
                            })?;
                            if variant == ShaderVariant::Union {
                                if entry.union[format_i].is_none() {
                                    // The union-shaped module and pipeline
                                    // build at the first union use — wasm
                                    // fills them ahead of encode in
                                    // `prepare_union_pipelines`.
                                    #[cfg(not(target_arch = "wasm32"))]
                                    {
                                        let module = entry.union_module.get_or_insert_with(|| {
                                            self.device.create_shader_module(
                                                wgpu::ShaderModuleDescriptor {
                                                    label: Some("backdrop effect (union)"),
                                                    source: wgpu::ShaderSource::Wgsl(
                                                        (&*entry.union_source).into(),
                                                    ),
                                                },
                                            )
                                        });
                                        entry.union[format_i] =
                                            Some(
                                                create_pipeline(
                                                    &self.device,
                                                    &self.config,
                                                    &self.pipeline_layout,
                                                    self.pipeline_cache.as_ref(),
                                                    module,
                                                    target_format,
                                                    CoveragePass::Painter(false),
                                                )
                                                .map_err(|e| {
                                                    RenderError::Render(format!(
                                                        "backdrop shader {id}: {e}"
                                                    ))
                                                })?,
                                            );
                                    }
                                    #[cfg(target_arch = "wasm32")]
                                    return Err(RenderError::Render(format!(
                                        "backdrop shader {id}'s union pipeline was not prepared"
                                    )));
                                }
                                entry.union[format_i]
                                    .as_ref()
                                    .expect("union effect pipeline ready")
                            } else {
                                entry.plain[format_i]
                                    .as_ref()
                                    .expect("plain effect pipelines are built at registration")
                            }
                        }
                        kind => core_pipeline(
                            &mut self.pipelines,
                            &factory,
                            target_format,
                            kind == PipelineKind::Replace,
                            variant,
                        )?,
                    };
                    render_pass.set_pipeline(pipe);
                }
                let key = (
                    range.source,
                    scratch_backdrop,
                    range.image.clone(),
                    range.mask,
                );
                let bind = match surf.binds1.entry(key) {
                    std::collections::hash_map::Entry::Occupied(e) => &*e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        stats.bind_groups_created += 1;
                        let backdrop = backdrop_slot
                            .and_then(|slot| surf.backdrop[slot].as_ref().map(|b| &b.view));
                        &*e.insert(make_bind1(
                            &self.device,
                            &self.layout1,
                            &self.dummy_view,
                            range.source.map(|s| match s {
                                Source::Scratch(i) => &surf.scratch[&i].view,
                                Source::Backdrop { group, region } => surf.backdrop_groups[&group]
                                    .captures[region as usize]
                                    .sample_view(),
                                Source::Projected(_) => {
                                    unreachable!("local images draw with the projective pipeline")
                                }
                            }),
                            backdrop,
                            range.image.as_ref().map(|image| {
                                image_view(
                                    image,
                                    &self.images,
                                    &self.bitmaps,
                                    &surf.shader_textures,
                                )
                            }),
                            range.mask.map(|k| {
                                self.atlas
                                    .mask_texture_view(k)
                                    .expect("mask texture stored before encode")
                            }),
                        ))
                    }
                };
                render_pass.set_bind_group(1, bind, &[]);
                if coverage_order {
                    // Four vertex shader runs per quad, but the indices
                    // submit the painter path's exact ordered triangles —
                    // required for bit-identical rasterization.
                    render_pass.draw_indexed(
                        0..6,
                        0,
                        (inst_base + range.instances.start)..(inst_base + range.instances.end),
                    );
                } else {
                    render_pass.draw(
                        0..6,
                        (inst_base + range.instances.start)..(inst_base + range.instances.end),
                    );
                }
                ri += 1;
            }
            drop(render_pass);
            // A staged resolve runs once the looked-through composites
            // from the painted levels landed.
            if let Some(ResolveDraw {
                pipeline,
                bind,
                staged: Some((capture, size)),
            }) = &resolve_draw
            {
                let mut resolve_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("backdrop resolve"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: capture,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let offset = (surf.globals_base + u32::try_from(i).unwrap()) * 256;
                resolve::draw(&mut resolve_pass, pipeline, bind, offset, *size);
                drop(resolve_pass);
                stats.passes += 1;
            }
            if let Some((_, parameters)) = surf.frame.shadows.iter().find(|(pass, _)| *pass == i) {
                let Target::Scratch(depth) = pass.target else {
                    unreachable!("shadow captures scratch")
                };
                // First shadow of the engine's life builds the blur
                // pipeline here (#170).
                self.shadow_blur
                    .get_or_insert_with(|| shadow::Blur::new(&self.device, self.scratch_format))
                    .apply(
                        &self.device,
                        &mut encoder,
                        &surf.scratch[&depth],
                        (pass.region[2], pass.region[3]),
                        *parameters,
                    )?;
                stats.passes +=
                    u32::from(parameters.spread != 0.0) + 2 * u32::from(parameters.sigma > 0.0);
            }
            if let Some((_, filter)) = surf.frame.filters.iter().find(|(pass, _)| *pass == i) {
                let capture = match pass.target {
                    Target::Scratch(depth) => &surf.scratch[&depth],
                    Target::Backdrop { group, region } => {
                        &surf.backdrop_groups[&group].captures[region as usize].target
                    }
                    Target::Part(_) | Target::Projected(_) | Target::Plane(_) => {
                        return Err(RenderError::Render(format!(
                            "filter {filter:?} registered on a surface pass"
                        )));
                    }
                };
                self.filters.apply(
                    *filter,
                    &filtrate::EffectContext {
                        device: &self.device,
                        queue: &self.queue,
                        input_format: TARGET_FORMAT,
                        output_format: TARGET_FORMAT,
                    },
                    capture,
                    (pass.region[2], pass.region[3]),
                    timing,
                    &mut encoder,
                )?;
            }
            // A levelled group's pyramid: each level reduces the filtered
            // level above by an exact 2×2 box, appended in pass and level
            // order after `write_uploads`' pass slots.
            if let Some(capture) = pass.capture {
                let levels = capture.levels;
                if levels > 1 {
                    let pipelines = self.reduce.get_or_insert_with(|| {
                        reduce::Pipelines::new(&self.device, self.shader_delivery)
                    });
                    let target = &mut surf
                        .backdrop_groups
                        .get_mut(&capture.group)
                        .expect("registered group")
                        .captures[capture.region as usize];
                    let pipeline = pipelines
                        .pipeline(&self.device, target.target.texture.format())
                        .clone();
                    for k in 1..levels {
                        let size = (
                            pass.region[2].div_ceil(1 << k),
                            pass.region[3].div_ceil(1 << k),
                        );
                        let source = if k == 1 {
                            &target.target.view
                        } else {
                            &target.levels[(k - 2) as usize]
                        };
                        let bind = pipelines
                            .bind(
                                &self.device,
                                &mut target.reduces[(k - 1) as usize],
                                &self.globals,
                                source,
                            )
                            .clone();
                        let mut level_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("backdrop level"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &target.levels[(k - 1) as usize],
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        let offset = (surf.globals_base
                            + u32::try_from(surf.frame.passes.len()).unwrap_or(u32::MAX)
                            + reduce_slot)
                            * 256;
                        reduce::draw(&mut level_pass, &pipeline, &bind, offset, size);
                        drop(level_pass);
                        reduce_slot += 1;
                        stats.passes += 1;
                    }
                }
            }
            for (_, key) in surf.frame.mips.iter().filter(|(pass, _)| *pass == i) {
                let pipes = self
                    .projective
                    .as_ref()
                    .expect("projective pipelines are built before encoding");
                let entry = projective_entry(&surf.projective, *key);
                entry.build_mips(&mut encoder, pipes);
                stats.passes += u32::try_from(entry.mips.len()).unwrap_or(u32::MAX);
            }
            stats.passes += 1;
            i += 1;
        }
        #[cfg(target_vendor = "apple")]
        surf.tile_targets.finish_frame();
        // Producer sync: each external frame's `wait` event becomes a
        // raw Metal command buffer committed ahead of the frame's, so the
        // GPU blocks in-queue — no CPU wait and no copy. Commit order on
        // the queue orders the wait before wgpu's own command buffer.
        #[cfg(target_vendor = "apple")]
        {
            use objc2_metal::{MTLCommandBuffer as _, MTLCommandQueue as _};
            for id in &surf.frame.external {
                let Some(crate::interop::FrameSync::Metal { event, value }) = self
                    .producers
                    .get(id)
                    .and_then(gpu_content::Producer::current)
                    .and_then(|slot| slot.frame.wait.as_ref())
                else {
                    continue;
                };
                // SAFETY: `self.queue` is the engine's own queue; Metal
                // is asserted on the next line.
                let hal_queue = unsafe { self.queue.as_hal::<wgpu::hal::metal::Api>() }
                    .expect("the engine queue is Metal");
                let buffer = hal_queue
                    .as_raw()
                    .commandBuffer()
                    .expect("Metal command buffer");
                buffer.encodeWaitForEvent_value(event.as_ref(), *value);
                buffer.commit();
            }
        }
        // Retired generations' release barriers join this encoder; their
        // signal semaphores and the staged producer waits register on the
        // queue immediately before the consuming submission, under the
        // submit guard (#166).
        #[cfg(all(unix, not(target_vendor = "apple")))]
        if let Some(native) = self.native.as_mut() {
            let releases = external::vulkan::drain_releases(native);
            if !releases.is_empty() {
                buffers.push(split_encoder(&mut encoder, &self.device));
                let mut release_cb =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("release"),
                        });
                // SAFETY: `release_cb` is a live Vulkan encoder; its `cb`
                // is recording while the release barriers are encoded.
                unsafe {
                    release_cb.as_hal_mut::<wgpu::hal::vulkan::Api, _, _>(|hal| {
                        let hal = hal.expect("vulkan encoder");
                        let cb = hal.raw_handle();
                        for release in &releases {
                            release.encode_barrier(&native.shared, cb);
                        }
                    });
                }
                buffers.push(release_cb.finish());
                native.releases = releases;
            }
        }
        let submission = {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            let _submit = self.submit_lock.lock().expect("submit guard");
            // The staging copies precede every other submission this
            // frame — including the external-frame semaphore pokes they
            // must not consume — so the upload slot's map resolves as
            // soon as the copies retire instead of after the passes.
            let upload_submission = upload_buffer.map(|buffer| {
                let submission = self.queue.submit([buffer]);
                track_submission(&self.queue, &self.tracker);
                diag::submit(&self.device, &self.queue, "upload staging");
                submission
            });
            #[cfg(all(unix, not(target_vendor = "apple")))]
            if let Some(native) = self.native.as_mut()
                && (!native.staged.is_empty() || !native.releases.is_empty())
            {
                let hal_queue =
                    // SAFETY: `self.queue` is the engine's own queue;
                    // Vulkan is asserted by `expect`.
                    unsafe { self.queue.as_hal::<wgpu::hal::vulkan::Api>() }.expect("vulkan queue");
                external::vulkan::submit_waits(native, &hal_queue);
                for release in &native.releases {
                    for (semaphore, value) in release.signals() {
                        hal_queue.add_signal_semaphore(semaphore, value);
                    }
                }
            }
            buffers.push(encoder.finish());
            let submission = self.queue.submit(buffers);
            track_submission(&self.queue, &self.tracker);
            #[cfg(all(unix, not(target_vendor = "apple")))]
            if let Some(native) = self.native.as_mut()
                && !native.staged.is_empty()
            {
                external::vulkan::mark_submitted(native);
            }
            #[cfg(all(unix, not(target_vendor = "apple")))]
            if let Some(native) = self.native.as_mut()
                && !native.acquiring.is_empty()
            {
                // The consuming submission is accepted; states promote to
                // `OwnedForRead` when the queue reports it complete.
                let acquired = std::mem::take(&mut native.acquiring);
                self.queue
                    .on_submitted_work_done(move || external::vulkan::mark_owned(acquired));
            }
            #[cfg(all(unix, not(target_vendor = "apple")))]
            if let Some(native) = self.native.as_mut() {
                // Bounded-cache evictions are destroyed once every
                // submission that could still reference them completes.
                let destroys = external::vulkan::drain_destroys(native);
                if !destroys.is_empty() {
                    let device = native.shared.vk.device.clone();
                    self.queue.on_submitted_work_done(move || {
                        for item in destroys {
                            // SAFETY: `item` was queued for deferred
                            // destroy and runs once every referencing
                            // submission completed — the contract the
                            // queue callback fires under.
                            unsafe { item.destroy(&device) };
                        }
                    });
                }
            }
            #[cfg(all(unix, not(target_vendor = "apple")))]
            if let Some(native) = self.native.as_mut()
                && !native.releases.is_empty()
            {
                // The release submission is accepted: exportable fences
                // are exported now — while their signal is still pending
                // — and destruction runs when the queue reports the
                // submission complete — never a CPU wait.
                let shared = native.shared.clone();
                let mut releases = std::mem::take(&mut native.releases);
                for release in &mut releases {
                    release.export_fence(&shared);
                    if let Some(flag) = &release.submitted_flag {
                        flag.store(true, std::sync::atomic::Ordering::Release);
                    }
                    if let Some(state) = &release.state {
                        *state.lock().expect("generation state") =
                            external::vulkan::State::ReleaseSubmitted;
                    }
                }
                self.queue.on_submitted_work_done(move || {
                    for release in releases {
                        release.destroy(&shared);
                    }
                });
            }
            (upload_submission, submission)
        };
        let (upload_submission, submission) = submission;
        if let Some(upload_submission) = upload_submission {
            self.uploads.submitted(upload_submission);
        }
        diag::submit(&self.device, &self.queue, "frame");
        self.frame_submission = Some(submission.clone());
        tracing::trace!(
            surface = ?id,
            passes = surf.frame.passes.len(),
            instances = surf.frame.instances.len(),
            ?submission,
            "surface submitted"
        );
        stats.instances += u32::try_from(surf.frame.instances.len()).unwrap_or(u32::MAX);
        Ok(())
    }

    /// Waits for a submission's completion — a deadline on GPU progress,
    /// not on the wait itself. Each `wait_timeout` window that elapses is
    /// judged against [`SubmissionTracker`]: any submission retiring inside
    /// the window proves a slow adapter is still draining, so the wait
    /// warns and opens another window; a whole window with zero retirements
    /// is a wedged queue and fails as [`RenderError::Timeout`]. Device loss
    /// still surfaces immediately as [`RenderError::DeviceLost`].
    #[cfg(not(target_arch = "wasm32"))]
    fn wait(
        &self,
        submission: &wgpu::SubmissionIndex,
        what: &'static str,
    ) -> Result<(), RenderError> {
        wait_with_progress(
            what,
            self.wait_timeout,
            submission,
            || {
                let status = self.device.poll(wgpu::PollType::Wait {
                    submission_index: Some(submission.clone()),
                    timeout: Some(self.wait_timeout),
                });
                diag::poll(&self.device, status.is_ok());
                status
            },
            || self.tracker.retired(),
        )
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn wait(
        &self,
        _submission: wgpu::SubmissionIndex,
        what: &'static str,
    ) -> Result<(), RenderError> {
        let (tx, rx) = futures_channel::oneshot::channel();
        self.queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });
        browser_wait(rx, self.wait_timeout, what).await
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn map_read(
        &self,
        slice: wgpu::BufferSlice<'_>,
        submission: &wgpu::SubmissionIndex,
        what: &'static str,
    ) -> Result<(), RenderError> {
        let (tx, rx) = std::sync::mpsc::channel();
        tracing::trace!(what, "map requested");
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        diag::map(&self.device, what, 0);
        self.wait(submission, what)?;
        match rx.try_recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(RenderError::Readback(format!("{what}: map failed: {e}"))),
            Err(_) => Err(RenderError::Readback(format!(
                "{what}: the map callback did not run after the wait"
            ))),
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn map_read(
        &self,
        slice: wgpu::BufferSlice<'_>,
        _submission: wgpu::SubmissionIndex,
        what: &'static str,
    ) -> Result<(), RenderError> {
        let (tx, rx) = futures_channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        browser_wait(rx, self.wait_timeout, what)
            .await?
            .map_err(|error| RenderError::Readback(format!("{what}: {error}")))
    }

    /// Hands this frame's samples to the completion queue without waiting.
    fn queue_timestamps(&mut self, count: u32, frame: FrameId) {
        let query_set = self.query_set.take().expect("a timed frame owns queries");
        let submission = self.frame_submission.clone().expect("a timed frame drew");
        let complete = Arc::new(AtomicU8::new(0));
        let flag = Arc::clone(&complete);
        self.queue.on_submitted_work_done(move || {
            flag.store(1, Ordering::Release);
        });
        self.pending_queries.push_back(PendingQueries {
            frame,
            submission,
            query_set,
            base: self.query_base,
            capacity: self.query_capacity,
            count,
            meta: std::mem::take(&mut self.pass_meta),
            complete,
        });
    }

    /// Submits a pending external-frame retirement even when nothing else
    /// is being drawn — an idle engine still releases the lease (#166).
    /// Returns the release submission when one was made.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn flush_native_releases(&mut self) -> Option<wgpu::SubmissionIndex> {
        let native = self.native.as_mut()?;
        let mut releases = external::vulkan::drain_releases(native);
        if releases.is_empty() {
            return None;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("external frame release"),
            });
        // SAFETY: `encoder` is a live Vulkan encoder; its `cb` is
        // recording while the release barriers are encoded.
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::vulkan::Api, _, _>(|hal| {
                let hal = hal.expect("vulkan encoder");
                let cb = hal.raw_handle();
                for release in &releases {
                    release.encode_barrier(&native.shared, cb);
                }
            });
        }
        let submission = {
            let _submit = self.submit_lock.lock().expect("submit guard");
            // SAFETY: `self.queue` is the engine's own queue; Vulkan is
            // asserted by `expect`.
            let hal_queue =
                unsafe { self.queue.as_hal::<wgpu::hal::vulkan::Api>() }.expect("vulkan queue");
            for release in &releases {
                for (semaphore, value) in release.signals() {
                    hal_queue.add_signal_semaphore(semaphore, value);
                }
            }
            self.queue.submit([encoder.finish()])
        };
        track_submission(&self.queue, &self.tracker);
        let shared = native.shared.clone();
        for release in &mut releases {
            // Same ordering as the frame submit path: export while the
            // release signal is still pending, before destruction.
            release.export_fence(&shared);
            if let Some(flag) = &release.submitted_flag {
                flag.store(true, std::sync::atomic::Ordering::Release);
            }
            if let Some(state) = &release.state {
                *state.lock().expect("generation state") =
                    external::vulkan::State::ReleaseSubmitted;
            }
        }
        self.queue.on_submitted_work_done(move || {
            for release in releases {
                release.destroy(&shared);
            }
        });
        let destroys = external::vulkan::drain_destroys(native);
        if !destroys.is_empty() {
            let device = native.shared.vk.device.clone();
            self.queue.on_submitted_work_done(move || {
                for item in destroys {
                    // SAFETY: `item` was queued for deferred destroy and
                    // runs once every referencing submission completed —
                    // the contract the queue callback fires under.
                    unsafe { item.destroy(&device) };
                }
            });
        }
        Some(submission)
    }

    /// Encodes the resolve only once the frame's samples are complete.
    fn resolve_timestamps(&mut self, pending: PendingQueries) {
        let staging = self.timestamp_staging();
        let buf = self
            .query_buffer
            .as_ref()
            .expect("timing has a resolve buffer");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("timestamp resolve"),
            });
        encoder.resolve_query_set(
            &pending.query_set,
            pending.base..pending.base + pending.count,
            buf,
            0,
        );
        encoder.copy_buffer_to_buffer(buf, 0, &staging, 0, u64::from(pending.count) * 8);
        let submission = self.queue.submit([encoder.finish()]);
        track_submission(&self.queue, &self.tracker);
        diag::submit(&self.device, &self.queue, "timestamp resolve");
        tracing::trace!(
            frame = pending.frame.get(),
            count = pending.count,
            ?submission,
            "timestamps resolved after draw completion"
        );
        self.pending_timestamps.push_back(PendingTimestamps {
            query_set: pending.query_set,
            query_base: pending.base,
            query_capacity: pending.capacity,
            frame: pending.frame,
            submission,
            staging,
            count: pending.count,
            meta: pending.meta,
            map_requested: false,
            ready: Arc::new(AtomicU8::new(0)),
        });
    }

    fn timestamp_staging(&mut self) -> wgpu::Buffer {
        let size = u64::from(self.query_capacity) * 8;
        self.query_staging.retain(|buffer| buffer.size() >= size);
        self.query_staging.pop().unwrap_or_else(|| {
            diag::create(&self.device, "timestamp staging", size);
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp staging"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        })
    }

    /// Reads back every pending frame's resolved timestamps whose copy
    /// has landed, oldest first — submissions complete in order, so the
    /// first unfinished one ends the drain. Never blocks.
    fn drain_timestamps(&mut self) {
        if !self.pending_queries.is_empty() {
            // Poll dispatches completion callbacks; it never waits for GPU idle.
            let _ = self.device.poll(wgpu::PollType::Poll);
            diag::poll(&self.device, false);
            while self
                .pending_queries
                .front()
                .is_some_and(|pending| pending.complete.load(Ordering::Acquire) != 0)
            {
                let pending = self.pending_queries.pop_front().expect("checked above");
                self.resolve_timestamps(pending);
            }
        }
        let period = f64::from(self.queue.get_timestamp_period());
        while let Some(pending) = self.pending_timestamps.front_mut() {
            pending.request_map();
            diag::map(
                &self.device,
                "timestamp staging",
                u64::from(pending.count) * 8,
            );
            if self.device.poll(wgpu::PollType::Poll).is_err() {
                break;
            }
            diag::poll(&self.device, false);
            match pending.ready.load(Ordering::Relaxed) {
                // A failed map drops the frame's timing instead of
                // blocking every later drain.
                2 => {
                    tracing::error!(
                        frame = pending.frame.get(),
                        "the timestamp readback map failed"
                    );
                    if let Some(pending) = self.pending_timestamps.pop_front() {
                        pending.staging.unmap();
                    }
                    continue;
                }
                1 => {}
                _ => break,
            }
            let Some(pending) = self.pending_timestamps.pop_front() else {
                break;
            };
            let timing = {
                let data = pending
                    .staging
                    .slice(..u64::from(pending.count) * 8)
                    .get_mapped_range()
                    .expect("buffer range is mapped and not overlapping");
                let ticks: &[u64] = bytemuck::cast_slice(&data);
                tracing::trace!(
                    frame = pending.frame.get(),
                    period,
                    ?ticks,
                    "timestamp ticks"
                );
                #[expect(clippy::cast_precision_loss)]
                let delta = |from: usize, to: usize| {
                    ticks
                        .get(to)
                        .zip(ticks.get(from))
                        .filter(|(end, start)| end > start)
                        .map(|(end, start)| period * (end - start) as f64 * 1e-9)
                };
                FrameTiming {
                    frame: pending.frame,
                    // The frame's GPU time runs from the first pass's
                    // start to the last pass's end.
                    gpu_seconds: delta(0, pending.count as usize - 1),
                    passes: pending
                        .meta
                        .into_iter()
                        .enumerate()
                        .map(|(i, meta)| PassTiming {
                            name: meta.name,
                            width: meta.width,
                            height: meta.height,
                            format: meta.format,
                            gpu_seconds: delta(2 * i, 2 * i + 1),
                        })
                        .collect(),
                }
            };
            tracing::debug!(
                frame = timing.frame.get(),
                passes = timing.passes.len(),
                gpu_ms = timing.gpu_seconds.map(|s| s * 1e3),
                "frame timed"
            );
            self.timings.push(timing);
            pending.staging.unmap();
            if pending.query_capacity >= self.query_capacity {
                self.query_pool.push((
                    pending.query_set,
                    pending.query_base,
                    pending.query_capacity,
                ));
            }
            if pending.staging.size() >= u64::from(self.query_capacity) * 8 {
                self.query_staging.push(pending.staging);
            }
        }
    }

    /// Acquires a frame's queries and grows the resolve buffer if needed.
    /// All surfaces are lowered before encoding, so replacing an undersized
    /// active set here cannot discard samples already written this frame.
    fn ensure_query_capacity(&mut self, queries: u32) {
        if queries <= self.query_capacity && self.query_set.is_some() {
            return;
        }
        let capacity = queries.next_power_of_two().max(2).max(self.query_capacity);
        if let Some(index) = self
            .query_pool
            .iter()
            .position(|(_, _, size)| *size == capacity)
        {
            let (set, base, _) = self.query_pool.swap_remove(index);
            self.query_set = Some(set);
            self.query_base = base;
        } else {
            let slots = (wgpu::QUERY_SET_MAX_QUERIES / capacity).clamp(1, TIMESTAMP_FRAMES_PER_SET);
            let set = self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: capacity * slots,
            });
            diag::create(&self.device, "frame timestamps", 0);
            self.query_pool
                .extend((1..slots).map(|slot| (set.clone(), slot * capacity, capacity)));
            self.query_set = Some(set);
            self.query_base = 0;
            tracing::debug!(capacity, slots, "timestamp query ranges allocated");
        }
        if capacity > self.query_capacity {
            let old = self.query_buffer.as_ref().map_or(0, wgpu::Buffer::size);
            self.query_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp resolve"),
                size: u64::from(capacity) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
            diag::grow(
                &self.device,
                "timestamp resolve",
                diag::Class::Query,
                old,
                u64::from(capacity) * 8,
                0,
                true,
            );
            self.query_capacity = capacity;
            self.query_pool.retain(|(_, _, size)| *size >= capacity);
        }
    }
}

/// A registered image's texture of `size`, holding `data` (f16 texels
/// from [`image_texels_f16`]).
fn upload_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height): (u32, u32),
    data: &[u8],
) -> GpuImage {
    let (texture, view) = create_target(
        device,
        "image",
        (width, height),
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        TARGET_FORMAT,
    );
    diag::create(
        device,
        "image",
        u64::from(width) * u64::from(height) * texel_bytes(TARGET_FORMAT),
    );
    write_image(device, queue, &texture, (width, height), data);
    GpuImage {
        texture,
        view,
        width,
        height,
    }
}

/// Writes `data` (f16 texels from [`image_texels_f16`]) over the whole of
/// an image texture of `size`.
fn write_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
    data: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    diag::upload(
        device,
        "image",
        data.len() as u64,
        Some((0, 0, width, height)),
    );
}

/// `Rgba8` or `Rgba16F` upload bytes -> premultiplied linear-P3 f16 texels,
/// the working texel format of [`GpuImage`].
///
/// Straight-alpha input decodes each channel; premultiplied input is
/// un-premultiplied in the encoded domain first — bounded at 1.0 for the
/// quantized `Rgba8` encoding, unbounded for `Rgba16F`, whose texels keep
/// extended (HDR and wide-gamut) values.
fn image_texels_f16(image: &ImageUpload) -> Result<Vec<u8>, ResourceError> {
    let convert = |enc: [f64; 4], unpremul_max: f64, data: &mut Vec<u8>| {
        let a = enc[3];
        // Straight-alpha input decodes each channel; premultiplied input
        // is un-premultiplied in the encoded domain first.
        let decode = |v: f64| {
            if image.premultiplied && a > 0.0 {
                (v / a).min(unpremul_max)
            } else {
                v
            }
        };
        let lin = match image.color_space {
            cherenkov::ImageColorSpace::LinearSrgb | cherenkov::ImageColorSpace::LinearP3 => {
                [decode(enc[0]), decode(enc[1]), decode(enc[2])]
            }
            _ => [
                srgb_decode_u8_f64(decode(enc[0])),
                srgb_decode_u8_f64(decode(enc[1])),
                srgb_decode_u8_f64(decode(enc[2])),
            ],
        };
        // sRGB-primaries input additionally needs the primaries' matrix;
        // Display P3 uses sRGB's transfer function, so the decode above
        // covers both encoded spaces. `LinearP3` is already the working
        // space: no transfer, no matrix.
        let lin_p3 = match image.color_space {
            cherenkov::ImageColorSpace::Srgb | cherenkov::ImageColorSpace::LinearSrgb => {
                let [x, y, z] = [
                    SRGB_TO_XYZ[0][2].mul_add(
                        lin[2],
                        SRGB_TO_XYZ[0][1].mul_add(lin[1], SRGB_TO_XYZ[0][0] * lin[0]),
                    ),
                    SRGB_TO_XYZ[1][2].mul_add(
                        lin[2],
                        SRGB_TO_XYZ[1][1].mul_add(lin[1], SRGB_TO_XYZ[1][0] * lin[0]),
                    ),
                    SRGB_TO_XYZ[2][2].mul_add(
                        lin[2],
                        SRGB_TO_XYZ[2][1].mul_add(lin[1], SRGB_TO_XYZ[2][0] * lin[0]),
                    ),
                ];
                [
                    XYZ_TO_P3[0][2].mul_add(z, XYZ_TO_P3[0][1].mul_add(y, XYZ_TO_P3[0][0] * x)),
                    XYZ_TO_P3[1][2].mul_add(z, XYZ_TO_P3[1][1].mul_add(y, XYZ_TO_P3[1][0] * x)),
                    XYZ_TO_P3[2][2].mul_add(z, XYZ_TO_P3[2][1].mul_add(y, XYZ_TO_P3[2][0] * x)),
                ]
            }
            cherenkov::ImageColorSpace::DisplayP3 | cherenkov::ImageColorSpace::LinearP3 => lin,
        };
        for v in [a * lin_p3[0], a * lin_p3[1], a * lin_p3[2], a] {
            data.extend_from_slice(&half::f16::from_f64(v).to_le_bytes());
        }
    };
    let mut data = Vec::with_capacity(image.width as usize * image.height as usize * 8);
    let pixels: &[u8] = &image.data;
    match image.format {
        cherenkov::ImageFormat::Rgba8 => {
            for px in pixels.as_chunks::<4>().0 {
                convert(px.map(|v| f64::from(v) / 255.0), 1.0, &mut data);
            }
        }
        cherenkov::ImageFormat::Rgba16F => {
            for px in pixels.as_chunks::<8>().0 {
                let enc = std::array::from_fn(|i| {
                    f64::from(half::f16::from_le_bytes([px[2 * i], px[2 * i + 1]]))
                });
                convert(enc, f64::INFINITY, &mut data);
            }
        }
        format => {
            return Err(ResourceError::Image(format!(
                "unsupported image format {format:?}"
            )));
        }
    }
    Ok(data)
}

fn image_texels(
    pixels: &[u8],
    color_space: cherenkov::ImageColorSpace,
    premultiplied: bool,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(pixels.len() * 2);
    for px in pixels.as_chunks::<4>().0 {
        let a = f64::from(px[3]) / 255.0;
        let decode = |v: u8| {
            if premultiplied && a > 0.0 {
                ((f64::from(v) / 255.0) / a).min(1.0)
            } else {
                f64::from(v) / 255.0
            }
        };
        let lin = match color_space {
            cherenkov::ImageColorSpace::LinearSrgb | cherenkov::ImageColorSpace::LinearP3 => {
                [decode(px[0]), decode(px[1]), decode(px[2])]
            }
            _ => [
                srgb_decode_u8_f64(decode(px[0])),
                srgb_decode_u8_f64(decode(px[1])),
                srgb_decode_u8_f64(decode(px[2])),
            ],
        };
        let lin_p3 = match color_space {
            cherenkov::ImageColorSpace::Srgb | cherenkov::ImageColorSpace::LinearSrgb => {
                let [x, y, z] = [
                    SRGB_TO_XYZ[0][2].mul_add(
                        lin[2],
                        SRGB_TO_XYZ[0][1].mul_add(lin[1], SRGB_TO_XYZ[0][0] * lin[0]),
                    ),
                    SRGB_TO_XYZ[1][2].mul_add(
                        lin[2],
                        SRGB_TO_XYZ[1][1].mul_add(lin[1], SRGB_TO_XYZ[1][0] * lin[0]),
                    ),
                    SRGB_TO_XYZ[2][2].mul_add(
                        lin[2],
                        SRGB_TO_XYZ[2][1].mul_add(lin[1], SRGB_TO_XYZ[2][0] * lin[0]),
                    ),
                ];
                [
                    XYZ_TO_P3[0][2].mul_add(z, XYZ_TO_P3[0][1].mul_add(y, XYZ_TO_P3[0][0] * x)),
                    XYZ_TO_P3[1][2].mul_add(z, XYZ_TO_P3[1][1].mul_add(y, XYZ_TO_P3[1][0] * x)),
                    XYZ_TO_P3[2][2].mul_add(z, XYZ_TO_P3[2][1].mul_add(y, XYZ_TO_P3[2][0] * x)),
                ]
            }
            cherenkov::ImageColorSpace::DisplayP3 | cherenkov::ImageColorSpace::LinearP3 => lin,
        };
        for v in [a * lin_p3[0], a * lin_p3[1], a * lin_p3[2], a] {
            data.extend_from_slice(&half::f16::from_f64(v).to_le_bytes());
        }
    }
    data
}

fn create_gpu_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    width: u32,
    height: u32,
    texels: &[u8],
) -> GpuImage {
    let (texture, view) = create_target(
        device,
        label,
        (width, height),
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        TARGET_FORMAT,
    );
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
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    GpuImage {
        texture,
        view,
        width,
        height,
    }
}

fn srgb_decode_u8_f64(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// A browser completion with the same configured timeout as native waits.
#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn browser_wait<T>(
    rx: futures_channel::oneshot::Receiver<T>,
    timeout: std::time::Duration,
    what: &'static str,
) -> Result<T, RenderError> {
    use futures_util::future::{Either, select};
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    match select(
        Box::pin(rx),
        Box::pin(gloo_timers::future::TimeoutFuture::new(millis)),
    )
    .await
    {
        Either::Left((Ok(value), _)) => Ok(value),
        Either::Left((Err(_), _)) => Err(RenderError::DeviceLost),
        Either::Right(_) => Err(RenderError::Timeout { what, timeout }),
    }
}

#[cfg(target_arch = "wasm32")]
impl GpuRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn prepare_filters(&mut self, surface: SurfaceId) -> Result<(), RenderError> {
        let surface = &self.surfaces[&surface];
        let uses: Vec<_> = surface
            .frame
            .filters
            .iter()
            .map(|(pass, id)| {
                let format = match surface.frame.passes[*pass].target {
                    Target::Scratch(depth) => surface.scratch[&depth].texture.format(),
                    Target::Backdrop { group, region } => surface.backdrop_groups[&group].captures
                        [region as usize]
                        .target
                        .texture
                        .format(),
                    Target::Part(_) | Target::Projected(_) | Target::Plane(_) => TARGET_FORMAT,
                };
                (*id, format)
            })
            .collect();
        for (id, format) in uses {
            self.filters
                .prepare(
                    id,
                    &filtrate::EffectContext {
                        device: &self.device,
                        queue: &self.queue,
                        input_format: format,
                        output_format: format,
                    },
                )
                .await?;
        }
        Ok(())
    }

    /// wasm: the variant-3 pipelines a frame's lowered passes need —
    /// core `(format, replace)` cells for `ShaderVariant::Union` ranges
    /// and the union-shaped slots of any backdrop effect they draw —
    /// prepared ahead of encode, which cannot await pipeline creation.
    /// The union engine module materializes here too, on the first
    /// frame that actually draws a union or outer-band member.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn prepare_union_pipelines(&mut self, surface: SurfaceId) -> Result<(), RenderError> {
        let surface = &self.surfaces[&surface];
        // (format index, replace) core cells and (shader id, format
        // index) effect slots the frame draws through the union variant.
        let mut core = FxHashSet::default();
        let mut effects = FxHashSet::default();
        for pass in &surface.frame.passes {
            let format = match pass.target {
                Target::Scratch(depth) => surface.scratch[&depth].texture.format(),
                Target::Backdrop { group, region } => surface.backdrop_groups[&group].captures
                    [region as usize]
                    .target
                    .texture
                    .format(),
                Target::Part(_) | Target::Projected(_) | Target::Plane(_) => TARGET_FORMAT,
            };
            let format_i = usize::from(format != TARGET_FORMAT);
            for range in &pass.ranges {
                if range.variant == ShaderVariant::Union {
                    if let PipelineKind::Effect(id) = range.pipeline {
                        effects.insert((id, format_i));
                    } else {
                        core.insert((format_i, range.pipeline == PipelineKind::Replace));
                    }
                }
            }
        }
        if core.is_empty() && effects.is_empty() {
            return Ok(());
        }
        let module = self
            .union_module
            .get_or_insert_with(|| self.shader_delivery.engine_module(&self.device, 3));
        for (format_i, replace) in core {
            let cell = &mut self.pipelines[format_i][usize::from(replace)][3];
            if cell.is_none() {
                *cell = Some(
                    create_pipeline(
                        &self.device,
                        &self.config,
                        &self.pipeline_layout,
                        self.pipeline_cache.as_ref(),
                        module,
                        if format_i == 0 {
                            TARGET_FORMAT
                        } else {
                            self.scratch_format
                        },
                        CoveragePass::Painter(replace),
                    )
                    .await
                    .map_err(|e| RenderError::Render(e.to_string()))?,
                );
            }
        }
        for (id, format_i) in effects {
            let Some(entry) = self.backdrop_shaders.get_mut(&id) else {
                continue;
            };
            if entry.union[format_i].is_none() {
                let module = entry.union_module.get_or_insert_with(|| {
                    self.device
                        .create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("backdrop effect (union)"),
                            source: wgpu::ShaderSource::Wgsl((&*entry.union_source).into()),
                        })
                });
                entry.union[format_i] = Some(
                    create_pipeline(
                        &self.device,
                        &self.config,
                        &self.pipeline_layout,
                        self.pipeline_cache.as_ref(),
                        module,
                        if format_i == 0 {
                            TARGET_FORMAT
                        } else {
                            self.scratch_format
                        },
                        CoveragePass::Painter(false),
                    )
                    .await
                    .map_err(|e| RenderError::Render(e.to_string()))?,
                );
            }
        }
        Ok(())
    }
}

/// Engine teardown: surface drops retire the last frame leases into the
/// device release queue, and a release only becomes safe to destroy once
/// the release submission completes — `release.destroy` runs on the
/// submission's completion callback, so shutdown submits the pending
/// releases and joins the queue once. A CPU wait at teardown is a host
/// join, not a producer fence wait; the no-CPU-wait contract covers
/// steady-state submissions only. Without this the imported images,
/// memories and semaphores — and the driver-held fds behind them — outlive
/// the engine until device destroy (#166 fd accounting).
#[cfg(all(unix, not(target_vendor = "apple")))]
impl Drop for GpuRenderer {
    fn drop(&mut self) {
        // Surfaces drop before fields: retire their frame leases while the
        // native context and queue still live.
        self.surfaces.clear();
        if self.native.is_some()
            && let Some(submission) = self.flush_native_releases()
        {
            // Completion callbacks run under the poll; a timeout leaves
            // the releases queued — the device drop reclaims the objects.
            drop(self.wait(&submission, "external frame teardown"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the window decision with scripted poll outcomes and a
    /// scripted retirement frontier — no device needed.
    fn drive(
        polls: impl IntoIterator<Item = Result<wgpu::PollStatus, wgpu::PollError>>,
        retired: impl IntoIterator<Item = u64>,
    ) -> Result<(), RenderError> {
        let mut polls = polls.into_iter();
        let mut retired = retired.into_iter();
        wait_with_progress::<u64>(
            "test wait",
            Duration::from_secs(30),
            &0,
            move || polls.next().expect("scripted poll"),
            move || retired.next().expect("scripted frontier"),
        )
    }

    #[test]
    fn wait_returns_when_the_submission_completes() {
        let outcome = drive([Ok(wgpu::PollStatus::WaitSucceeded)], [0]);
        assert!(outcome.is_ok());
    }

    #[test]
    fn progress_inside_a_window_keeps_the_wait_open() {
        // Two elapsed windows while the frontier still advances — a slow
        // but draining queue — then completion: the wait succeeds.
        let outcome = drive(
            [
                Err(wgpu::PollError::Timeout),
                Err(wgpu::PollError::Timeout),
                Ok(wgpu::PollStatus::WaitSucceeded),
            ],
            [0, 1, 2],
        );
        assert!(outcome.is_ok());
    }

    #[test]
    fn a_window_with_no_progress_times_out() {
        // The frontier never moves across the first elapsed window: the
        // queue is wedged and the wait fails instead of opening another.
        let outcome = drive(
            [
                Err(wgpu::PollError::Timeout),
                Ok(wgpu::PollStatus::WaitSucceeded),
            ],
            [0, 0],
        );
        assert!(matches!(
            outcome,
            Err(RenderError::Timeout {
                what: "test wait",
                ..
            })
        ));
    }

    #[test]
    fn poll_errors_other_than_timeout_still_fail_fast() {
        let outcome = drive([Err(wgpu::PollError::WrongSubmissionIndex(9, 3))], [0]);
        assert!(matches!(outcome, Err(RenderError::DeviceLost)));
    }
}
