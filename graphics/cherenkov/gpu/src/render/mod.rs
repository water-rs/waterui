//! The render thread: sole owner of GPU state.

mod bindings;
mod bitmap;
mod capabilities;
mod colr;
mod composite;
pub mod composition;
mod content_range;
mod damage;
pub mod diag;
#[cfg(target_os = "linux")]
pub mod dmabuf_export;
pub mod external;
pub mod filter;
mod glyph;
mod gpu_content;
mod gpu_cov;
#[cfg(any(target_vendor = "apple", target_os = "android"))]
mod headroom;
mod image_storage;
mod instance;
mod lower;
mod paint;
mod path;
mod repaint;
#[cfg(test)]
pub use path::CoverageRoute;
pub mod planes;
mod prepared;
pub mod present;
mod programs;
mod projective;
mod raster;
mod reduce;
mod resolve;
pub mod shaders;
mod shadow;
mod space_convert;
mod stroke_abi;
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
    FrameStats, FrameTiming, ImageId, ImageUpload, LayerId, MemoryUsage, OffscreenFormat,
    PassTiming, Pressure, ProducerId, Readback, RenderError, Renderer, ResourceError, ResourceId,
    SurfaceError, SurfaceFrame, SurfaceId, SurfaceInfo, Visibility,
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
    /// A coverage-order pass's partial-phase pipeline, from the cell
    /// module (`true`) for GPU-routed ranges or the simple one.
    Coverage(bool),
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

    /// The index of the pass's program in `programs::ENGINE`.
    const fn program(self) -> usize {
        match self {
            Self::Painter(_) => 0,
            Self::Coverage(CoveragePhase::Opaque) => 1,
            Self::Coverage(CoveragePhase::Partial) => 2,
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

/// The working format: premultiplied linear Display P3 in `Rgba16Float`.
/// Every surface stores it except an offscreen target that requests
/// [`OffscreenFormat::LinearF32`]; projected images, content and shader
/// paint textures, images and tile attachments always do.
const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The closed set of formats a pass renders into, by format slot: the
/// working format, linear/encoded 8-bit scratch formats and an
/// `Rgba32Float` offscreen target. Every format-keyed pipeline and texture array is
/// indexed by [`format_slot`], so surfaces and scratch layers sharing a
/// format share one cell.
const RENDER_FORMATS: [wgpu::TextureFormat; 4] = [
    WORKING_FORMAT,
    wgpu::TextureFormat::Rgba8Unorm,
    wgpu::TextureFormat::Rgba32Float,
    wgpu::TextureFormat::Rgba8UnormSrgb,
];

/// The slot of `format` in [`RENDER_FORMATS`].
///
/// # Panics
/// When `format` is not one the engine renders into: every pass target is
/// created in one of them, so another format is an engine bug.
const fn format_slot(format: wgpu::TextureFormat) -> usize {
    match format {
        wgpu::TextureFormat::Rgba16Float => 0,
        wgpu::TextureFormat::Rgba8Unorm => 1,
        wgpu::TextureFormat::Rgba32Float => 2,
        wgpu::TextureFormat::Rgba8UnormSrgb => 3,
        _ => panic!("the engine renders only into RENDER_FORMATS"),
    }
}

/// One cell per [`RENDER_FORMATS`] slot.
type FormatCells<T> = [T; RENDER_FORMATS.len()];

/// The device features an `Rgba32Float` surface needs: blending into it,
/// and filtered sampling of the captures and backdrop copies taken from it
/// (the engine's group-1 textures are filterable). Device creation enables
/// them wherever the adapter offers them.
fn float32_target_features() -> wgpu::Features {
    wgpu::Features::FLOAT32_BLENDABLE | wgpu::Features::FLOAT32_FILTERABLE
}

/// The storage format of a surface on `target`, or the error naming the
/// device features it lacks — never a substitute format. Only an
/// offscreen target chooses its format; every other target is handed to
/// a host or a compositor in the working format.
fn surface_format(
    device: &wgpu::Device,
    target: &GpuTarget,
) -> Result<wgpu::TextureFormat, SurfaceError> {
    let GpuTarget::Offscreen(offscreen) = target else {
        return Ok(WORKING_FORMAT);
    };
    match offscreen.format {
        OffscreenFormat::LinearF16 => Ok(WORKING_FORMAT),
        format @ OffscreenFormat::LinearF32 => {
            let missing = float32_target_features() & device.features().complement();
            if missing.is_empty() {
                Ok(wgpu::TextureFormat::Rgba32Float)
            } else {
                Err(SurfaceError::UnsupportedFormat {
                    format,
                    missing: missing.to_string(),
                })
            }
        }
    }
}

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

/// The row pitch of a readback buffer holding `width` texels of
/// `format`: the tight row rounded up to the copy alignment.
fn readback_pitch(format: wgpu::TextureFormat, width: u32) -> u32 {
    let texel = u32::try_from(texel_bytes(format)).expect("a texel size fits in u32");
    (width * texel).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// The pixels of a mapped readback of a `size` surface stored in
/// `format`, `pitch` bytes apart: f16 texels widen exactly, f32 texels
/// are taken as stored.
fn decode_readback(
    data: &[u8],
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    pitch: u32,
) -> Vec<[f32; 4]> {
    let tight = width as usize * usize::try_from(texel_bytes(format)).expect("a texel fits usize");
    let rows = (0..height).map(|row| {
        let start = (row * pitch) as usize;
        &data[start..start + tight]
    });
    match format {
        wgpu::TextureFormat::Rgba16Float => rows
            .flat_map(|row| row.as_chunks::<8>().0)
            .map(|texel| {
                let bits: [u16; 4] = bytemuck::cast(*texel);
                bits.map(|channel| half::f16::from_bits(channel).to_f32())
            })
            .collect(),
        wgpu::TextureFormat::Rgba32Float => rows
            .flat_map(|row| row.as_chunks::<16>().0)
            .map(|texel| bytemuck::cast::<[u8; 16], [f32; 4]>(*texel))
            .collect(),
        _ => unreachable!("a surface stores the working format or Rgba32Float"),
    }
}

/// Whether `pass` draws in coverage order: a surface pass of plain
/// source-over fills, front to back against the coverage depth.
fn coverage_order(pass: &lower::Pass) -> bool {
    matches!(pass.target, Target::Part(_))
        && !pass.ranges.is_empty()
        && pass.ranges.iter().all(|range| {
            range.pipeline == PipelineKind::SrcOver
                && range.variant == ShaderVariant::Simple
                && range.source.is_none()
                && range.image.is_none()
                && range.mask.is_none()
        })
}

/// The `wgpu` format for a [`ScratchFormat`].
const fn scratch_wgpu(format: ScratchFormat) -> wgpu::TextureFormat {
    match format {
        ScratchFormat::Content | ScratchFormat::LinearF16 => wgpu::TextureFormat::Rgba16Float,
        ScratchFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
    }
}

/// The pass-report spelling of a texture format.
const fn format_name(format: wgpu::TextureFormat) -> &'static str {
    match format {
        wgpu::TextureFormat::Rgba16Float => "rgba16float",
        wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
        wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8unorm-srgb",
        wgpu::TextureFormat::Rgba32Float => "rgba32float",
        _ => "unknown",
    }
}

/// One isolation scratch or backdrop texture.
#[derive(Clone)]
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
    revision: u64,
    /// Per-texel colour conversion flags, packed above sampling in image paints.
    pub sampling_flags: u32,
    /// The texture holding source texels.
    pub texture: wgpu::Texture,
    /// Its view for bind group 1.
    pub view: wgpu::TextureView,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
}

impl GpuImage {
    /// The texture's storage bytes: its format's blocks over the
    /// allocated extent.
    pub fn bytes(&self) -> u64 {
        image_storage::bytes(&self.texture)
    }
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
    part_ranges: Vec<present::ColorRange>,
    #[cfg(target_os = "android")]
    part_damage: Vec<cherenkov::damage::Region>,
    /// Texture-target hand-off: each rendered texture goes to the host.
    textures: Option<std::sync::mpsc::Sender<wgpu::Texture>>,
    ranges: Option<std::sync::mpsc::Sender<present::ColorRange>>,
    refresh: cherenkov::RefreshRange,
    present_pending: bool,
    #[cfg(target_os = "android")]
    present_generation: u64,
    /// The last display properties the frame carried — a change triggers
    /// the window's output re-selection (#98).
    display: cherenkov::Display,
    size: (u32, u32),
    /// The target's storage format, and every part's and plane capture's:
    /// [`WORKING_FORMAT`] unless an offscreen target requested another.
    format: wgpu::TextureFormat,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    /// Scratch textures keyed by the planner's reusable physical slots.
    scratch: FxHashMap<usize, ScratchTarget>,
    /// Transient depth for opaque regions; cleared and discarded in one pass.
    coverage_depth: Option<ScratchTarget>,
    /// Backdrop copies for blend composites, by the [`format_slot`] of the
    /// target they copy.
    backdrop: [Option<ScratchTarget>; RENDER_FORMATS.len()],
    /// The staged resolves' shared 1:1 device copies: every reduced
    /// capture with looked-through composites to apply first copies its
    /// device rect into the slot of its format, draws the composites
    /// over it and resolves from it — its contents die with that pass's
    /// resolve, so one texture per format serves every group, within
    /// and across frames. The slots are the captures' [`format_slot`]s.
    /// Each grows, never shrinks, to the largest staged device rect of
    /// its format.
    staging: [Option<ScratchTarget>; RENDER_FORMATS.len()],
    /// The staged-resolve bind group beside each staging slot: every
    /// staged resolve binds the same globals buffer and staging view at
    /// a dynamic offset, so one bind per slot serves all of them. It is
    /// dropped and rebuilt with its texture — a stale entry would keep
    /// the replaced view alive after the accounting says it is freed.
    /// The per-region bind caches hold only direct resolves.
    staging_bind: [Option<resolve::Bind>; RENDER_FORMATS.len()],
    /// Backdrop groups registered on this surface by raw id.
    backdrop_groups: FxHashMap<u64, BackdropGroupState>,
    anchor_scratch: lower::AnchorScratch,
    /// Path emission buffers, lent to each lowering so their capacity
    /// carries across frames; released by a critical trim.
    raster_scratch: path::EmitScratch,
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
    /// Image lifetimes and attachment assignment before allocation. Every
    /// backend has read dependencies; native executors also assign tile slots.
    composition: composite::plan::ExecutionPlan,
    composition_cache: composite::cache::Cache,
    draw_demand: composite::demand::Demand,
    command_damage: damage::Commands,
    propagated_damage: composite::demand::Changes,
    repaint_clears: repaint::Clears,
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
    fn replace_target(&mut self, size: (u32, u32), target: wgpu::Texture, view: wgpu::TextureView) {
        self.command_damage.forget();
        self.size = size;
        self.target = target;
        self.view = view;
    }

    fn new(
        window: Option<present::WindowSurface>,
        textures: Option<std::sync::mpsc::Sender<wgpu::Texture>>,
        ranges: Option<std::sync::mpsc::Sender<present::ColorRange>>,
        refresh: cherenkov::RefreshRange,
        output: (wgpu::Texture, wgpu::TextureView),
        waker: cherenkov::CompletionWaker,
        promotes: bool,
    ) -> Self {
        let (target, view) = output;
        Self {
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
            part_ranges: Vec::new(),
            #[cfg(target_os = "android")]
            part_damage: Vec::new(),
            textures,
            ranges,
            refresh,
            present_pending: false,
            #[cfg(target_os = "android")]
            present_generation: 0,
            display: cherenkov::Display::default(),
            size: (target.width(), target.height()),
            format: target.format(),
            target,
            view,
            scratch: FxHashMap::default(),
            coverage_depth: None,
            backdrop: Default::default(),
            staging: Default::default(),
            staging_bind: Default::default(),
            backdrop_groups: FxHashMap::default(),
            anchor_scratch: lower::AnchorScratch::default(),
            raster_scratch: path::EmitScratch::new(),
            layers: FxHashMap::default(),
            bindings: FxHashMap::default(),
            hosted: FxHashMap::default(),
            shader_textures: FxHashMap::default(),
            frame: LoweredFrame::default(),
            composition: composite::plan::ExecutionPlan::default(),
            composition_cache: composite::cache::Cache::default(),
            draw_demand: composite::demand::Demand::default(),
            command_damage: damage::Commands::default(),
            propagated_damage: composite::demand::Changes::default(),
            repaint_clears: repaint::Clears::default(),
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
        }
    }

    fn lowering_bytes(&self) -> u64 {
        #[cfg(target_os = "android")]
        let damage_bytes = (self.part_damage.capacity()
            * std::mem::size_of::<cherenkov::damage::Region>()) as u64
            + self
                .part_damage
                .iter()
                .map(|region| region.heap_bytes().0)
                .sum::<u64>();
        #[cfg(not(target_os = "android"))]
        let damage_bytes = 0;
        damage_bytes
            + self.composition.bytes()
            + self.composition_cache.bytes()
            + self.draw_demand.bytes()
            + self.command_damage.bytes()
            + self.propagated_damage.bytes()
            + self.repaint_clears.bytes()
            + (self.frame.commands.capacity() * size_of::<damage::Command>()) as u64
            + (self.frame.command_sources.capacity() * size_of::<damage::CommandSource>()) as u64
            + self.anchor_scratch.heap_bytes()
            + (self.part_ranges.capacity() * std::mem::size_of::<present::ColorRange>()) as u64
            + self.raster_scratch.heap_bytes()
            + self
                .layers
                .values()
                .map(ContentData::gpu_input_bytes)
                .sum::<u64>()
    }

    fn classify_parts(
        &mut self,
        frame: &SurfaceFrame<'_>,
        layers: &mut FxHashMap<LayerId, ContentData>,
        images: &FxHashMap<u64, GpuImage>,
    ) {
        let previous = self.part_ranges.first().copied();
        content_range::parts(
            frame.tree,
            layers,
            images,
            &self.bindings,
            &self.plan,
            frame.clear,
            &mut self.part_ranges,
        );
        if let Some(sender) = &self.ranges {
            let range = self.part_ranges[0];
            if previous != Some(range) {
                let _ = sender.send(range);
            }
        }
    }

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
    /// regions of all groups, pyramid levels included; the shared staging
    /// is counted by [`staging_bytes`](Self::staging_bytes).
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
            .map(|texture| texture.image.bytes() + 272)
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
fn source_view(
    surf: &SurfaceState,
    target: Target,
    pass: usize,
) -> Result<&wgpu::TextureView, RenderError> {
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
        Target::Scratch(_) => &surf.scratch[&surf.composition.scratch_slot(pass, target)].view,
        Target::Backdrop { group, .. } => {
            return Err(RenderError::Render(format!(
                "backdrop group {group} copies from a capture"
            )));
        }
    })
}

/// The format pass target `target` stores, which a capture copying it
/// takes too: the surface's for a part or a plane, the working format
/// for a projected image, the scratch format for a semantic isolation's
/// scratch. A capture never copies a capture.
fn stored_format(
    target: Target,
    surf: &SurfaceState,
    pass: usize,
) -> Result<wgpu::TextureFormat, RenderError> {
    match target {
        Target::Part(_) | Target::Plane(_) => Ok(surf.format),
        Target::Projected(_) => Ok(WORKING_FORMAT),
        Target::Scratch(_) => Ok(surf.scratch[&surf.composition.scratch_slot(pass, target)]
            .texture
            .format()),
        Target::Backdrop { group, .. } => Err(RenderError::Render(format!(
            "backdrop group {group} copies from a capture"
        ))),
    }
}

/// The blend backdrop slot of a pass drawing into `target`: the
/// [`format_slot`] of the target's format, since the copy keeps it. A
/// capture pass never blends against a backdrop copy.
fn blend_backdrop_slot(
    target: Target,
    surf: &SurfaceState,
    pass: usize,
) -> Result<usize, RenderError> {
    if matches!(target, Target::Backdrop { .. }) {
        return Err(RenderError::Render(
            "a blend backdrop copy on a capture pass".into(),
        ));
    }
    Ok(format_slot(stored_format(target, surf, pass)?))
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
        let slot = format_slot(stored_format(capture.copy_from, surf, index)?);
        let staging = &surf.staging[slot]
            .as_ref()
            .expect("staging allocated before encode")
            .view;
        pipelines
            .bind(device, &mut surf.staging_bind[slot], globals, staging)
            .clone()
    } else {
        let source = source_view(surf, capture.copy_from, index)?.clone();
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

/// The group-1 bind group key: `(source, blend backdrop slot, image,
/// mask texture)`.
type Bind1Key = (
    Option<lower::Source>,
    Option<usize>,
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
/// plain-variant `SrcOver` pipelines (one per distinct target format);
/// the plain module and the union-linked module text are stored so a
/// member composite's replace pipeline, and a union-field member's
/// pipelines, are built on their first use — lazily inside encode on
/// native targets, ahead of encode by `prepare_lazy_pipelines` on wasm.
struct BackdropShaderEntry {
    /// The plain-variant module, kept for its replace pipelines.
    plain_module: wgpu::ShaderModule,
    /// The union-shaped module text, validated at registration; the
    /// module is built on the first union use.
    union_source: std::sync::Arc<str>,
    /// The union-shaped module, built lazily with the first union slot.
    union_module: Option<wgpu::ShaderModule>,
    /// The plain-variant pipelines by `[replace][format slot]`: the
    /// source-over slots of the working and scratch formats built at
    /// registration, every other slot on first use.
    plain: [FormatCells<Option<wgpu::RenderPipeline>>; 2],
    /// The union-shaped pipelines by `[replace][format slot]` — built on
    /// first use.
    union: [FormatCells<Option<wgpu::RenderPipeline>>; 2],
}

/// All render-thread state.
pub struct GpuRenderer {
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    preparation: rayon::ThreadPool,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    presenter: Option<present::Presenter>,
    /// The plane realization of every surface whose target exposes a
    /// system-compositor parent. Platform objects stay on the render
    /// thread, outside the surface states lowering moves to its workers.
    #[cfg_attr(
        not(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32")),
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
    /// The filter pipelines every registered effect on this device shares.
    filter_pipelines: filtrate::PipelineCache,
    /// The silhouette-morphology pipeline — `None` until a frame actually
    /// composes a shadow, so shadow-free engines pay nothing (#170).
    shadow_blur: Option<shadow::Blur>,
    /// The reduced-capture resolve pipelines — `None` until a frame
    /// resolves a backdrop group captured below device resolution.
    resolve: Option<resolve::Pipelines>,
    /// The capture pyramid reduce pipelines — `None` until a frame
    /// reduces a levelled capture.
    reduce: Option<reduce::Pipelines>,
    /// The filter space conversion pipelines — `None` until a frame
    /// filters an sRGB-encoded scope.
    space_convert: Option<space_convert::Pipelines>,
    last_frame: Option<Instant>,
    origin: Option<Instant>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Core pipelines keyed by target format, blend mode and shader
    /// variant: `[format][replace][variant]` where the format index is
    /// the [`format_slot`] — when surface and scratch share a format,
    /// both draw through one slot, so identical
    /// pipelines are built once (#170). A `None` cell is created at first
    /// use — preparation is measured on the frame that needs it (#170 B3).
    /// `[format slot][replace][ShaderVariant]` — a fourth variant slot holds
    /// the union-field pipelines for backdrop-union members, a fifth the
    /// long-gradient pipelines, a sixth the cell-buffer pipelines of
    /// GPU-routed ranges (`DrawRange::module`) and a seventh the
    /// image-source pipelines. Slots 3, 5 and 6 are never built at init:
    /// their modules and pipelines are created on the first union or
    /// outer-band member, the first GPU-routed draw and the first image
    /// paint with source flags (on wasm, by `prepare_lazy_pipelines`
    /// before the frame that needs them).
    pipelines: FormatCells<[[Option<wgpu::RenderPipeline>; 12]; 2]>,
    /// `[format slot][module][opaque, partial]` coverage pipelines, same cell policy
    /// as the core set: the simple module's pair is built at init under
    /// `core_pipelines_eager()` (always, on wasm), else created at the
    /// first coverage pass (#170 B3, #49); the cell module's pair, which
    /// GPU-routed ranges draw with, at the first such range.
    coverage_pipelines: FormatCells<[[Option<wgpu::RenderPipeline>; 2]; 2]>,
    /// The pipeline layout every core and backdrop pipeline shares.
    pipeline_layout: wgpu::PipelineLayout,
    #[cfg(target_vendor = "apple")]
    tile_executor: composite::metal::Executor,
    /// The Vulkan tile executor's mirror of the engine's pipeline
    /// layout, descriptor sets and attachments (#2191).
    ///
    /// The reason the device declined where it cannot run an epoch —
    /// it lacks per-pixel attachment access or lazily allocated memory
    /// — which is also what makes [`GpuRenderer::tile_limits`] decline
    /// to plan; the two answers come from this one field, so no frame
    /// can be planned for an executor that is not there.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    tile_mirror: Result<composite::vulkan::Mirror, composite::vulkan::Declined>,
    /// The configured pipeline cache, opened once for the whole set.
    pipeline_cache: PipelineCache,
    /// Registered backdrop effect shaders, by raw id — see
    /// [`BackdropShaderEntry`].
    backdrop_shaders: FxHashMap<u64, BackdropShaderEntry>,
    /// The demand-built engine modules by variant index — union, and
    /// image — each created on the first frame that draws a range of that
    /// variant (async preparation cannot run inside the synchronous
    /// encode loop, so the module materializes in
    /// `prepare_lazy_pipelines`).
    #[cfg(target_arch = "wasm32")]
    lazy_modules: FxHashMap<usize, shaders::EnginePrograms>,
    /// The configured isolation texture format.
    scratch_format: wgpu::TextureFormat,
    layout0: wgpu::BindGroupLayout,
    layout1: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    stops: wgpu::Buffer,
    /// `image_storage::transfer_table`, group 0's `image_decode`.
    image_decode: wgpu::Buffer,
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
    /// The one-word stand-in `bind0` binds as the coverage arena until
    /// the GPU coverage route allocates the real one.
    arena_placeholder: wgpu::Buffer,
    /// The coverage arena generation `bind0` was built against.
    bound_arena_gen: u64,
    /// The frame's GPU coverage dispatch context (issue #2543).
    gpu_cov: gpu_cov::GpuCoverage,
    atlas: Atlas,
    /// Per-commit cell writes: the emptied container the last upload
    /// returned, refilled in place each frame (#2671).
    commit_writes: glyph::CellWrites,
    /// Per-surface pending origins, rebuilt in place each apply.
    pending_origins: Vec<PendingOrigin>,
    /// `(draw index, anchor instance)` of the surface's GPU draws,
    /// refilled in place each apply.
    gpu_draw_ranges: Vec<(u32, u32)>,
    /// Replay-pin slots, reused across evicting commits (#119).
    commit_touches: Vec<u32>,
    /// A dummy 1×1 view for unused group-1 slots.
    dummy_view: wgpu::TextureView,
    /// The texture `dummy_view` views.
    ///
    /// The Vulkan tile executor records natively, so every image its
    /// descriptor sets name has to be moved into the layout the native
    /// recording assumes — and a transition takes the texture, not the
    /// view (#2191).
    #[cfg(all(unix, not(target_vendor = "apple")))]
    dummy: wgpu::Texture,
    /// A dummy 1×1 `u32` view for unused external-frame plane slots.
    dummy_uint_view: wgpu::TextureView,
    /// The external-frame group-1 layout, `None` until a surface first
    /// draws an external frame.
    ext_layout: Option<wgpu::BindGroupLayout>,
    /// `[format slot]` external pipelines — the same slot indexing as
    /// `pipelines`; `None` until external frames are used (#170: the
    /// working and scratch formats' prepared at `set_external_frame`
    /// registration, never at idle, a surface's own format at its first
    /// external draw).
    external_pipes: FormatCells<Option<wgpu::RenderPipeline>>,
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
    /// A pass's beginning sample is an encoder timestamp written just
    /// before the pass opens, not the pass's own beginning-of-pass write.
    /// Set wherever the device has `TIMESTAMP_QUERY_INSIDE_ENCODERS`
    /// (Vulkan): Mali executes the in-pass beginning sample at a point
    /// that moves against the pass's own work — before or after roughly
    /// 20 µs of it, frame to frame — so a bracket opened there measures
    /// one of two spans and a per-process p50 lands on whichever
    /// dominates (#2840). The encoder sample precedes the pass's barriers
    /// and work in command order.
    pass_start_on_encoder: bool,
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
    /// supports (Metal on Apple GPUs samples only at stage boundaries);
    /// see `pass_start_on_encoder` for where a pass starts on Vulkan.
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

/// Cell origins and raster resolutions produced by one deferred raster.
enum PendingOrigin {
    /// A glyph's single origin and optional shelf.
    Glyph((u32, u32), Option<u32>),
    /// An admitted path whose cells and shelves remain pinned in the atlas.
    Path(u64),
    /// A clip mask's cell origin.
    Mask([f32; 2]),
    /// A GPU-routed path: its compute record index (issue #2543).
    Gpu(u32),
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
            a.get_texture_format_features(WORKING_FORMAT)
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
    // The float32 features are what an `Rgba32Float` offscreen surface
    // needs; a device without them rejects that surface by name.
    let mut required = supported
        & (image_storage::COMPRESSION_FEATURES
            | wgpu::Features::INDIRECT_FIRST_INSTANCE
            | wgpu::Features::SHADER_F16
            | float32_target_features());
    // The fixed engine shaders are precompiled (#57, #2726): Vulkan loads
    // SPIR-V, Metal a metallib and DX12 DXIL through the passthrough API;
    // wgpu-hal advertises the feature unconditionally on all three.
    if matches!(
        info.backend,
        wgpu::Backend::Vulkan | wgpu::Backend::Metal | wgpu::Backend::Dx12
    ) && supported.contains(wgpu::Features::PASSTHROUGH_SHADERS)
    {
        required |= wgpu::Features::PASSTHROUGH_SHADERS;
    }
    // Pass-boundary timestamps are all Metal is asked for: Apple GPUs
    // advertise `TIMESTAMP_QUERY_INSIDE_ENCODERS` but sample only at
    // stage boundaries, so an encoder-level `write_timestamp` goes through
    // a dummy blit encoder that wgpu itself documents as unreliable.
    // Vulkan is asked for both, because the tile executor records its
    // epoch in a native command buffer that is no wgpu pass: the only
    // place its bracket can go is the encoder, where Vulkan's
    // `vkCmdWriteTimestamp` is exact (#2191). Every render pass starts on
    // an encoder sample there too (`pass_start_on_encoder`, #2840). Every
    // wgpu Vulkan adapter with `TIMESTAMP_QUERY` has it, so it is
    // required, not probed.
    if config.timestamps && supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
        required |= wgpu::Features::TIMESTAMP_QUERY;
        if info.backend == wgpu::Backend::Vulkan {
            required |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        }
    }
    // Share compilation within this engine even without a persistent cache.
    if supported.contains(wgpu::Features::PIPELINE_CACHE) {
        required |= wgpu::Features::PIPELINE_CACHE;
    }
    let limits = wgpu::Limits::default().or_worse_values_from(&adapter.limits());
    // On Vulkan, device creation is where external-memory, semaphore and
    // YCbCr capabilities are enabled — the `open_with_callback` hook adds
    // the extensions/features wgpu does not request, preserving every
    // requirement wgpu does (#166). A non-Vulkan adapter falls through to
    // `request_device` unchanged.
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("cherenkov-gpu"),
        required_features: required,
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: crate::interop::memory_hints(info.backend),
        trace: wgpu::Trace::Off,
    };
    #[cfg(all(unix, not(target_vendor = "apple")))]
    if info.backend == wgpu::Backend::Vulkan {
        let (device, queue) = create_vulkan_device(&adapter, &descriptor)?;
        fire_probe(config, CreationPhase::Device, Some(&adapter), Some(&device))?;
        tracing::info!(features = ?device.features(), "device");
        return Ok((instance, adapter, device, queue));
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))
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
///
/// The on-chip composition capabilities (issue #2191) are enabled here
/// for the same reason: `as_hal` cannot turn on an extension after
/// `vkCreateDevice`, so a device that was not opened through this path
/// composes portably for the rest of its life. Every host that shares a
/// device with the engine therefore opens it here —
/// [`crate::interop::open_vulkan_device`] is this function.
#[cfg(all(unix, not(target_vendor = "apple"), not(target_arch = "wasm32")))]
pub fn create_vulkan_device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Device, wgpu::Queue), EngineError> {
    use external::vulkan;
    // SAFETY: `adapter` is the engine's own wgpu adapter; `as_hal`
    // borrows it for this scope and reports `None` off-Vulkan.
    let Some(hal_adapter) = (unsafe { adapter.as_hal::<wgpu::hal::vulkan::Api>() }) else {
        return Err(EngineError::Backend("adapter is not Vulkan".into()));
    };
    let caps = hal_adapter.physical_device_capabilities();
    let supported = |name: &core::ffi::CStr| caps.supports_extension(name);
    // SAFETY: the instance owns the physical device this adapter wraps,
    // and both outlive `hal_adapter`'s borrow.
    let mut tile = unsafe {
        composite::vulkan::Request::probe(
            hal_adapter.shared_instance().raw_instance(),
            hal_adapter.raw_physical_device(),
            supported,
        )
    };
    let mut extensions: Vec<&'static core::ffi::CStr> = vulkan::extra_device_extensions()
        .into_iter()
        .filter(|ext| supported(ext))
        .collect();
    extensions.extend(tile.extensions(supported));
    // The YCbCr conversion feature is keyed to its extension's presence.
    let ycbcr = supported(ash::khr::sampler_ycbcr_conversion::NAME);
    // `create_info.p_next` points at these structs until `vkCreateDevice`
    // runs inside `open_with_callback`; the `FnOnce` callback is dropped
    // before that call, so they are boxed in this scope instead of being
    // captured by the closure.
    let mut ycbcr_features = Box::new(
        ash::vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default()
            .sampler_ycbcr_conversion(true),
    );
    // SAFETY: the chain lives in this scope, which outlives the
    // `vkCreateDevice` call below. Its tail is null because the create
    // info the callback sees carries no `p_next` yet: wgpu's own
    // `add_to_device_create` runs after the callback and links its
    // structures ahead of whatever the callback left, so nothing wgpu
    // asked for is dropped.
    let tile_chain = unsafe { tile.chain(core::ptr::null()) };
    let access = tile.access();
    let callback: Option<Box<wgpu::hal::vulkan::CreateDeviceCallback<'_>>> =
        if ycbcr || !extensions.is_empty() {
            if ycbcr {
                ycbcr_features.p_next = tile_chain.unwrap_or(core::ptr::null()).cast_mut();
            }
            let head = if ycbcr {
                core::ptr::from_ref(&*ycbcr_features).cast::<core::ffi::c_void>()
            } else {
                tile_chain.unwrap_or(core::ptr::null())
            };
            Some(Box::new(
                move |args: wgpu::hal::vulkan::CreateDeviceCallbackArgs<'_, '_, '_>| {
                    for ext in &extensions {
                        if !args.extensions.contains(ext) {
                            args.extensions.push(ext);
                        }
                    }
                    if !head.is_null() {
                        args.create_info.p_next = head;
                    }
                },
            ))
        } else {
            None
        };
    let hints = descriptor.memory_hints.clone();
    tracing::info!(?access, "vulkan on-chip composition");
    // SAFETY: `callback` pins the feature chains in this scope, which
    // outlive the `vkCreateDevice` call inside `open_with_callback` (the
    // `FnOnce` runs there, not later).
    let opened = unsafe {
        hal_adapter.open_with_callback(
            descriptor.required_features,
            &descriptor.required_limits,
            &hints,
            callback,
        )
    }
    .map_err(|e| EngineError::Backend(format!("vulkan device creation: {e}")))?;
    drop(hal_adapter);
    // SAFETY: `opened` came from `hal_adapter.open_with_callback` on
    // this same adapter just above — the pairing contract.
    let (device, queue) =
        unsafe { adapter.create_device_from_hal::<wgpu::hal::vulkan::Api>(opened, descriptor) }
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
#[allow(
    clippy::future_not_send,
    reason = "the browser uses the page event loop; wgpu handles' Send bound varies with Cargo feature unification"
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
#[allow(
    clippy::future_not_send,
    reason = "the browser uses the page event loop; wgpu handles' Send bound varies with Cargo feature unification"
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
        .get_texture_format_features(WORKING_FORMAT)
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
    // The float32 features are what an `Rgba32Float` offscreen surface
    // needs; a device without them rejects that surface by name.
    let mut required = supported
        & (image_storage::COMPRESSION_FEATURES
            | wgpu::Features::INDIRECT_FIRST_INSTANCE
            | wgpu::Features::SHADER_F16
            | float32_target_features());
    // Pass-boundary timestamps are all Metal is asked for: Apple GPUs
    // advertise `TIMESTAMP_QUERY_INSIDE_ENCODERS` but sample only at
    // stage boundaries, so an encoder-level `write_timestamp` goes through
    // a dummy blit encoder that wgpu itself documents as unreliable.
    // Vulkan is asked for both, because the tile executor records its
    // epoch in a native command buffer that is no wgpu pass: the only
    // place its bracket can go is the encoder, where Vulkan's
    // `vkCmdWriteTimestamp` is exact (#2191). Every render pass starts on
    // an encoder sample there too (`pass_start_on_encoder`, #2840). Every
    // wgpu Vulkan adapter with `TIMESTAMP_QUERY` has it, so it is
    // required, not probed.
    if config.timestamps && supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
        required |= wgpu::Features::TIMESTAMP_QUERY;
        if info.backend == wgpu::Backend::Vulkan {
            required |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        }
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
            memory_hints: crate::interop::memory_hints(info.backend),
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
                bindings::Kind::Storage => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: entry.dynamic_offset,
                    min_binding_size: wgpu::BufferSize::new(entry.min_size),
                },
                bindings::Kind::Texture { filterable } => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable },
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

/// The texture behind the view [`image_view`] returns, for the paths
/// that must transition the resource rather than bind it.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn image_texture<'a>(
    source: &lower::ImageSource,
    images: &'a FxHashMap<u64, GpuImage>,
    bitmaps: &'a FxHashMap<BitmapKey, GpuBitmap>,
    shaders: &'a FxHashMap<std::sync::Arc<paint::Key>, paint::Texture>,
) -> &'a wgpu::Texture {
    match source {
        lower::ImageSource::Registered(id) => {
            &images
                .get(id)
                .unwrap_or_else(|| panic!("a lowered range samples unregistered image {id}"))
                .texture
        }
        lower::ImageSource::Bitmap(key) => {
            &bitmaps.get(key).expect("prepared bitmap").image.texture
        }
        lower::ImageSource::Shader(key) => &shaders[key].image.texture,
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

/// The placeholder coverage arena bound until the GPU coverage
/// route first sizes its buffer (issues #2543, #3081).
fn arena_placeholder(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("coverage arena"),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The buffers of the engine bind group (`ENGINE_GROUP0`).
#[derive(Clone, Copy)]
struct EngineBuffers<'a> {
    globals: &'a wgpu::Buffer,
    instances: &'a wgpu::Buffer,
    stops: &'a wgpu::Buffer,
    arena: &'a wgpu::Buffer,
    image_decode: &'a wgpu::Buffer,
}

fn make_bind0(
    device: &wgpu::Device,
    layout0: &wgpu::BindGroupLayout,
    buffers: &EngineBuffers<'_>,
    atlas: &Atlas,
) -> wgpu::BindGroup {
    let EngineBuffers {
        globals,
        instances,
        stops,
        arena,
        image_decode,
    } = *buffers;
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
                binding: 4,
                resource: arena.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(atlas.view()),
            },
            wgpu::BindGroupEntry {
                binding: bindings::IMAGE_DECODE_BINDING,
                resource: image_decode.as_entire_binding(),
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

/// `texture`'s `VkImage`.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn raw_vk_image(texture: &wgpu::Texture) -> ash::vk::Image {
    // SAFETY: `texture` is a live wgpu texture on the Vulkan backend —
    // asserted by `expect` — borrowed for this scope.
    let hal = unsafe { texture.as_hal::<wgpu::hal::vulkan::Api>() };
    // SAFETY: `hal` is the texture's own hal object, live above.
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
        ShaderVariant::LongGradient => 4,
        ShaderVariant::Cell => 5,
        ShaderVariant::CellSimple => 9,
        ShaderVariant::CellGlyph => 10,
        ShaderVariant::CellFull => 11,
        ShaderVariant::Image => 6,
        ShaderVariant::Glyph => 7,
        ShaderVariant::Composite => 8,
    }
}

/// The core slots built at init — every variant but Union and Image,
/// which materialize lazily (native `core_pipeline` demand cells) or in
/// `prepare_lazy_pipelines` (wasm). The list drives both the module
/// creation and the pipeline fill, zipped so a slot never indexes the
/// modules array.
const EAGER_CORE_VARIANTS: [usize; 4] = [0, 1, 2, 4];

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

/// One cache shared by the eager pipeline set. A configured path retains
/// the existing persistent cache; otherwise native init drops the temporary
/// compilation cache before returning the engine.
fn open_pipeline_cache(device: &wgpu::Device, config: &GpuConfig) -> Option<wgpu::PipelineCache> {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return None;
    }
    let data = config
        .pipeline_cache
        .as_ref()
        .and_then(|path| std::fs::read(path).ok());
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

/// Shared compilation cache and its existing persistence destination.
#[derive(Clone, Debug, Default)]
struct PipelineCache {
    handle: Option<wgpu::PipelineCache>,
    path: Option<std::path::PathBuf>,
}

impl PipelineCache {
    fn new(handle: Option<&wgpu::PipelineCache>, config: &GpuConfig) -> Self {
        Self {
            handle: handle.cloned(),
            path: config.pipeline_cache.clone(),
        }
    }

    fn persist(&self) {
        persist_cache_file(self.handle.as_ref(), self.path.as_deref());
    }
}

/// Persists the pipeline cache to its configured path, best effort:
/// future runs start closer to warm; a failed write only means the next
/// start recompiles.
fn persist_pipeline_cache(cache: Option<&wgpu::PipelineCache>, config: &GpuConfig) {
    persist_cache_file(cache, config.pipeline_cache.as_deref());
}

fn persist_cache_file(cache: Option<&wgpu::PipelineCache>, path: Option<&std::path::Path>) {
    if let (Some(cache), Some(path)) = (cache, path)
        && let Some(data) = cache.get_data()
        && let Err(error) = write_pipeline_cache(path, &data)
    {
        tracing::debug!(?path, %error, "could not persist pipeline cache");
    }
}

fn write_pipeline_cache(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data)
}

/// The closed pipeline set: one instanced-quad pipeline from `shader.wgsl`
/// on the shared pipeline layout.
#[cfg(not(target_arch = "wasm32"))]
fn create_pipeline(
    device: &wgpu::Device,
    config: &GpuConfig,
    layout: &wgpu::PipelineLayout,
    cache: Option<&wgpu::PipelineCache>,
    programs: &shaders::EnginePrograms,
    format: wgpu::TextureFormat,
    mode: CoveragePass,
) -> Result<wgpu::RenderPipeline, EngineError> {
    let pipeline = compile_pipeline(device, layout, cache, programs, format, mode)?;
    persist_pipeline_cache(cache, config);
    Ok(pipeline)
}

#[cfg(not(target_arch = "wasm32"))]
fn compile_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    cache: Option<&wgpu::PipelineCache>,
    programs: &shaders::EnginePrograms,
    format: wgpu::TextureFormat,
    mode: CoveragePass,
) -> Result<wgpu::RenderPipeline, EngineError> {
    let program = &programs[mode.program()];
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cherenkov"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &program.vertex,
            entry_point: Some(program.vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &program.fragment,
            entry_point: Some(program.fragment_entry),
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
    Ok(pipeline)
}

#[cfg(target_arch = "wasm32")]
#[allow(
    clippy::future_not_send,
    reason = "the browser uses the page event loop; wgpu handles' Send bound varies with Cargo feature unification"
)]
async fn create_pipeline(
    device: &wgpu::Device,
    config: &GpuConfig,
    layout: &wgpu::PipelineLayout,
    cache: Option<&wgpu::PipelineCache>,
    programs: &shaders::EnginePrograms,
    format: wgpu::TextureFormat,
    mode: CoveragePass,
) -> Result<wgpu::RenderPipeline, EngineError> {
    let program = &programs[mode.program()];
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cherenkov"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &program.vertex,
            entry_point: Some(program.vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &program.fragment,
            entry_point: Some(program.fragment_entry),
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
            .engine_programs(self.device, variant_index(variant));
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

/// The core pipeline for `(format, replace, variant)` — the
/// [`format_slot`] index shares pipelines across surfaces and scratch
/// layers whose formats coincide, and an empty cell is created at first
/// use so preparation lands inside the frame that needs it (#170).
fn core_pipeline<'m>(
    pipelines: &'m mut FormatCells<[[Option<wgpu::RenderPipeline>; 12]; 2]>,
    factory: &PipelineFactory<'_>,
    format: wgpu::TextureFormat,
    replace: bool,
    variant: ShaderVariant,
) -> Result<&'m wgpu::RenderPipeline, RenderError> {
    let cell = &mut pipelines[format_slot(format)][usize::from(replace)][variant_index(variant)];
    if cell.is_none() {
        *cell = Some(factory.create(format, CoveragePass::Painter(replace), variant)?);
    }
    Ok(cell.as_ref().expect("core pipeline ready"))
}

/// The `[opaque, partial]` coverage pipeline for a surface target in
/// `format` — the same cell policy as `core_pipeline`: an empty cell is
/// created at the first coverage pass that needs it (#170 B3, #49).
fn coverage_pipeline<'m>(
    pipelines: &'m mut FormatCells<[[Option<wgpu::RenderPipeline>; 2]; 2]>,
    factory: &PipelineFactory<'_>,
    format: wgpu::TextureFormat,
    phase: CoveragePhase,
    gpu: bool,
) -> Result<&'m wgpu::RenderPipeline, RenderError> {
    let cell = &mut pipelines[format_slot(format)][usize::from(gpu)][phase as usize];
    if cell.is_none() {
        *cell = Some(factory.create(
            format,
            CoveragePass::Coverage(phase),
            if gpu {
                ShaderVariant::Cell
            } else {
                ShaderVariant::Simple
            },
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
    program: &shaders::RenderProgram,
    format: wgpu::TextureFormat,
    cache: &PipelineCache,
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
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cherenkov external"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &program.vertex,
            entry_point: Some(program.vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &program.fragment,
            entry_point: Some(program.fragment_entry),
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
        cache: cache.handle.as_ref(),
    });
    cache.persist();
    pipeline
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

/// The `image_decode` storage binding: the transfer table straight 8-bit
/// image sources decode through (`image_storage::transfer_table`).
fn image_decode_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("image decode"),
        contents: bytemuck::cast_slice(&image_storage::transfer_table()),
        usage: wgpu::BufferUsages::STORAGE,
    })
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
        // Specialised fragment shaders from one source file, compiled
        // at build time with a statically selected fragment entry point;
        // the union module is not in this eager
        // set (it materializes with the first union member). On Vulkan
        // and Metal these are the embedded passthrough binaries (#57).
        let shader_delivery = shaders::delivery(info.backend, &device)?;
        // What this device — the engine's own or a host's shared one —
        // can keep on chip (#2191). A shared device opened outside
        // `interop::open_vulkan_device` reports `None` here, which is
        // the only visible sign that its host bypassed the engine's
        // creation path.
        #[cfg(all(unix, not(target_vendor = "apple"), not(target_arch = "wasm32")))]
        if info.backend == wgpu::Backend::Vulkan {
            tracing::info!(
                access = ?composite::vulkan::device_access(&device),
                "vulkan on-chip composition"
            );
        }
        let modules = core_pipelines_eager()
            .then(|| EAGER_CORE_VARIANTS.map(|v| shader_delivery.engine_programs(&device, v)));
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
        let mut coverage_pipelines =
            FormatCells::<[[Option<wgpu::RenderPipeline>; 2]; 2]>::default();
        // The closed core set, deduplicated by format slot: when the
        // working and scratch formats coincide every cell lands in one
        // slot and the later iteration's `is_none` skips its duplicates
        // (#170). A surface's own `Rgba32Float` slot is never built here.
        // CoreDemand leaves the cells empty — `core_pipeline` fills each
        // at the first draw that needs it (#170 B3). The coverage pair
        // follows the same policy via `coverage_pipeline`.
        if let Some(modules) = &modules {
            // Each shader family compiles independently. Keep formats and blend
            // modes within a family serial so they reuse the shared driver cache.
            // The four workers exist only during initialization; drawing never
            // waits for background compilation or substitutes another pipeline.
            let compiled = std::thread::scope(|scope| {
                let workers = std::array::from_fn::<_, 4, _>(|index| {
                    let variant = EAGER_CORE_VARIANTS[index];
                    let module = &modules[index];
                    let (device, layout, cache) =
                        (&device, &pipeline_layout, pipeline_cache.as_ref());
                    scope.spawn(move || {
                        let mut compiled = Vec::new();
                        for format in [WORKING_FORMAT, scratch_format]
                            .into_iter()
                            .enumerate()
                            .filter_map(|(index, format)| {
                                (index == 0 || format != WORKING_FORMAT).then_some(format)
                            })
                        {
                            for replace in [false, true] {
                                let pipeline = compile_pipeline(
                                    device,
                                    layout,
                                    cache,
                                    module,
                                    format,
                                    CoveragePass::Painter(replace),
                                )?;
                                compiled.push((format_slot(format), replace, pipeline));
                            }
                        }
                        Ok::<_, EngineError>((variant, compiled))
                    })
                });
                workers
                    .into_iter()
                    .map(|worker| worker.join().expect("pipeline compiler panicked"))
                    .collect::<Result<Vec<_>, EngineError>>()
            })?;
            for (variant, compiled) in compiled {
                for (slot, replace, pipeline) in compiled {
                    pipelines[slot][usize::from(replace)][variant] = Some(pipeline);
                }
            }
            for (cell, phase) in coverage_pipelines[format_slot(WORKING_FORMAT)][0]
                .iter_mut()
                .zip([CoveragePhase::Opaque, CoveragePhase::Partial])
            {
                *cell = Some(compile_pipeline(
                    &device,
                    &pipeline_layout,
                    pipeline_cache.as_ref(),
                    &modules[0],
                    WORKING_FORMAT,
                    CoveragePass::Coverage(phase),
                )?);
            }
            persist_pipeline_cache(pipeline_cache.as_ref(), &config);
        }
        // The pipelines own their compiled programs. Drop only the temporary
        // compilation cache here; configured disk caches keep their existing
        // renderer lifetime and persistence behavior.
        let pipeline_cache = config.pipeline_cache.as_ref().and(pipeline_cache);
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
        let image_decode = image_decode_buffer(&device);
        diag::create(&device, "globals", globals.size());
        diag::create(&device, "instances", instances.size());
        diag::create(&device, "stops", stops.size());
        diag::create(&device, "image decode", image_decode.size());
        fire_probe(
            &config,
            CreationPhase::Buffers,
            Some(&adapter),
            Some(&device),
        )?;
        let atlas = Atlas::new(&device, config.budget.gpu.0);
        fire_probe(&config, CreationPhase::Atlas, Some(&adapter), Some(&device))?;
        let arena_placeholder = arena_placeholder(&device);
        let bind0 = make_bind0(
            &device,
            &layout0,
            &EngineBuffers {
                globals: &globals,
                instances: &instances,
                stops: &stops,
                arena: &arena_placeholder,
                image_decode: &image_decode,
            },
            &atlas,
        );
        #[cfg_attr(
            not(all(unix, not(target_vendor = "apple"))),
            expect(
                unused_variables,
                reason = "only the Vulkan executor transitions the dummy texture (#2191)"
            )
        )]
        let (dummy, dummy_view) = create_target(
            &device,
            "dummy source",
            (1, 1),
            wgpu::TextureUsages::TEXTURE_BINDING,
            WORKING_FORMAT,
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
        let pass_start_on_encoder = timestamps
            && device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
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
        // The tile executor's mirror, built once for a device that can
        // run an epoch. Its shader modules and layouts are the whole
        // cost here — every pipeline is created on first use — and a
        // device that cannot run an epoch builds none of it (#2191,
        // #2689). Once the device is eligible, a mirror that fails to
        // build is an engine failure.
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let tile_mirror = match composite::vulkan::device_eligibility(&device, &adapter) {
            Ok(eligible) => Ok(composite::vulkan::Mirror::new(
                &device,
                &adapter,
                eligible,
                config.pipeline_cache.as_deref(),
            )
            .map_err(|e| EngineError::Backend(format!("tile composition: {e}")))?),
            Err(declined) => {
                tracing::info!(?declined, "vulkan tile composition unavailable");
                Err(declined)
            }
        };
        let gpu_cov = gpu_cov::GpuCoverage::new(
            &device,
            config.budget.gpu.0 / gpu_cov::BUDGET_SHARE,
            PipelineCache::new(pipeline_cache.as_ref(), &config),
        );
        let pipeline_cache = PipelineCache::new(pipeline_cache.as_ref(), &config);
        let renderer = GpuRenderer {
            #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
            preparation: rayon::ThreadPoolBuilder::new()
                .num_threads(cherenkov::workers::count(None)?.get())
                .thread_name(|i| format!("cherenkov-geometry-{i}"))
                .build()
                .map_err(|e| EngineError::Backend(format!("geometry pool: {e}")))?,
            instance,
            adapter,
            presenter: None,
            #[cfg_attr(
                not(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32")),
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
            filter_pipelines: filtrate::PipelineCache::with_driver_cache(
                pipeline_cache.handle.clone(),
            ),
            shadow_blur: None,
            resolve: None,
            reduce: None,
            space_convert: None,
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
            #[cfg(all(unix, not(target_vendor = "apple")))]
            tile_mirror,
            pipeline_cache,
            scratch_format,
            layout0,
            layout1,
            globals,
            instances,
            stops,
            image_decode,
            quad_indices,
            bind0,
            dummy_view,
            #[cfg(all(unix, not(target_vendor = "apple")))]
            dummy,
            dummy_uint_view,
            ext_layout: None,
            external_pipes: Default::default(),
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
            arena_placeholder,
            bound_arena_gen: 0,
            gpu_cov,
            bound_stop_size: 32 * 16,
            bound_globals_size: 256,
            atlas,
            commit_writes: glyph::CellWrites::default(),
            pending_origins: Vec::new(),
            gpu_draw_ranges: Vec::new(),
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
            pass_start_on_encoder,
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
#[allow(
    clippy::future_not_send,
    reason = "the browser uses the page event loop; wgpu handles' Send bound varies with Cargo feature unification"
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
    // Specialised fragment shaders with statically selected entry points;
    // the union module is not in this eager set. WebGPU keeps WGSL (#57).
    let shader_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader_delivery = shaders::delivery(info.backend, &device)?;
    let modules = EAGER_CORE_VARIANTS.map(|v| shader_delivery.engine_programs(&device, v));
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
    // The closed core set, deduplicated by format slot: when the working
    // and scratch formats coincide every cell lands in one slot and the
    // later iteration's `is_none` skips its duplicates (#170). wasm
    // always prepares these eagerly — async creation cannot run inside
    // the synchronous encode loop — and a surface's own `Rgba32Float`
    // cells in `prepare_lazy_pipelines` before its first frame.
    for format in [WORKING_FORMAT, scratch_format] {
        let slot = format_slot(format);
        for replace in [false, true] {
            for (variant, module) in EAGER_CORE_VARIANTS.iter().zip(&modules) {
                let cell = &mut pipelines[slot][usize::from(replace)][*variant];
                if cell.is_none() {
                    *cell = Some(
                        create_pipeline(
                            &device,
                            &config,
                            &pipeline_layout,
                            pipeline_cache.as_ref(),
                            module,
                            format,
                            CoveragePass::Painter(replace),
                        )
                        .await?,
                    );
                }
            }
        }
    }
    let simple_coverage = <[_; 2]>::from(
        futures_util::future::try_join(
            create_pipeline(
                &device,
                &config,
                &pipeline_layout,
                pipeline_cache.as_ref(),
                &modules[0],
                WORKING_FORMAT,
                CoveragePass::Coverage(CoveragePhase::Opaque),
            ),
            create_pipeline(
                &device,
                &config,
                &pipeline_layout,
                pipeline_cache.as_ref(),
                &modules[0],
                WORKING_FORMAT,
                CoveragePass::Coverage(CoveragePhase::Partial),
            ),
        )
        .await?,
    )
    .map(Some);
    let mut coverage_pipelines = FormatCells::<[[Option<wgpu::RenderPipeline>; 2]; 2]>::default();
    coverage_pipelines[format_slot(WORKING_FORMAT)][0] = simple_coverage;
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
    let image_decode = image_decode_buffer(&device);
    diag::create(&device, "globals", globals.size());
    diag::create(&device, "instances", instances.size());
    diag::create(&device, "stops", stops.size());
    diag::create(&device, "image decode", image_decode.size());
    fire_probe(
        &config,
        CreationPhase::Buffers,
        Some(&adapter),
        Some(&device),
    )?;
    let atlas = Atlas::new(&device, config.budget.gpu.0);
    fire_probe(&config, CreationPhase::Atlas, Some(&adapter), Some(&device))?;
    let arena_placeholder = arena_placeholder(&device);
    let bind0 = make_bind0(
        &device,
        &layout0,
        &EngineBuffers {
            globals: &globals,
            instances: &instances,
            stops: &stops,
            arena: &arena_placeholder,
            image_decode: &image_decode,
        },
        &atlas,
    );
    let (_, dummy_view) = create_target(
        &device,
        "dummy source",
        (1, 1),
        wgpu::TextureUsages::TEXTURE_BINDING,
        WORKING_FORMAT,
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
    let pass_start_on_encoder = timestamps
        && device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
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
    let gpu_cov = gpu_cov::GpuCoverage::new(
        &device,
        config.budget.gpu.0 / gpu_cov::BUDGET_SHARE,
        PipelineCache::new(pipeline_cache.as_ref(), &config),
    );
    let pipeline_cache = PipelineCache::new(pipeline_cache.as_ref(), &config);
    let renderer = GpuRenderer {
        instance,
        adapter,
        presenter: None,
        #[cfg_attr(
            not(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32")),
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
        lazy_modules: FxHashMap::default(),
        filters: filter::Registry::default(),
        filter_pipelines: filtrate::PipelineCache::with_driver_cache(pipeline_cache.handle.clone()),
        shadow_blur: None,
        resolve: None,
        reduce: None,
        space_convert: None,
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
        image_decode,
        quad_indices,
        arena_placeholder,
        bound_arena_gen: 0,
        gpu_cov,
        bind0,
        dummy_view,
        dummy_uint_view,
        ext_layout: None,
        external_pipes: Default::default(),
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
        commit_writes: glyph::CellWrites::default(),
        pending_origins: Vec::new(),
        gpu_draw_ranges: Vec::new(),
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
        pass_start_on_encoder,
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
                            range: surface.static_layers[&placement.layer].range,
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
        (entry.range == present::ColorRange::HighDynamicRange
            || entry.quiet_frames >= entry.quiet_required)
            .then_some((
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

impl GpuRenderer {
    /// [`Renderer::add_image`]'s work, awaiting the device's report.
    #[expect(
        clippy::future_not_send,
        reason = "the renderer is not Send: native targets block on this future on the render thread, and the browser engine runs it on the page's event loop"
    )]
    async fn register_image(
        &mut self,
        id: ImageId,
        image: ImageUpload,
    ) -> Result<(), ResourceError> {
        let size = (image.width, image.height);
        let data = image_storage::Upload::new(&image, self.device.features())?;
        let image = upload_image(&self.device, &self.queue, size, &data).await?;
        self.images.insert(id.raw(), image);
        self.images_gen += 1;
        Ok(())
    }

    /// [`Renderer::replace_image`]'s work, awaiting the device's report.
    #[expect(
        clippy::future_not_send,
        reason = "the renderer is not Send: native targets block on this future on the render thread, and the browser engine runs it on the page's event loop"
    )]
    async fn swap_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        let data = image_storage::Upload::new(&image, self.device.features())?;
        let size = (image.width, image.height);
        let current = self
            .images
            .get(&id.raw())
            .expect("replace targets a registered image");
        if (current.width, current.height) == size
            && current.texture.format() == data.format
            && current.sampling_flags == data.flags
        {
            // The copy is queued ahead of the next submission, after every
            // frame already submitted, so no frame samples a partly
            // written texture. The view is unchanged: bind groups and
            // lowered paints stay valid.
            write_image(&self.device, &self.queue, &current.texture, size, &data).await?;
            // Local images that sampled the previous pixels are no longer
            // current.
            self.image_replacements += 1;
            self.images
                .get_mut(&id.raw())
                .expect("registered image")
                .revision += 1;
            return Ok(());
        }
        let revision = current.revision + 1;
        let mut replacement = upload_image(&self.device, &self.queue, size, &data).await?;
        replacement.revision = revision;
        self.image_replacements += 1;
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
                bytes: old.bytes(),
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
}

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
        mut target: GpuTarget,
        waker: cherenkov::CompletionWaker,
    ) -> Result<SurfaceInfo, SurfaceError> {
        let _diag_guard = diag::Guard::scope(self.diag.as_ref());
        diag::set_surface(Some(id.raw()));
        let size = self.target_size(&target)?;
        let format = surface_format(&self.device, &target)?;
        let ranges = target.take_content_ranges();
        let (window, textures, refresh, presents) = match target {
            GpuTarget::Offscreen(offscreen) => (None, None, offscreen.refresh, false),
            GpuTarget::Texture(texture) => (None, Some(texture.textures), texture.refresh, false),
            #[cfg(target_os = "android")]
            GpuTarget::SurfaceControl(target) => {
                let refresh = self.surface_control(id, target, waker.clone())?;
                (None, None, refresh, true)
            }
            #[cfg(target_os = "linux")]
            GpuTarget::Dmabuf(target) => (None, None, self.dmabuf_export(id, target)?, true),
            #[cfg(target_arch = "wasm32")]
            GpuTarget::Dom(target) => (None, None, self.dom_planes(id, &target)?, true),
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
        let (target, view) =
            create_target(&self.device, "surface target", size, TARGET_USAGES, format);
        notify_texture(textures.as_ref(), &target);
        diag::create(
            &self.device,
            "surface target",
            u64::from(size.0) * u64::from(size.1) * texel_bytes(format),
        );
        self.surfaces.insert(
            id,
            SurfaceState::new(
                window,
                textures,
                ranges,
                refresh,
                (target, view),
                waker,
                promotes,
            ),
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
            state.format,
        );
        if let Some(window) = &mut state.window {
            window.resize(&self.device, size);
        }
        notify_texture(state.textures.as_ref(), &target);
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
        state.replace_target(size, target, view);
        for part in &mut state.parts {
            *part = create_target(
                &self.device,
                "engine part",
                size,
                TARGET_USAGES,
                state.format,
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
        state.backdrop = Default::default();
        state.staging = Default::default();
        state.staging_bind = Default::default();
        state.retire_binds(&self.device);
        state.painter_replays.clear();
        state.bind_gen += 1;
        diag::grow(
            &self.device,
            "surface target",
            diag::Class::Target,
            old_target,
            u64::from(size.0) * u64::from(size.1) * texel_bytes(state.format),
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
        let head = parsed
            .head()
            .map_err(|e| ResourceError::Font(format!("head: {e}")))?;
        let units_per_em = head.units_per_em();
        let default_bounds = kurbo::Rect::new(
            f64::from(head.x_min()),
            f64::from(head.y_min()),
            f64::from(head.x_max()),
            f64::from(head.y_max()),
        );
        let has_colr = parsed.colr().is_ok();
        let bitmap = bitmap::BitmapFont::detect(&font.data, font.index)?.map(Arc::new);
        Ok(PreparedFont {
            data: font.data,
            index: font.index,
            units_per_em,
            default_bounds,
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
            max_bytes: self.config.budget.gpu.0,
            storage: image_storage::limits(self.device.features()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn add_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        pollster::block_on(self.register_image(id, image))
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    fn add_image(
        &mut self,
        id: ImageId,
        image: ImageUpload,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>> {
        self.register_image(id, image)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        pollster::block_on(self.swap_image(id, image))
    }

    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    fn replace_image(
        &mut self,
        id: ImageId,
        image: ImageUpload,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>> {
        self.swap_image(id, image)
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
                    bytes: image.bytes(),
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
        self.gpu_cov.trim();
        if self.gpu_cov.arena_generation() != self.bound_arena_gen {
            let atlas = &self.atlas;
            self.bind0 = make_bind0(
                &self.device,
                &self.layout0,
                &EngineBuffers {
                    globals: &self.globals,
                    instances: &self.instances,
                    stops: &self.stops,
                    arena: &self.arena_placeholder,
                    image_decode: &self.image_decode,
                },
                atlas,
            );
            self.bound_arena_gen = self.gpu_cov.arena_generation();
        }
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
            surf.composition = composite::plan::ExecutionPlan::default();
            surf.composition_cache = composite::cache::Cache::default();
            #[cfg(target_vendor = "apple")]
            surf.tile_targets.clear();
            surf.coverage_depth = None;
            surf.backdrop = Default::default();
            surf.staging = Default::default();
            surf.staging_bind = Default::default();
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
        // The commit batch's reusable lists are the same kind of
        // retained scratch: a trimmed engine keeps none of their peak
        // capacity, and the next commit regrows them as needed (#2711).
        self.commit_writes = glyph::CellWrites::default();
        if pressure != Pressure::Critical {
            return;
        }
        self.atlas.clear();
        self.bitmaps.clear();
        self.images_gen += 1;
        for font in self.fonts.values() {
            font.colr.borrow_mut().clear();
            font.clear_bounds();
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
            surf.raster_scratch = path::EmitScratch::new();
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
            &EngineBuffers {
                globals: &self.globals,
                instances: &self.instances,
                stops: &self.stops,
                arena: self.gpu_cov.arena().unwrap_or(&self.arena_placeholder),
                image_decode: &self.image_decode,
            },
            atlas,
        );
        self.bound_atlas = atlas.generation();
        self.bound_arena_gen = self.gpu_cov.arena_generation();
        diag::set_phase("render");
    }

    fn owned_animations(&self, surface: SurfaceId) -> &[LayerId] {
        self.planes
            .get(&surface)
            .map_or(&[], |planes| planes::SystemPlanes::owned_animations(planes))
    }

    fn memory(&self) -> MemoryUsage {
        let gpu = self.instances.size()
            // The GPU coverage route's arena, draws and scratch buffers.
            + self.gpu_cov.bytes()
            + self
                .planes
                .values()
                .map(planes::SystemPlanes::native_bytes)
                .sum::<u64>()
            + self.uploads.gpu_bytes()
            + self.stops.size()
            + self.image_decode.size()
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
            + self.images.values().map(GpuImage::bytes).sum::<u64>()
            + self
                .bitmaps
                .values()
                .map(|bitmap| bitmap.image.bytes())
                .sum::<u64>();
        let captures = self
            .surfaces
            .values()
            .map(SurfaceState::backdrop_bytes)
            .sum();
        let filter_targets = self.filters.target_bytes();
        let filter_images = self.filters.image_bytes();
        let capture_format = self
            .surfaces
            .values()
            .flat_map(|surf| surf.backdrop_groups.values())
            .flat_map(|g| &g.captures)
            .map(|c| format_name(c.target.texture.format()))
            .next();
        let glyph_cache =
            self.atlas.cpu_bytes() + self.fonts.values().map(FontData::bounds_bytes).sum::<u64>();
        let cpu = glyph_cache
            + self
                .planes
                .values()
                .map(planes::SystemPlanes::cpu_bytes)
                .sum::<u64>()
            + self.gpu_cov.cpu_bytes()
            + self.commit_writes.heap_bytes()
            + self
                .surfaces
                .values()
                .map(SurfaceState::lowering_bytes)
                .sum::<u64>()
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
        // The tile executor's attachments, reported apart from the rest
        // because they are the one allocation the engine makes that it
        // expects device memory *not* to back (#2191). Apple's are
        // memoryless and have no commitment to read; Vulkan's are
        // lazily allocated, and the driver says how much of them it had
        // to commit. Only the committed part is real device memory, so
        // only it joins `gpu`.
        #[cfg(target_vendor = "apple")]
        let (tile_reserved, tile_committed) = (
            self.surfaces
                .values()
                .map(|surf| surf.tile_targets.reserved_bytes())
                .sum(),
            None,
        );
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let (tile_reserved, tile_committed) = self.tile_mirror.as_ref().map_or((0, None), |m| {
            (m.reserved_bytes(), Some(m.committed_total_bytes()))
        });
        #[cfg(not(unix))]
        let (tile_reserved, tile_committed) = (0, None);
        MemoryUsage {
            gpu: cherenkov::Bytes(
                gpu + filter_targets
                    + filter_images
                    + self.shadow_blur.as_ref().map_or(0, shadow::Blur::gpu_bytes)
                    + tile_committed.unwrap_or(0),
            ),
            cpu: cherenkov::Bytes(cpu),
            glyph_cache: cherenkov::Bytes(glyph_cache),
            tile_reserved: cherenkov::Bytes(tile_reserved),
            tile_committed: tile_committed.map(cherenkov::Bytes),
            backdrop_captures: cherenkov::Bytes(captures),
            backdrop_capture_format: capture_format,
            filter_targets: cherenkov::Bytes(filter_targets),
            filter_images: cherenkov::Bytes(filter_images),
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
        let format = state.format;
        diag::set_surface(Some(surface.raw()));
        diag::set_phase("readback");
        let bytes_per_row = readback_pitch(format, w);
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
        let pixels = decode_readback(&data, format, (w, h), bytes_per_row);
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
        let format = state.format;
        diag::set_surface(Some(surface.raw()));
        diag::set_phase("readback");
        let bytes_per_row = readback_pitch(format, w);
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
        let pixels = decode_readback(&data, format, (w, h), bytes_per_row);
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
            #[cfg(target_arch = "wasm32")]
            GpuTarget::Dom(target) => target.size(),
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
        waker: cherenkov::CompletionWaker,
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
            waker,
        )?;
        self.planes.insert(id, system);
        self.presenter.get_or_insert_with(|| {
            present::Presenter::with_cache(
                &self.device,
                self.shader_delivery,
                self.pipeline_cache.clone(),
            )
        });
        Ok(target.refresh)
    }

    /// Realizes surface `id` under a `DomTarget`'s element and returns its
    /// refresh range.
    #[cfg(target_arch = "wasm32")]
    fn dom_planes(
        &mut self,
        id: SurfaceId,
        target: &crate::interop::web::DomTarget,
    ) -> Result<cherenkov::RefreshRange, SurfaceError> {
        let system =
            planes::web::DomPlanes::new(&self.instance, &self.adapter, &self.device, target)?;
        self.planes.insert(id, system);
        self.presenter.get_or_insert_with(|| {
            present::Presenter::with_cache(
                &self.device,
                self.shader_delivery,
                self.pipeline_cache.clone(),
            )
        });
        Ok(target.refresh.clone())
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
        self.presenter.get_or_insert_with(|| {
            present::Presenter::with_cache(
                &self.device,
                self.shader_delivery,
                self.pipeline_cache.clone(),
            )
        });
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
        let t_prepare = Instant::now();
        let prepared = self.prepare_plane_content(frame)?;
        let prepared_seconds = t_prepare.elapsed().as_secs_f64();
        stats.phases.lower_seconds = prepared_seconds;
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
            if !prepared.contains_key(&sf.id) && self.plane_only_frames(sf) {
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
        let results = self.lower_all(&mut pending, &dirty, &inputs, prepared);
        // GPU coverage generations count once per commit, wherever the
        // entry landed (water-rs/waterui#3081).
        stats.paths_rasterized += self.gpu_cov.generated();
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
            prepared_seconds + t_lower.elapsed().as_secs_f64() - (stats.phases.wait_seconds - wait);
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
                // variant-3 cells a union or outer-band member needs, and
                // the effect pipelines built on first use, are prepared
                // here, once the frame is lowered.
                #[cfg(target_arch = "wasm32")]
                self.prepare_lazy_pipelines(sf.id).await?;
            }
            diag::set_phase("encode");
            let t = Instant::now();
            for sf in &dirty {
                let count = self.frame_pass_count;
                let meta = self.pass_meta.len();
                if let Err(error) = self.encode_surface(sf.id, frame.id, timing, stats) {
                    self.frame_pass_count = count;
                    self.pass_meta.truncate(meta);
                    result = Err(error);
                    break;
                }
                self.finish_projective(sf.id);
                let surface = self.surfaces.get_mut(&sf.id).expect("rendered surface");
                #[cfg(target_os = "android")]
                {
                    surface.present_generation += 1;
                }
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
        let t_prepare = Instant::now();
        let prepared = self.prepare_plane_content(frame)?;
        let prepared_seconds = t_prepare.elapsed().as_secs_f64();
        stats.phases.lower_seconds = prepared_seconds;
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
            if !prepared.contains_key(&sf.id) && self.plane_only_frames(sf) {
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
        let results = self.lower_all(&mut pending, &dirty, &inputs, prepared);
        // GPU coverage generations count once per commit, wherever the
        // entry landed (water-rs/waterui#3081).
        stats.paths_rasterized += self.gpu_cov.generated();
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
            prepared_seconds + t_lower.elapsed().as_secs_f64() - (stats.phases.wait_seconds - wait);
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
                if let Err(error) = self.encode_surface(sf.id, frame.id, timing, stats) {
                    self.frame_pass_count = count;
                    self.pass_meta.truncate(meta);
                    result = Err(error);
                    break;
                }
                self.finish_projective(sf.id);
                let surface = self.surfaces.get_mut(&sf.id).expect("rendered surface");
                #[cfg(target_os = "android")]
                {
                    surface.present_generation += 1;
                }
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
        self.shaders
            .add(&self.device, id.raw(), source, &self.pipeline_cache)
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
        self.shaders
            .add(&self.device, id.raw(), source, &self.pipeline_cache)
            .await
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
                    self.pipeline_cache.handle.as_ref(),
                    &shaders::wgsl_engine_programs(module),
                    format,
                    CoveragePass::Painter(false),
                )
                .map_err(|e| ResourceError::Shader(e.to_string()))
            };
        // One pipeline per distinct working or scratch format on the plain
        // variant — equal formats share one slot (#170).
        let mut pipelines: FormatCells<Option<Result<wgpu::RenderPipeline, ResourceError>>> =
            Default::default();
        for format in [WORKING_FORMAT, self.scratch_format] {
            let slot = format_slot(format);
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
        let mut plain = <[FormatCells<Option<wgpu::RenderPipeline>>; 2]>::default();
        for (cell, built) in plain[0].iter_mut().zip(pipelines) {
            *cell = built.transpose()?;
        }
        self.backdrop_shaders.insert(
            id.raw(),
            BackdropShaderEntry {
                plain_module: module,
                union_source,
                union_module: None,
                plain,
                union: Default::default(),
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
        // `prepare_lazy_pipelines`.
        let union_source: std::sync::Arc<str> = backdrop_effect_text(&source.source, true).into();
        shaders::validate_wgsl(&union_source)?;
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("backdrop effect"),
                source: wgpu::ShaderSource::Wgsl(backdrop_effect_text(&source.source, false)),
            });
        // One pipeline per distinct working or scratch format on the plain
        // variant — equal formats share one slot (#170).
        let mut pipelines: FormatCells<Option<Result<wgpu::RenderPipeline, ResourceError>>> =
            Default::default();
        for format in [WORKING_FORMAT, self.scratch_format] {
            let slot = format_slot(format);
            if pipelines[slot].is_none() {
                pipelines[slot] = Some(
                    create_pipeline(
                        &self.device,
                        &self.config,
                        &self.pipeline_layout,
                        self.pipeline_cache.handle.as_ref(),
                        &shaders::wgsl_engine_programs(&module),
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
        let mut plain = <[FormatCells<Option<wgpu::RenderPipeline>>; 2]>::default();
        for (cell, built) in plain[0].iter_mut().zip(pipelines) {
            *cell = built.transpose()?;
        }
        self.backdrop_shaders.insert(
            id.raw(),
            BackdropShaderEntry {
                plain_module: module,
                union_source,
                union_module: None,
                plain,
                union: Default::default(),
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
            &self.queue,
            window.parent,
            size,
            window.output,
            window.probe,
            waker,
        );
        self.planes.insert(id, system);
        self.presenter.get_or_insert_with(|| {
            present::Presenter::with_cache(
                &self.device,
                self.shader_delivery,
                self.pipeline_cache.clone(),
            )
        });
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
        self.presenter.get_or_insert_with(|| {
            present::Presenter::with_cache(
                &self.device,
                self.shader_delivery,
                self.pipeline_cache.clone(),
            )
        });
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
    #[cfg(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32"))]
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
        if let Err(error) = self.ensure_external(&[WORKING_FORMAT, self.scratch_format]) {
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

    /// Builds the external-frame group-1 layout on first need, and the
    /// external pipeline of every format in `formats` that has none yet.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::unnecessary_wraps,
            reason = "browser WebGPU reports pipeline errors asynchronously, so only the native error scope can fail here"
        )
    )]
    fn ensure_external(&mut self, formats: &[wgpu::TextureFormat]) -> Result<(), RenderError> {
        if self.ext_layout.is_some()
            && formats
                .iter()
                .all(|&format| self.external_pipes[format_slot(format)].is_some())
        {
            return Ok(());
        }
        let ext_layout = self.ext_layout.get_or_insert_with(|| {
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("cherenkov external 1"),
                    entries: &layout_entries(bindings::EXTERNAL_GROUP1),
                })
        });
        let module = self.shader_delivery.external_program(&self.device);
        // Error scopes resolve asynchronously; only the native path pops
        // synchronously. A failure here is an engine bug, so the wasm path
        // reports through the uncaptured-error handler instead.
        #[cfg(not(target_arch = "wasm32"))]
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        // One pipeline per distinct format — equal formats share a slot
        // (#170).
        for &format in formats {
            let slot = format_slot(format);
            if self.external_pipes[slot].is_none() {
                self.external_pipes[slot] = Some(create_external_pipeline(
                    &self.device,
                    &self.layout0,
                    ext_layout,
                    &module,
                    format,
                    &self.pipeline_cache,
                ));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(RenderError::Render(format!("external pipeline: {error}")));
        }
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

    /// Builds the projective mip pipeline on the first frame that
    /// composes a projective layer, and the composite pipelines of every
    /// format in `formats` that has none yet.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::unnecessary_wraps,
            reason = "browser WebGPU reports pipeline errors asynchronously, so only the native error scope can fail here"
        )
    )]
    fn ensure_projective(&mut self, formats: &[wgpu::TextureFormat]) -> Result<(), RenderError> {
        if self
            .projective
            .as_ref()
            .is_some_and(|pipelines| formats.iter().all(|&format| pipelines.has(format)))
        {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = self.projective.get_or_insert_with(|| {
            projective::Pipelines::new(
                &self.device,
                self.shader_delivery,
                &self.layout0,
                self.pipeline_cache.clone(),
            )
        });
        for &format in formats {
            pipelines.prepare(&self.device, format);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            self.projective = None;
            return Err(RenderError::Render(format!("projective pipeline: {error}")));
        }
        Ok(())
    }

    /// Bytes of every retained local image.
    fn projective_bytes(&self) -> u64 {
        self.surfaces
            .values()
            .map(SurfaceState::projective_bytes)
            .sum()
    }

    /// Every resident GPU byte except the local images, which are
    /// evictable and give way to everything else.
    fn resident_bytes(&self) -> u64 {
        self.memory().gpu.0 - self.projective_bytes()
    }

    /// The bytes local images may hold: the GPU budget less every other
    /// resident resource.
    fn projective_available(&self) -> u64 {
        self.config
            .budget
            .gpu
            .0
            .saturating_sub(self.resident_bytes())
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
            let surface = self
                .surfaces
                .get(&id)
                .map_or(WORKING_FORMAT, |surf| surf.format);
            self.ensure_projective(&[surface, self.scratch_format, WORKING_FORMAT])?;
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
            surf.command_damage.forget();
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
        surface: &mut SurfaceState,
        sf: &SurfaceFrame<'_>,
        currents: &FxHashMap<ProducerId, &external::Slot>,
    ) -> Result<(planes::Presentation, PlatformCommit), RenderError> {
        let (device, queue, presenter) = present;
        if let Some(system) = system {
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
                        range: surface.part_ranges[n],
                        #[cfg(target_os = "android")]
                        damage: &surface.part_damage[n],
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
                        #[cfg(target_os = "android")]
                        generation: surface.present_generation,
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
        } else {
            surface
                .window
                .as_mut()
                .expect("pending window")
                .content(device, surface.part_ranges[0])
                .map_err(|error| RenderError::Render(error.to_string()))?;
            Ok((
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
            ))
        }
    }

    fn coverage_options(&self) -> (Option<gpu_cov::CoverageLimit>, path::CoverageRoute) {
        let has_compute = self
            .adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            && self.device.limits().max_compute_workgroups_per_dimension > 0
            && self
                .device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        #[cfg(test)]
        if let Some(route) = self.config.coverage_route {
            assert!(
                route != path::CoverageRoute::Gpu || has_compute,
                "forced GPU coverage requires compute and indirect first-instance support"
            );
            return (self.coverage_limit(has_compute), route);
        }
        (
            self.coverage_limit(has_compute),
            path::CoverageRoute::Automatic,
        )
    }

    /// The device's coverage binding limit, when it computes.
    fn coverage_limit(&self, has_compute: bool) -> Option<gpu_cov::CoverageLimit> {
        has_compute.then(|| gpu_cov::CoverageLimit::new(&self.device.limits()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn lower_all(
        &mut self,
        pending: &mut [SurfaceState],
        frames: &[&SurfaceFrame<'_>],
        inputs: &[projective::Inputs],
        mut prepared: FxHashMap<SurfaceId, Lowered>,
    ) -> Vec<Result<Lowered, RenderError>> {
        let coverage_options = self.coverage_options();
        let group_maps: Vec<FxHashMap<u64, BackdropGroupInfo>> = pending
            .iter()
            .map(|surf| surf.backdrop_info(&mut self.filters))
            .collect();
        self.ready_planes(frames, pending);
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        let preparation = &self.preparation;
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
            let outputs = std::thread::scope(|s| {
                pending
                    .iter_mut()
                    .zip(snapshots)
                    .zip(frames)
                    .zip(group_maps.iter().zip(inputs).zip(self.ready_sets.iter()))
                    .map(|(((surf, fonts), frame), ((groups, inputs), ready))| {
                        let prepared = prepared.remove(&frame.id).unwrap_or_default();
                        s.spawn(move || {
                            let result = Self::lower_content(
                                #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
                                preparation,
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
                                coverage_options,
                                prepared,
                            );
                            (result, fonts)
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                    .collect::<Vec<_>>()
            });
            outputs
                .into_iter()
                .map(|(result, fonts)| {
                    for (id, font) in fonts {
                        self.fonts[&id].merge_bounds(font);
                    }
                    result
                })
                .collect()
        } else {
            pending
                .iter_mut()
                .zip(frames)
                .zip(group_maps.iter().zip(inputs).zip(self.ready_sets.iter()))
                .map(|((surf, frame), ((groups, inputs), ready))| {
                    Self::lower_content(
                        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
                        preparation,
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
                        coverage_options,
                        prepared.remove(&frame.id).unwrap_or_default(),
                    )
                })
                .collect()
        };
        // The batch's pending rasters commit transactionally
        // (#169 A3): a dry run decides fit / repack / recycle before any
        // placement or upload happens, so a failed placement never
        // enqueues uploads into an atlas the same preparation abandons.
        self.commit_rasters(pending, &mut results);
        results
    }

    #[cfg(target_arch = "wasm32")]
    fn lower_all(
        &mut self,
        pending: &mut [SurfaceState],
        frames: &[&SurfaceFrame<'_>],
        inputs: &[projective::Inputs],
        mut prepared: FxHashMap<SurfaceId, Lowered>,
    ) -> Vec<Result<Lowered, RenderError>> {
        let coverage_options = self.coverage_options();
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
                    coverage_options,
                    prepared.remove(&frame.id).unwrap_or_default(),
                )
            })
            .collect();
        // The batch's pending rasters commit transactionally
        // (#169 A3): a dry run decides fit / repack / recycle before any
        // placement or upload happens, so a failed placement never
        // enqueues uploads into an atlas the same preparation abandons.
        self.commit_rasters(pending, &mut results);
        results
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

    /// Prepare native-plane content before admission and output classification.
    /// The normal lowering consumes these rasters and command counts, so early
    /// preparation neither repeats work nor loses its transactional uploads.
    fn prepare_plane_content(
        &mut self,
        frame: &Frame<'_>,
    ) -> Result<FxHashMap<SurfaceId, Lowered>, RenderError> {
        let mut prepared = FxHashMap::default();
        if !<planes::Platform as planes::Compositor>::CAPTURES {
            return Ok(prepared);
        }
        for sf in frame.surfaces {
            let surf = self.surfaces.get_mut(&sf.id).expect("registered surface");
            if !surf.promotes || !sf.changed {
                continue;
            }
            let mut lowered = Lowered::default();
            for content in surf.layers.values_mut() {
                lowered.commands += content.retained.prepare(&mut prepared::Lowerer {
                    fonts: &self.fonts,
                    images: &self.images,
                    pending: &mut lowered.pending,
                })?;
            }
            if lowered.commands != 0 {
                prepared.insert(sf.id, lowered);
            }
        }
        Ok(prepared)
    }

    /// Observe content lifetimes without outlining glyphs until admission.
    /// Periodic changes must outlive their previous quiet interval before
    /// another capture is admitted.
    fn observe_static(&mut self, sf: &SurfaceFrame<'_>) -> Result<(), RenderError> {
        let surf = self.surfaces.get_mut(&sf.id).expect("registered surface");
        if !surf.promotes || !<planes::Platform as planes::Compositor>::CAPTURES {
            return Ok(());
        }
        let scale_changed = sf.display.scale.to_bits() != surf.display.scale.to_bits();
        let output_changed =
            scale_changed || sf.display.headroom.to_bits() != surf.display.headroom.to_bits();
        if output_changed {
            for entry in surf.static_layers.values_mut() {
                if scale_changed
                    || (cfg!(not(target_vendor = "apple"))
                        && entry.range == present::ColorRange::HighDynamicRange)
                {
                    entry.capture = None;
                    surf.plan_dirty = true;
                }
            }
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
            let quiet = entry.observe(stamp, resources, density);
            if entry.quiet_frames == 1 {
                entry.range = content_range::of_ops(ops, &self.images);
            }
            let admit = if entry.range == present::ColorRange::HighDynamicRange {
                entry.quiet_frames == 1
            } else {
                quiet
            };
            if admit {
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
    #[expect(
        clippy::too_many_arguments,
        reason = "one surface's prepared inputs and lowering resources"
    )]
    fn lower_content(
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        preparation: &rayon::ThreadPool,
        surf: &mut SurfaceState,
        frame: &SurfaceFrame<'_>,
        resources: GlyphResources<'_>,
        groups: &FxHashMap<u64, BackdropGroupInfo>,
        inputs: &projective::Inputs,
        ready: &FxHashSet<LayerId>,
        coverage_options: (Option<gpu_cov::CoverageLimit>, path::CoverageRoute),
        mut lowered: Lowered,
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
        let result = {
            let glyphs = GlyphContext {
                atlas: resources.atlas,
                live_stamp: resources.atlas.live_stamp(),
                fonts: resources.fonts,
                images: resources.images,
                bitmaps: resources.bitmaps,
                content: &surf.bindings,
            };
            // The GPU coverage route gates on the device's compute
            // capability (issue #2543, scope item 7): the lowering
            // carries the adapter's flag so the #2546 route hook cannot
            // route a compute-less device.
            let mut lowering = Lowering::new(
                &mut surf.frame,
                surf.size,
                &mut surf.anchor_scratch,
                &mut surf.raster_scratch,
                coverage_options.0,
            );
            #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
            let _ = lowering.preparation.insert(preparation);
            lowering.coverage_route = coverage_options.1;
            lowering.pending = std::mem::take(&mut lowered.pending);
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
            lowered.commands += lowering.commands_lowered;
            lowered.layers = lowering.layers_composed;
            lowered.glyphs = lowering.glyphs_rasterized();
            lowered.paths = lowering.paths_rasterized();
            lowered.cell_patches = std::mem::take(&mut lowering.cell_patches);
            lowered.emission_patches = std::mem::take(&mut lowering.emission_patches);
            lowered.mask_patches = std::mem::take(&mut lowering.mask_patches);
            lowered.pending = std::mem::take(&mut lowering.pending);
            lowered.path_arena = std::mem::take(&mut lowering.path_arena);
            result
        };
        if result.is_ok() && (surf.promotes || surf.window.is_some() || surf.ranges.is_some()) {
            surf.classify_parts(frame, &mut layers, resources.images);
        }
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
    /// dry-runs all placements against shelf metadata alone. A `Resize`
    /// verdict repacks surviving coverage into the target shape and
    /// remaps this batch's references (#3302); an exhausted repack
    /// exhausts the batch rather than discard it. When even a repacked
    /// atlas cannot hold the batch the commit reclaims cold shelves in
    /// place, so surviving entries — and the retained emissions
    /// referencing them — stay valid (#119). The upload batches the
    /// committed cells into one `write_texture` per newly allocated
    /// shelf region.
    // `never` so Callgrind can attribute the commit path's inclusive
    // cost — the one-per-frame call boundary is free evidence (#119).
    #[inline(never)]
    fn commit_rasters(
        &mut self,
        pending: &mut [SurfaceState],
        results: &mut [Result<Lowered, RenderError>],
    ) {
        // The frame's GPU coverage context restarts with its lowers
        // (issue #2543).
        self.gpu_cov.begin_frame();
        let rasters: Vec<&glyph::PendingRaster> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .flat_map(|l| l.pending.iter())
            .collect();
        // The frame's live GPU-coverage entries touch before any miss
        // may allocate, so an eviction can never take a block this
        // frame draws (water-rs/waterui#3081).
        let mut draws_coverage = false;
        for raster in &rasters {
            if let glyph::PendingRaster::GpuPath(draw) = raster {
                self.gpu_cov.touch(draw.key);
                draws_coverage = true;
            }
        }
        // One eviction and compaction pass per commit, ahead of every
        // miss's allocation.
        self.gpu_cov.settle(&self.device, draws_coverage);
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
            touches_len = self.replay_pins(pending, results, &mut touches);
            self.atlas.begin_commit(&touches);
            touches.clear();
            self.commit_touches = touches;
            match self.atlas.plan(&rasters) {
                glyph::AtlasPlan::Fits | glyph::AtlasPlan::FitsEviction => "fits-eviction",
                glyph::AtlasPlan::Resize(size, pages) => {
                    if !self.repack_atlas(size, pages, pending, results) {
                        return;
                    }
                    "relocated"
                }
                glyph::AtlasPlan::Recycle => {
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
        // The upload consumes its batch and returns the emptied
        // container, so `commit_writes` always comes back empty and the
        // next commit refills the same lists (#2671).
        let mut writes = std::mem::take(&mut self.commit_writes);
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
        self.commit_writes = self
            .atlas
            .upload_committed(&self.device, &self.queue, writes);
    }

    /// `AtlasPlan::Resize`'s cold path: repacks surviving coverage into
    /// the target shape, remaps this batch's references to the new
    /// origins, and queues the copies ahead of the frame's uploads
    /// (#3302). `false` — every still-`Ok` result marked
    /// [`RenderError::AtlasExhausted`] — when even the surviving set
    /// does not fit; the repack issued no copy work then.
    fn repack_atlas(
        &mut self,
        size: u32,
        pages: u32,
        pending: &mut [SurfaceState],
        results: &mut [Result<Lowered, RenderError>],
    ) -> bool {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("atlas relocation"),
            });
        let Some(relocation) = self.atlas.relocate(&self.device, &mut encoder, size, pages) else {
            Self::discard_unapplied(pending, results);
            return false;
        };
        for (surf, result) in pending.iter_mut().zip(results.iter()) {
            let Ok(lowered) = result else {
                continue;
            };
            Self::remap_surface(&relocation, surf, lowered);
        }
        // An atlas that migrated no cells — the empty-atlas grow —
        // recorded no copies, so there is nothing to submit.
        if relocation.moved() > 0 {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            let _submit = self.submit_lock.lock().expect("submit guard");
            self.queue.submit([encoder.finish()]);
            track_submission(&self.queue, &self.tracker);
            diag::submit(&self.device, &self.queue, "atlas relocation");
        }
        true
    }

    /// Re-points one surface's retained and frame UVs at the repack's
    /// new origins. Every live emission's sampled cells were pinned
    /// through `replay_pins` or the pending batch, so
    /// [`glyph::Relocation::cell`] never misses for them — a miss is an
    /// invariant violation. Emissions whose old references were already
    /// stale drop; the survivors' refs are rebuilt at apply from the
    /// remapped coordinates (#3302).
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "atlas UVs are nonnegative integer texel origins stored as f32 — exact both directions"
    )]
    fn remap_surface(relocation: &glyph::Relocation, surf: &mut SurfaceState, lowered: &Lowered) {
        for content in surf.layers.values_mut() {
            let (_, emissions) = content.retained.prepared();
            for entry in emissions.iter_mut() {
                let Some(emission) = entry.data.as_ref() else {
                    continue;
                };
                if !content.storage.refs[emission.refs_range()]
                    .iter()
                    .all(|&(slot, epoch)| relocation.old_ref_live(slot, epoch))
                {
                    entry.data = None;
                }
            }
            // The dead ranges above are never read again — every
            // surviving emission's pairs are refolded at apply — so
            // the arena refills rather than accumulating stale pairs.
            content.storage.refs.clear();
            for entry in emissions.iter_mut() {
                let Some(emission) = entry.data.as_mut() else {
                    continue;
                };
                // Unplaced pending cells still hold `(0, 0)`: their
                // deferred patches name the new origins instead (#119).
                let pending = emission
                    .pending_cells()
                    .map(|i| (lowered.emission_patches[i].0 - emission.cell_inst_base()) as usize);
                let instances = emission.instances.clone();
                for inst in Self::placed(content.storage.instances[instances].iter_mut(), pending) {
                    if matches!(inst.kind, instance::KIND_GLYPH | instance::KIND_REGION) {
                        let loc = relocation
                            .cell(inst.uv[0] as u32, inst.uv[1] as u32)
                            .expect("a pinned emission's cells migrate with the repack");
                        inst.uv = [loc.x as f32, loc.y as f32];
                    }
                }
                emission.refs = lower::DEFERRED_REFS;
            }
        }
        let cell_pending = lowered.cell_patches.iter().map(|&(i, ..)| i as usize);
        for inst in Self::placed(surf.frame.instances.iter_mut(), cell_pending) {
            if matches!(inst.meta[0], instance::KIND_GLYPH | instance::KIND_REGION) {
                let loc = relocation
                    .cell(inst.uv[0] as u32, inst.uv[1] as u32)
                    .expect("a pinned cell migrates with the repack");
                inst.uv[..2].copy_from_slice(&[loc.x as f32, loc.y as f32]);
            }
        }
        let mask_pending = lowered.mask_patches.iter().map(|&(i, _)| i as usize);
        for inst in Self::placed(surf.frame.instances.iter_mut(), mask_pending) {
            let flags = inst.meta[3] >> 24;
            if flags & instance::FLAG_HAS_MASK != 0 && flags & instance::FLAG_MASK_TEXTURE == 0 {
                let loc = relocation
                    .cell(inst.uv[2] as u32, inst.uv[3] as u32)
                    .expect("a pinned mask cell migrates with the repack");
                inst.uv[2..].copy_from_slice(&[loc.x as f32, loc.y as f32]);
            }
        }
    }

    /// The shelf slots the commit must pin: the shelves each new
    /// emission's glyph instances sample plus the clip masks bound
    /// through `uv[2..3]` — all recovered from stored UVs, so the
    /// lowering records no slot list (#119).
    fn replay_pins(
        &self,
        pending: &mut [SurfaceState],
        results: &[Result<Lowered, RenderError>],
        touches: &mut Vec<u32>,
    ) -> usize {
        for (surf, result) in pending.iter_mut().zip(results.iter()) {
            // A failed lowering leaves retained emissions whose
            // pending cells index a patch list that never applies —
            // the surface is discarded below, so none of its pins may
            // stand (#119).
            let Ok(lowered) = result else {
                continue;
            };
            for content in surf.layers.values_mut() {
                let (_, emissions) = content.retained.prepared();
                for emission in emissions.iter().filter_map(|e| e.data.as_ref()) {
                    // An emission's pending cells carry no atlas
                    // origin until `apply_pending` places them — their
                    // stored UVs are still `(0, 0)`, so resolving one
                    // would pin whatever shelf covers the atlas
                    // origin. Their patch range names them already,
                    // ascending by instance (#2440).
                    let pending = emission.pending_cells().map(|i| {
                        (lowered.emission_patches[i].0 - emission.cell_inst_base()) as usize
                    });
                    Self::for_each_glyph_shelf(
                        &self.atlas,
                        Self::placed(
                            content.storage.instances[emission.instances.clone()].iter(),
                            pending,
                        ),
                        |slot| touches.push(slot),
                    );
                    // One stored UV names only the shelf its own quad
                    // samples, but an emission's band set is wider: a
                    // region interval widened across cells can span
                    // shelves. The `(slot, epoch)` refs folded at
                    // apply hold every band the emission samples, so
                    // the still-live ones pin too (#2909). A deferred
                    // `refs` has no pairs yet — apply derives them,
                    // and every cell they will name is already pinned
                    // by the UV scan above.
                    if emission.refs & lower::DEFERRED_REFS == 0 {
                        touches.extend(
                            content.storage.refs[emission.refs_range()]
                                .iter()
                                .filter(|&&(slot, epoch)| self.atlas.shelf_matches(slot, epoch))
                                .map(|&(slot, _)| slot),
                        );
                    }
                }
            }
            // Clip masks bound through `uv[2..3]` pin the same way:
            // the frame's masked instances recover their shelf from
            // the stored atlas coordinates. A mask still pending its
            // placement keeps the unpatched `(0, 0)` there — the
            // lowering's `mask_patches` names those instances,
            // ascending by slot (#2440).
            let pending = lowered.mask_patches.iter().map(|&(inst, _)| inst as usize);
            for inst in Self::placed(surf.frame.instances.iter(), pending) {
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

    /// Drops the elements of `items` whose indices appear in
    /// `unplaced` — an ascending set of positions whose stored UVs
    /// still lack an atlas origin: pending cells and pending clip
    /// masks patch it in at apply, so resolving them pins the shelf
    /// covering `(0, 0)`, not one they sampled (#2440).
    fn placed<T>(
        items: impl Iterator<Item = T>,
        mut unplaced: impl Iterator<Item = usize>,
    ) -> impl Iterator<Item = T> {
        let mut next = unplaced.next();
        items.enumerate().filter_map(move |(i, item)| {
            if next == Some(i) {
                next = unplaced.next();
                None
            } else {
                Some(item)
            }
        })
    }

    /// The shelves `instances`' glyph quads sample, recovered from their
    /// stored UVs — the leaf records no per-cell slot at emit time, so
    /// the commit derives them here instead (#119).
    fn for_each_glyph_shelf<'a>(
        atlas: &Atlas,
        instances: impl Iterator<Item = &'a lower::RetainedInstance>,
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
        writes: &mut glyph::CellWrites,
    ) -> Result<(), RenderError> {
        let pending = std::mem::take(&mut lowered.pending);
        // The batch's texel arena moves into the commit's writes, which
        // keep it alive until the upload below has consumed its bytes.
        let arena = writes.push_arena(std::mem::take(&mut lowered.path_arena));
        let mut origins = std::mem::take(&mut self.pending_origins);
        origins.clear();
        for raster in pending {
            origins.push(self.apply_raster(raster, arena, writes)?);
        }
        // DrawRange.gpu records a pending index; resolve it to the
        // compute record `apply_raster` assigned (issue #2543).
        for range in surf
            .frame
            .passes
            .iter_mut()
            .flat_map(|pass| pass.ranges.iter_mut())
        {
            if let Some(p) = range.gpu {
                let PendingOrigin::Gpu(index) = origins[p as usize] else {
                    unreachable!("gpu draw range must reference a GpuPath raster");
                };
                range.gpu = Some(index);
            }
        }
        // Admissions can move the atlas's map entries. Borrow only after
        // the whole batch is placed; its touched shelves remain pinned.
        let paths: Vec<_> = origins
            .iter()
            .map(|origin| match origin {
                PendingOrigin::Path(key) => Some(self.atlas.path(*key).expect("just stored")),
                _ => None,
            })
            .collect();
        let cell_origin = |p: u32, c: u32| match &origins[p as usize] {
            PendingOrigin::Glyph((x, y), _) => [*x as f32, *y as f32],
            PendingOrigin::Path(_) => {
                let cell = &paths[p as usize].expect("admitted path").cells[c as usize];
                [f32::from(cell.x), f32::from(cell.y)]
            }
            _ => unreachable!("cell patch must reference cell raster"),
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
                    let mut previous_pending = None;
                    for i in emission.pending_cells() {
                        let (inst, p, c) = lowered.emission_patches[i];
                        let local = inst - emission.cell_inst_base();
                        content.storage.instances[emission.instances.start + local as usize].uv
                            [..2]
                            .copy_from_slice(&cell_origin(p, c));
                        // Path cell patches are consecutive. Register their
                        // complete shelf set once instead of once per cell.
                        if previous_pending != Some(p) {
                            let bands = match &origins[p as usize] {
                                PendingOrigin::Glyph(_, slot) => slot.as_slice(),
                                PendingOrigin::Path(_) => self.atlas.emit_slot_arena(
                                    paths[p as usize].expect("admitted path").slots.clone(),
                                ),
                                _ => unreachable!("cell patch must reference cell raster"),
                            };
                            content.storage.refs.extend(
                                bands
                                    .iter()
                                    .map(|&slot| (slot, self.atlas.shelf_epoch(slot))),
                            );
                        }
                        previous_pending = Some(p);
                    }
                    // Glyph cells record no slot at emit time: the shelf
                    // each stored UV samples is recovered here instead
                    // (#119).
                    Self::for_each_glyph_shelf(
                        &self.atlas,
                        content.storage.instances[emission.instances.clone()].iter(),
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
                    .all(|&(s, ep)| self.atlas.shelf_matches(s, ep));
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
        arena: u32,
        writes: &mut glyph::CellWrites,
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
                // An outline-less glyph owns no cell, so its admission
                // references no band (#2327).
                let out = self
                    .atlas
                    .place_glyph(key, left, top, w, h, texels, writes)
                    .map(|(origin, shelf)| PendingOrigin::Glyph(origin, shelf))
                    .ok_or(RenderError::AtlasFull)?;
                if !hit && w == 0 {
                    diag::atlas_cell(&self.device, (0, 0, 0, 0));
                }
                Ok(out)
            }
            PendingRaster::Path { key, emit, cells } => {
                self.atlas
                    .place_path(key, emit, cells, arena, writes)
                    .ok_or(RenderError::AtlasFull)?;
                Ok(PendingOrigin::Path(key))
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
                upload,
            } => {
                if !self.bitmaps.contains_key(&key) {
                    let image = create_gpu_image(
                        &self.device,
                        &self.queue,
                        "bitmap glyph",
                        (width, height),
                        &upload,
                    );
                    self.bitmaps.insert(key, GpuBitmap { image, em });
                }
                Ok(PendingOrigin::None)
            }
            PendingRaster::GpuPath(draw) => {
                let index = self.gpu_cov.draw(&self.device, &draw)?;
                Ok(PendingOrigin::Gpu(index))
            }
        }
    }

    /// What this engine's tile executor can hold, or why there is none
    /// to plan for (issues #2549, #2191).
    ///
    /// Both executors bind four slots — slot zero stores the epoch's
    /// output, the other three carry live intermediates — in
    /// `Rgba16Float`, and both order a slot's accesses per pixel within
    /// a pass, so a frame whose isolation scratches are stored in
    /// another format can never place one on chip and is not planned at
    /// all. An `Rgba32Float` surface is planned: the planner keeps its
    /// parts out of every epoch for [`composition::Materialization::Format`]. What differs is what makes the executor available:
    ///
    /// - Apple's shaders come from the prebuilt metallib (#57), so a
    ///   build that delivers WGSL has no executor to plan for.
    /// - Vulkan's need the device to have been opened with per-pixel
    ///   attachment access and to have lazily allocated memory for the
    ///   slots; [`GpuRenderer::tile_mirror`] holds the mirror exactly
    ///   when both hold, and otherwise the capability that is missing.
    ///
    /// The error is the reason the frame's images are attributed to, so
    /// a census distinguishes "this build has no executor" from "this
    /// device cannot run one".
    #[cfg_attr(
        not(unix),
        expect(
            clippy::unused_self,
            reason = "tile executor state exists only on Unix"
        )
    )]
    fn tile_limits(&self) -> Result<composite::plan::Limits, composition::Materialization> {
        // Which executor this target has, and whether it is installed.
        // The mirror is absent only because the Vulkan device declined,
        // so the capability it lacks is the reason; a non-metallib
        // Apple build and a target with no executor at all share
        // `NoExecutor`.
        #[cfg(target_vendor = "apple")]
        let (slots, available) = (
            composite::metal::SLOTS,
            (self.shader_delivery == shaders::ShaderDelivery::Metallib)
                .then_some(())
                .ok_or(composition::Materialization::NoExecutor),
        );
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let (slots, available) = (
            composite::vulkan::SLOTS,
            self.tile_mirror
                .as_ref()
                .map(|_| ())
                .map_err(|declined| match declined {
                    composite::vulkan::Declined::Access(_) => {
                        composition::Materialization::DeviceAccess
                    }
                    composite::vulkan::Declined::TransientMemory => {
                        composition::Materialization::TransientMemory
                    }
                }),
        );
        #[cfg(not(unix))]
        let (slots, available) = (0, Err(composition::Materialization::NoExecutor));

        available?;
        Ok(composite::plan::Limits {
            slots,
            format: WORKING_FORMAT,
            ordering: composite::plan::Ordering::PerPixel,
        })
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
        // What the backend's tile executor can hold, read before the
        // surface is borrowed for the frame's attachment planning.
        let tile_limits = self.tile_limits();
        // Apply this surface's frame and plan materialized image storage.
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        for (&layer, entry) in &mut surf.static_layers {
            if !surf.plan.planes.iter().any(|plane| plane.layer == layer) {
                entry.capture = None;
                continue;
            }
            if entry.capture.is_none() {
                surf.command_damage.forget();
                let domain = entry.domain.expect("a static candidate has a domain");
                let (texture, view) = create_target(
                    &self.device,
                    "static plane capture",
                    domain.size,
                    TARGET_USAGES,
                    surf.format,
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
        let part_bytes = u64::from(surf.size.0) * u64::from(surf.size.1) * texel_bytes(surf.format);
        while surf.parts.len() < parts {
            surf.parts.push(create_target(
                &self.device,
                "engine part",
                surf.size,
                TARGET_USAGES,
                surf.format,
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
        // Damage dependencies exist on every backend; tile limits only
        // determine whether those images can stay in a native epoch.
        surf.composition_cache.update(
            &mut surf.composition,
            &surf.frame,
            composite::plan::Formats {
                content: self.config.scratch_format == ScratchFormat::Content,
                surface: surf.format,
                scratch: self.scratch_format,
            },
            tile_limits,
        );
        surf.command_damage
            .update_resources(&surf.frame, |range| match &range.image {
                Some(lower::ImageSource::Content(id)) => self
                    .producers
                    .get(id)
                    .and_then(gpu_content::Producer::current)
                    .map_or(0, |slot| slot.generation),
                Some(lower::ImageSource::Registered(id)) => self.images[id].revision,
                _ => 0,
            });
        for (index, key) in &surf.frame.filters {
            if self.filters.wants_redraw(*key) {
                surf.command_damage.invalidate_pass(&surf.frame, *index);
            }
        }
        for (index, pass) in surf.frame.passes.iter().enumerate() {
            for range in &pass.ranges {
                if let Some(lower::ImageSource::Shader(key)) = &range.image
                    && self.shaders.animated(key)
                {
                    surf.command_damage
                        .invalidate_range(&surf.frame, index, range);
                }
            }
        }
        surf.propagated_damage
            .update(&surf.frame, &surf.composition, |index, region| {
                for rect in surf.command_damage.pass(index).rectangles() {
                    let (x, y, w, h) = rect.xywh();
                    region.add(x, y, w, h);
                }
            });
        tracing::trace!(
            command_invalidated_pass_pixels = surf.command_damage.pixels(),
            "command damage before effect propagation"
        );
        surf.draw_demand
            .update(&surf.frame, &surf.composition, |index, _, _, required| {
                for rect in surf.propagated_damage.pass(index).rectangles() {
                    let (x, y, w, h) = rect.xywh();
                    required.add(x, y, w, h);
                }
            });
        surf.repaint_clears
            .prepare(&mut surf.frame, &surf.draw_demand);
        let region_max: Vec<_> = surf
            .composition
            .scratch
            .iter()
            .map(|slot| (slot.size, slot.format))
            .collect();
        // Reuse storage within a format; a format change replaces the texture.
        let mut retired_slots = Vec::new();
        surf.scratch.retain(|slot, texture| {
            let keep = region_max
                .get(*slot)
                .is_some_and(|(size, _)| *size != (0, 0));
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
                retired_slots.push(*slot);
            }
            keep
        });
        if !retired_slots.is_empty() {
            surf.bind_gen += 1;
            retire_binds1(
                surf,
                &self.device,
                self.images_gen,
                self.atlas.mask_texture_generation(),
                "tile lifetime",
                |key| matches!(key.0, Some(Source::Scratch(i)) if retired_slots.contains(&i)),
            );
        }
        for (i, &((w, h), format)) in region_max.iter().enumerate() {
            if w == 0 || h == 0 {
                continue;
            }
            let (nw, nh) = (
                w.max(
                    surf.scratch
                        .get(&i)
                        .filter(|s| s.texture.format() == format)
                        .map_or(0, |s| s.width),
                ),
                h.max(
                    surf.scratch
                        .get(&i)
                        .filter(|s| s.texture.format() == format)
                        .map_or(0, |s| s.height),
                ),
            );
            if surf
                .scratch
                .get(&i)
                .is_some_and(|s| s.width >= w && s.height >= h && s.texture.format() == format)
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
                format,
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
                u64::from(nw) * u64::from(nh) * texel_bytes(format),
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
        let mut staged_max = [None; RENDER_FORMATS.len()];
        for (index, pass) in surf.frame.passes.iter().enumerate() {
            let Some(capture) = pass.capture else {
                continue;
            };
            let (w, h) = (pass.region[2], pass.region[3]);
            let format = stored_format(capture.copy_from, surf, index)?;
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
                let entry = staged_max[format_slot(format)].get_or_insert((format, 0, 0));
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
        let mut backdrop_max = [(0u32, 0u32); RENDER_FORMATS.len()];
        let passes = surf
            .frame
            .passes
            .iter()
            .enumerate()
            .filter_map(|(index, pass)| {
                (!surf.composition.contains_native_pass(index)).then_some((index, pass))
            });
        for (index, pass) in passes {
            if let Some(r) = pass.backdrop_copy {
                let slot = blend_backdrop_slot(pass.target, surf, index)?;
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
            let format = RENDER_FORMATS[slot];
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
                |key| key.1 == Some(slot),
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
        let mut gpu_draws = std::mem::take(&mut self.gpu_draw_ranges);
        gpu_draws.clear();
        gpu_draws.extend(
            surf.frame
                .passes
                .iter()
                .flat_map(|pass| &pass.ranges)
                .filter_map(|range| range.gpu.map(|index| (index, range.instances.start))),
        );
        for &(index, anchor_index) in &gpu_draws {
            self.gpu_cov.write_draw(
                index,
                inst_base + anchor_index,
                &mut surf.frame.instances[anchor_index as usize],
            );
        }
        self.gpu_draw_ranges = gpu_draws;
        surf.inst_base = inst_base;
        surf.globals_base = globals_base;
        // Appended `KIND_POLYGON` slots store the painter index locally;
        // shift it into the shared frame buffer's coordinates.
        for range in surf.frame.passes.iter().flat_map(|p| p.tier_ranges.iter()) {
            for index in range.clone() {
                let inst = &mut surf.frame.instances[usize::try_from(index).unwrap_or(usize::MAX)];
                inst.grad[0] = f32::from_bits(inst.grad[0].to_bits() + inst_base);
            }
        }
        {
            let atlas = &self.atlas;
            if atlas.generation() != self.bound_atlas {
                diag::bind_groups_dropped(&self.device, 1, "atlas generation");
                self.bind0 = make_bind0(
                    &self.device,
                    &self.layout0,
                    &EngineBuffers {
                        globals: &self.globals,
                        instances: &self.instances,
                        stops: &self.stops,
                        arena: self.gpu_cov.arena().unwrap_or(&self.arena_placeholder),
                        image_decode: &self.image_decode,
                    },
                    atlas,
                );
                self.bound_atlas = atlas.generation();
                self.bound_arena_gen = self.gpu_cov.arena_generation();
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
                &EngineBuffers {
                    globals: &self.globals,
                    instances: &self.instances,
                    stops: &self.stops,
                    arena: self.gpu_cov.arena().unwrap_or(&self.arena_placeholder),
                    image_decode: &self.image_decode,
                },
                atlas,
            );
            self.bound_atlas = atlas.generation();
            self.invalidate_painter_replays();
            self.bound_instance_size = self.instances.size();
            self.bound_stop_size = self.stops.size();
            self.bound_globals_size = self.globals.size();
            self.bound_arena_gen = self.gpu_cov.arena_generation();
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
            let coverage_passes = u32::from(self.gpu_cov.has_misses());
            self.ensure_query_capacity(
                2 * (u32::try_from(passes).expect("frame pass count fits u32") + coverage_passes),
            );
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
        // The frame this surface is encoded for: the composition
        // attribution's key (#2547), recorded for a planned frame.
        frame: cherenkov::FrameId,
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
        // The formats this surface's passes draw into: its parts', its
        // scratch layers' and its projected images'.
        let Some(surface_format) = self.surfaces.get(&id).map(|surf| surf.format) else {
            return Ok(());
        };
        let formats = [surface_format, self.scratch_format, WORKING_FORMAT];
        let needs_external = self.surfaces[&id]
            .frame
            .passes
            .iter()
            .flat_map(|pass| &pass.ranges)
            .any(|range| matches!(range.image, Some(lower::ImageSource::Content(_))));
        if needs_external {
            self.ensure_external(&formats)?;
        }
        if self
            .surfaces
            .get(&id)
            .is_some_and(|surf| !surf.composed.is_empty())
        {
            self.ensure_projective(&formats)?;
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
                &EngineBuffers {
                    globals: &self.globals,
                    instances: &self.instances,
                    stops: &self.stops,
                    arena: self.gpu_cov.arena().unwrap_or(&self.arena_placeholder),
                    image_decode: &self.image_decode,
                },
                atlas,
            );
            self.bound_atlas = atlas.generation();
            self.bound_instance_size = self.instances.size();
            self.bound_stop_size = self.stops.size();
            self.bound_globals_size = self.globals.size();
            self.bound_arena_gen = self.gpu_cov.arena_generation();
            self.invalidate_painter_replays();
        }
        // The GPU coverage route (issues #2543, #3081): validate the
        // frame's arena requirement and size the arena and draws
        // buffer before the prologue lands on the encoder below; a
        // moved arena leaves `bind0` stale.
        let gpu_active = self.gpu_cov.has_draws();
        if gpu_active {
            self.gpu_cov.validate_output_sizes(&self.device.limits())?;
            self.gpu_cov.ensure(&self.device);
            if self.gpu_cov.arena_generation() != self.bound_arena_gen {
                let atlas = &self.atlas;
                diag::bind_groups_dropped(&self.device, 1, "coverage arena");
                self.bind0 = make_bind0(
                    &self.device,
                    &self.layout0,
                    &EngineBuffers {
                        globals: &self.globals,
                        instances: &self.instances,
                        stops: &self.stops,
                        arena: self.gpu_cov.arena().expect("coverage arena sized"),
                        image_decode: &self.image_decode,
                    },
                    atlas,
                );
                self.bound_atlas = atlas.generation();
                self.bound_arena_gen = self.gpu_cov.arena_generation();
                self.invalidate_painter_replays();
            }
        }
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        #[cfg(target_os = "android")]
        {
            surf.part_damage.resize_with(surf.plan.parts(), || {
                cherenkov::damage::Region::new(surf.size)
            });
            for region in &mut surf.part_damage {
                if region.extent() == surf.size {
                    region.clear();
                } else {
                    *region = cherenkov::damage::Region::new(surf.size);
                }
            }
        }
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
            cache: self.pipeline_cache.handle.as_ref(),
            delivery: self.shader_delivery,
        };
        surf.painter_replays
            .retain(|index, _| *index < surf.frame.passes.len());
        let mask_gen = self.atlas.mask_texture_generation();
        // Decision S2: the whole frame's coverage dispatch runs once,
        // in this encoder ahead of the raster passes — whichever dirty
        // surface is encoded first records it (#2543).
        if self.gpu_cov.needs_prologue() {
            // Only a coverage miss records a compute pass: all-hit
            // frames need just the count-word copies (#3081).
            let misses = self.gpu_cov.has_misses();
            let first_query = self.query_base + 2 * self.frame_pass_count;
            let timestamp_writes = self.query_set.as_ref().filter(|_| misses).map(|query_set| {
                wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(first_query),
                    end_of_pass_write_index: Some(first_query + 1),
                }
            });
            if misses {
                self.frame_pass_count += 1;
                if self.timestamps {
                    self.pass_meta.push(PassMeta {
                        name: "coverage".to_owned(),
                        width: 0,
                        height: 0,
                        format: "compute",
                    });
                }
            }
            self.gpu_cov.record_prologue(
                &self.device,
                &self.queue,
                &mut encoder,
                self.shader_delivery,
                timestamp_writes,
            )?;
        }
        let mut i = 0;
        // The pyramid steps' globals-slot index within this surface:
        // `write_uploads` lays them out in pass and level order after the
        // pass slots.
        let mut reduce_slot = 0u32;
        // Composition attribution (#2547) collects only with a sink
        // installed; the flag is the whole cost of the disabled path.
        #[cfg(unix)]
        let attribute = self.config.composition_diag.is_some();
        // Only an executor creates tile pipelines, so where there is
        // none this stays the empty list the report carries (#2549).
        #[cfg(unix)]
        let mut tile_pipelines = Vec::new();
        #[cfg(not(unix))]
        let tile_pipelines = Vec::new();
        // The Vulkan executor's frame: its descriptor pools are taken
        // for the duration of this surface's own submission, and the
        // resource identities its sets are written against are frozen
        // here. A surface with no epoch to record opens none, so an
        // unplanned frame costs nothing (#2191).
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let tile_frame = match self
            .tile_mirror
            .as_mut()
            .ok()
            .filter(|_| surf.composition.epochs().next().is_some())
        {
            Some(mirror) => {
                mirror
                    .begin_frame(composite::vulkan::mirror::Stamp {
                        bind_gen: surf.bind_gen,
                        images_gen: self.images_gen,
                        mask_gen,
                    })
                    .map_err(|e| RenderError::Render(format!("tile composition: {e}")))?;
                true
            }
            None => false,
        };
        while i < surf.frame.passes.len() {
            let end = surf
                .composition
                .epoch_at(i)
                .map_or(i + 1, |epoch| epoch.passes.end);
            if (i..end).all(|pass| surf.draw_demand.pass(pass).is_empty()) {
                reduce_slot += surf
                    .frame
                    .reduces
                    .iter()
                    .filter(|(pass, _)| (i..end).contains(pass))
                    .map(|(_, levels)| levels - 1)
                    .sum::<u32>();
                i = end;
                continue;
            }
            #[cfg(all(unix, not(target_vendor = "apple")))]
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
                #[cfg(target_os = "android")]
                let written = {
                    let first_output = epoch
                        .ops
                        .iter()
                        .find(|op| op.target == 0)
                        .expect("an epoch writes its output");
                    (
                        surf.composition.images[epoch.output.0].target,
                        first_output.pass,
                        epoch.region,
                    )
                };
                composite::vulkan::exec::encode(
                    self.tile_mirror.as_mut().expect("the epoch was planned"),
                    &composite::vulkan::exec::Resources {
                        device: &self.device,
                        globals: &self.globals,
                        instances: &self.instances,
                        stops: &self.stops,
                        image_decode: &self.image_decode,
                        dummy: &self.dummy_view,
                        dummy_texture: &self.dummy,
                        images: &self.images,
                        bitmaps: &self.bitmaps,
                        atlas: &self.atlas,
                    },
                    surf,
                    self.images_gen,
                    i,
                    composite::vulkan::exec::Recording {
                        encoder: &mut encoder,
                        buffers: &mut buffers,
                        timestamps: self.query_set.as_ref().map(|query_set| {
                            (
                                query_set,
                                self.query_base + 2 * pass_index,
                                self.query_base + 2 * pass_index + 1,
                            )
                        }),
                        stats,
                        pipelines: attribute.then_some(&mut tile_pipelines),
                    },
                )
                .map_err(|e| RenderError::Render(format!("tile composition: {e}")))?;
                #[cfg(target_os = "android")]
                if let (Target::Part(part), first, region) = written {
                    for [x, y, w, h] in
                        repaint::Scissors::new(surf.draw_demand.pass(first), region).rectangles()
                    {
                        surf.part_damage[part as usize].add(x + region[0], y + region[1], w, h);
                    }
                }
                stats.passes += 1;
                i = end;
                continue;
            }
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
                    composite::metal::Recording {
                        encoder: &mut encoder,
                        timestamps: self.query_set.as_ref().map(|query_set| {
                            wgpu::RenderPassTimestampWrites {
                                query_set,
                                beginning_of_pass_write_index: Some(
                                    self.query_base + 2 * pass_index,
                                ),
                                end_of_pass_write_index: Some(self.query_base + 2 * pass_index + 1),
                            }
                        }),
                        stats,
                        pipelines: attribute.then_some(&mut tile_pipelines),
                    },
                );
                stats.passes += 1;
                i = end;
                continue;
            }
            // First reduced capture of the engine's life builds the
            // resolve pipelines here, like the shadow blur (#170).
            let resolve_draw = if resolve::of(&surf.frame.passes[i]).is_some() {
                let pipelines = self.resolve.get_or_insert_with(|| {
                    resolve::Pipelines::new(
                        &self.device,
                        self.shader_delivery,
                        self.pipeline_cache.clone(),
                    )
                });
                prepare_resolve(&self.device, pipelines, &self.globals, surf, i)?
            } else {
                None
            };
            let pass = &surf.frame.passes[i];
            let coverage_order = coverage_order(pass);
            let replayable = !pass.ranges.iter().any(|range| range.gpu.is_some())
                && !coverage_order
                && !pass.ranges.is_empty()
                && pass.ranges.iter().all(|range| {
                    range.pipeline == PipelineKind::SrcOver
                        && matches!(range.variant, ShaderVariant::Simple | ShaderVariant::Shadow)
                        && range.source.is_none()
                        && range.image.is_none()
                        && range.mask.is_none()
                        // Indirect draws are recorded per frame — a
                        // replayed bundle would bind stale arguments.
                        && range.gpu.is_none()
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
            #[cfg(target_os = "android")]
            if let Target::Part(part) = pass.target
                && (pass.clear.is_some() || !pass.ranges.is_empty())
            {
                for [x, y, w, h] in
                    repaint::Scissors::new(surf.draw_demand.pass(i), pass.region).rectangles()
                {
                    surf.part_damage[part as usize].add(
                        x + pass.region[0],
                        y + pass.region[1],
                        w,
                        h,
                    );
                }
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
                Target::Scratch(_) => {
                    let slot = surf.composition.scratch_slot(i, pass.target);
                    (&surf.scratch[&slot].view, &surf.scratch[&slot].texture)
                }
                // A staged resolve's draws land in the staging texture.
                Target::Backdrop { group, region } => {
                    let group_state = &surf.backdrop_groups[&group];
                    let capture = if resolve::of(pass).is_some() && resolve::staged(pass) {
                        let slot = format_slot(stored_format(
                            pass.capture
                                .expect("a staged resolve implies a capture")
                                .copy_from,
                            surf,
                            i,
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
                let (src, sx, sy) = match capture.copy_from {
                    Target::Plane(layer) => (
                        &surf.static_layers[&layer]
                            .capture
                            .as_ref()
                            .expect("allocated capture")
                            .source
                            .as_ref()
                            .expect("capture source allocated before encode")
                            .0,
                        0,
                        0,
                    ),
                    Target::Part(n) => (surf.part(n).1, 0, 0),
                    Target::Projected(key) => {
                        (&projective_entry(&surf.projective, key).texture, 0, 0)
                    }
                    Target::Scratch(_) => (
                        &surf.scratch[&surf.composition.scratch_slot(i, capture.copy_from)].texture,
                        0,
                        0,
                    ),
                    Target::Backdrop { group, region } => {
                        let capture = &surf.backdrop_groups[&group].captures[region as usize];
                        (&capture.target.texture, 0, 0)
                    }
                };
                debug_assert!(draw_region[0] + draw_region[2] <= src.width());
                debug_assert!(draw_region[1] + draw_region[3] <= src.height());
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: src,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: draw_region[0].saturating_sub(sx),
                            y: draw_region[1].saturating_sub(sy),
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
                let slot = blend_backdrop_slot(pass.target, surf, i)?;
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
            let texture = texture.clone();
            let view = view.clone();
            let (texture, view) = (&texture, &view);
            let partial_clear = surf.repaint_clears.instance(i);
            let load = match pass.clear.filter(|_| partial_clear.is_none()) {
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
            let begin_index = self.query_base + 2 * pass_index;
            let in_pass_begin =
                (!self.pass_start_on_encoder && partial_clear.is_none()).then_some(begin_index);
            let in_pass_end = (!native_run).then_some(begin_index + 1);
            let timestamp_writes = self
                .query_set
                .as_ref()
                .filter(|_| in_pass_begin.or(in_pass_end).is_some())
                .map(|qs| wgpu::RenderPassTimestampWrites {
                    query_set: qs,
                    beginning_of_pass_write_index: in_pass_begin,
                    end_of_pass_write_index: in_pass_end,
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
            if self.pass_start_on_encoder
                && let Some(query_set) = &self.query_set
            {
                encoder.write_timestamp(query_set, begin_index);
            }
            if let Some(instance) = partial_clear {
                let bind1 = surf
                    .binds1
                    .entry((None, None, None, None))
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
                repaint::ClearDraw {
                    timestamps: self
                        .query_set
                        .as_ref()
                        .filter(|_| !self.pass_start_on_encoder)
                        .map(|query_set| wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: Some(begin_index),
                            end_of_pass_write_index: None,
                        }),
                    pipeline: core_pipeline(
                        &mut self.pipelines,
                        &factory,
                        texture.format(),
                        true,
                        ShaderVariant::Simple,
                    )?,
                    bind0: &self.bind0,
                    bind1,
                    offset: (surf.globals_base + u32::try_from(i).expect("pass index")) * 256,
                    instance: inst_base + instance,
                }
                .encode(
                    &mut encoder,
                    view,
                    &repaint::Scissors::new(surf.draw_demand.raster_pass(i), draw_region),
                );
                stats.passes += 1;
                stats.draws += u32::try_from(surf.draw_demand.pass(i).rectangles().len())
                    .expect("damage rectangle count");
            }
            let scissors = repaint::Scissors::new(surf.draw_demand.raster_pass(i), draw_region);
            let raster_draws = scissors.draw_count();
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
            let format_i = format_slot(texture.format());
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
                let mut gpu_module = false;
                render_pass.set_pipeline(coverage_pipeline(
                    &mut self.coverage_pipelines,
                    &factory,
                    target_format,
                    CoveragePhase::Opaque,
                    gpu_module,
                )?);
                render_pass.set_bind_group(
                    0,
                    &self.bind0,
                    &[(surf.globals_base + u32::try_from(i).unwrap()) * 256],
                );
                let bind = surf
                    .binds1
                    .entry((None, None, None, None))
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
                    if range.gpu.is_some() != gpu_module {
                        gpu_module = range.gpu.is_some();
                        render_pass.set_pipeline(coverage_pipeline(
                            &mut self.coverage_pipelines,
                            &factory,
                            target_format,
                            CoveragePhase::Opaque,
                            gpu_module,
                        )?);
                        stats.pipeline_switches += 1;
                    }
                    if let Some(g) = range.gpu {
                        scissors.draw(&mut render_pass, |raster| {
                            raster.draw_indirect(
                                self.gpu_cov.draws_buffer(),
                                gpu_cov::GpuCoverage::draw_base(g)
                                    + u64::from(gpu_cov::ARGS_OPAQUE),
                            );
                        });
                    } else {
                        scissors.draw(&mut render_pass, |raster| {
                            raster.draw(
                                0..8,
                                (inst_base + range.instances.start)
                                    ..(inst_base + range.instances.end),
                            );
                        });
                    }
                    stats.draws += raster_draws;
                }
                // The polygon tiers draw vertex indices past 8, which
                // the cell-buffer variants read as a GPU-draw join: they
                // run under the plain opaque pipeline.
                if gpu_module {
                    gpu_module = false;
                    render_pass.set_pipeline(coverage_pipeline(
                        &mut self.coverage_pipelines,
                        &factory,
                        target_format,
                        CoveragePhase::Opaque,
                        gpu_module,
                    )?);
                    stats.pipeline_switches += 1;
                }
                for (tier, range) in pass.tier_ranges.iter().enumerate() {
                    if !range.is_empty() {
                        scissors.draw(&mut render_pass, |raster| {
                            raster.draw(
                                0..(16u32 << tier),
                                (inst_base + range.start)..(inst_base + range.end),
                            );
                        });
                        stats.draws += raster_draws;
                    }
                }
                render_pass.set_pipeline(coverage_pipeline(
                    &mut self.coverage_pipelines,
                    &factory,
                    target_format,
                    CoveragePhase::Partial,
                    false,
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
            let mut pipeline = if coverage_order {
                Bound::Coverage(false)
            } else {
                Bound::Engine(PipelineKind::SrcOver, ShaderVariant::Simple)
            };
            let backdrop_slot = scratch_backdrop.then(|| format_slot(target_format));
            if replayable {
                let bind1 = surf
                    .binds1
                    .entry((None, None, None, None))
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
                scissors.draw(&mut render_pass, |raster| {
                    raster.execute_bundles([&surf.painter_replays[&i].bundle]);
                });
                stats.draws += u32::try_from(pass.ranges.len())
                    .expect("draw ranges fit the instance limit")
                    * raster_draws;
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
                stats.draws += raster_draws;
                if let PipelineKind::Projective { replace } = range.pipeline {
                    let pipes = self
                        .projective
                        .as_ref()
                        .expect("projective pipelines are built before encoding");
                    if pipeline != Bound::Projective(replace) {
                        pipeline = Bound::Projective(replace);
                        stats.pipeline_switches += 1;
                        render_pass.set_pipeline(pipes.composite(format_i, replace));
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
                    scissors.draw(&mut render_pass, |raster| {
                        raster.draw(
                            0..6,
                            (inst_base + range.instances.start)..(inst_base + range.instances.end),
                        );
                    });
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
                    stats.draws +=
                        u32::try_from(draws.len() - 1).unwrap_or(u32::MAX) * raster_draws;
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
                                &scissors,
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
                    scissors.draw(&mut render_pass, |raster| {
                        raster.draw(
                            0..6,
                            (inst_base + range.instances.start)..(inst_base + range.instances.end),
                        );
                    });
                    ri += 1;
                    continue;
                }
                let want = if coverage_order {
                    Bound::Coverage(range.gpu.is_some())
                } else {
                    Bound::Engine(range.pipeline, range.module())
                };
                if want != pipeline {
                    pipeline = want;
                    stats.pipeline_switches += 1;
                    let pipe = match pipeline {
                        Bound::Coverage(gpu) => coverage_pipeline(
                            &mut self.coverage_pipelines,
                            &factory,
                            target_format,
                            CoveragePhase::Partial,
                            gpu,
                        )?,
                        Bound::External | Bound::Projective(_) => {
                            unreachable!("a range here wants an engine or coverage pipeline")
                        }
                        Bound::Engine(kind, variant) => match kind {
                            PipelineKind::Effect { id, replace } => {
                                let entry =
                                    self.backdrop_shaders.get_mut(&id).ok_or_else(|| {
                                        RenderError::Render(format!(
                                            "backdrop shader {id} is not registered"
                                        ))
                                    })?;
                                let union = variant == ShaderVariant::Union;
                                let replace_i = usize::from(replace);
                                let missing = if union {
                                    entry.union[replace_i][format_i].is_none()
                                } else {
                                    entry.plain[replace_i][format_i].is_none()
                                };
                                if missing {
                                    // A union-shaped or replace pipeline builds
                                    // at its first use — wasm fills them ahead
                                    // of encode in `prepare_lazy_pipelines`.
                                    #[cfg(not(target_arch = "wasm32"))]
                                    {
                                        let module = if union {
                                            &*entry.union_module.get_or_insert_with(|| {
                                                self.device.create_shader_module(
                                                    wgpu::ShaderModuleDescriptor {
                                                        label: Some("backdrop effect (union)"),
                                                        source: wgpu::ShaderSource::Wgsl(
                                                            (&*entry.union_source).into(),
                                                        ),
                                                    },
                                                )
                                            })
                                        } else {
                                            &entry.plain_module
                                        };
                                        let built = create_pipeline(
                                            &self.device,
                                            &self.config,
                                            &self.pipeline_layout,
                                            self.pipeline_cache.handle.as_ref(),
                                            &shaders::wgsl_engine_programs(module),
                                            target_format,
                                            CoveragePass::Painter(replace),
                                        )
                                        .map_err(|e| {
                                            RenderError::Render(format!(
                                                "backdrop shader {id}: {e}"
                                            ))
                                        })?;
                                        let cells = if union {
                                            &mut entry.union
                                        } else {
                                            &mut entry.plain
                                        };
                                        cells[replace_i][format_i] = Some(built);
                                    }
                                    #[cfg(target_arch = "wasm32")]
                                    return Err(RenderError::Render(format!(
                                        "backdrop shader {id}'s pipeline was not prepared"
                                    )));
                                }
                                let cells = if union { &entry.union } else { &entry.plain };
                                cells[replace_i][format_i]
                                    .as_ref()
                                    .expect("the effect pipeline is built")
                            }
                            kind => core_pipeline(
                                &mut self.pipelines,
                                &factory,
                                target_format,
                                kind == PipelineKind::Replace,
                                variant,
                            )?,
                        },
                    };
                    render_pass.set_pipeline(pipe);
                }
                let source = range.source.map(|source| match source {
                    Source::Scratch(depth) => {
                        Source::Scratch(surf.composition.scratch_slot(i, Target::Scratch(depth)))
                    }
                    other => other,
                });
                let key = (source, backdrop_slot, range.image.clone(), range.mask);
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
                            source.map(|s| match s {
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
                if let Some(g) = range.gpu {
                    if coverage_order {
                        scissors.draw(&mut render_pass, |raster| {
                            raster.draw_indexed_indirect(
                                self.gpu_cov.draws_buffer(),
                                gpu_cov::GpuCoverage::draw_base(g)
                                    + u64::from(gpu_cov::ARGS_PARTIAL),
                            );
                        });
                    } else {
                        scissors.draw(&mut render_pass, |raster| {
                            raster.draw_indirect(
                                self.gpu_cov.draws_buffer(),
                                gpu_cov::GpuCoverage::draw_base(g)
                                    + u64::from(gpu_cov::ARGS_NORMAL),
                            );
                        });
                    }
                } else if coverage_order {
                    // Four vertex shader runs per quad, but the indices
                    // submit the painter path's exact ordered triangles —
                    // required for bit-identical rasterization.
                    scissors.draw(&mut render_pass, |raster| {
                        raster.draw_indexed(
                            0..6,
                            0,
                            (inst_base + range.instances.start)..(inst_base + range.instances.end),
                        );
                    });
                } else {
                    scissors.draw(&mut render_pass, |raster| {
                        raster.draw(
                            0..6,
                            (inst_base + range.instances.start)..(inst_base + range.instances.end),
                        );
                    });
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
                stats.passes += self
                    .shadow_blur
                    .get_or_insert_with(|| {
                        shadow::Blur::new(
                            &self.device,
                            self.shader_delivery,
                            self.scratch_format,
                            &self.pipeline_cache,
                        )
                    })
                    .apply(
                        &self.device,
                        &mut encoder,
                        &surf.scratch[&surf.composition.scratch_slot(i, Target::Scratch(depth))],
                        (pass.region[2], pass.region[3]),
                        lower::shadow_support(&surf.frame, i),
                        *parameters,
                    )?;
            }
            if let Some((_, filter)) = surf.frame.filters.iter().find(|(pass, _)| *pass == i) {
                let capture = match pass.target {
                    Target::Scratch(_) => {
                        &surf.scratch[&surf.composition.scratch_slot(i, pass.target)]
                    }
                    Target::Backdrop { group, region } => {
                        &surf.backdrop_groups[&group].captures[region as usize].target
                    }
                    Target::Part(_) | Target::Projected(_) | Target::Plane(_) => {
                        return Err(RenderError::Render(format!(
                            "filter {filter:?} registered on a surface pass"
                        )));
                    }
                };
                // A filter works in the linear working space: over an
                // sRGB-encoded scope its input is decoded and its output
                // encoded back.
                let convert = (pass.space == cherenkov::BlendSpace::SrgbEncoded).then(|| {
                    self.space_convert.get_or_insert_with(|| {
                        space_convert::Pipelines::new(
                            &self.device,
                            self.shader_delivery,
                            self.pipeline_cache.clone(),
                        )
                    })
                });
                self.filters.apply(
                    *filter,
                    &filtrate::EffectContext {
                        device: &self.device,
                        queue: &self.queue,
                        input_format: capture.texture.format(),
                        output_format: capture.texture.format(),
                        pipelines: &self.filter_pipelines,
                    },
                    filter::Scope {
                        cache: &self.pipeline_cache,
                        target: capture,
                        size: (pass.region[2], pass.region[3]),
                        encoded: convert,
                    },
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
                        reduce::Pipelines::new(
                            &self.device,
                            self.shader_delivery,
                            self.pipeline_cache.clone(),
                        )
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
        // Attribute every analyzed image, including materialized images
        // on targets without a native tile executor.
        if let Some(sink) = self
            .config
            .composition_diag
            .as_ref()
            .filter(|_| surf.composition.is_planned())
        {
            // The attachments the composition plan governs: the surface's
            // parts and the isolation scratches a materialized image
            // needs, against the epochs' memoryless tile slots.
            let part_bytes =
                u64::from(surf.size.0) * u64::from(surf.size.1) * texel_bytes(surf.target.format());
            let parts = u64::try_from(surf.parts.len()).expect("part count fits u64");
            let backed = part_bytes * (1 + parts)
                + surf.scratch.values().map(ScratchTarget::bytes).sum::<u64>();
            // Metal's memoryless attachments have no commitment to
            // query, so Apple reports what they reserve and nothing
            // else; Vulkan asks the driver what it actually spent.
            #[cfg(target_vendor = "apple")]
            let (peak_reserved_bytes, peak_committed_bytes) =
                (surf.tile_targets.memoryless_bytes(), None);
            #[cfg(all(unix, not(target_vendor = "apple")))]
            let (peak_reserved_bytes, peak_committed_bytes) = self
                .tile_mirror
                .as_ref()
                .ok()
                .filter(|_| tile_frame)
                .map_or((0, None), |mirror| {
                    (mirror.memoryless_bytes(), Some(mirror.committed_bytes()))
                });
            #[cfg(not(unix))]
            let (peak_reserved_bytes, peak_committed_bytes) = (0, None);
            sink.record(composition::frame_report(
                frame.get(),
                id.raw(),
                &surf.frame,
                &surf.composition,
                composition::AttachmentBytes {
                    peak_reserved_bytes,
                    peak_committed_bytes,
                    peak_backed_bytes: backed,
                },
                tile_pipelines,
                surf.draw_demand.output_pixels(&surf.frame),
            ));
        }
        #[cfg(target_vendor = "apple")]
        surf.tile_targets.finish_frame();
        // The epoch's attachments and descriptor pools are released
        // against this surface's own submission: `finish_frame` retires
        // the sizes the frame did not bind, and the flag `end_frame`
        // hands back lets a later frame recycle the pools once the
        // queue reports this submission complete (#2191).
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let tile_idle = self
            .tile_mirror
            .as_mut()
            .ok()
            .filter(|_| tile_frame)
            .and_then(|mirror| {
                mirror.finish_frame();
                mirror.end_frame()
            });
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
            surf.command_damage.submitted();
            // The coverage prologue this encoder carries has landed:
            // entries it generated are replayable and a pending
            // compaction's copies have executed (water-rs/waterui#3081).
            self.gpu_cov.submitted();
            track_submission(&self.queue, &self.tracker);
            // The epoch's descriptor sets were referenced by the
            // command buffers just submitted and by nothing else, so
            // the pools are free to reset the moment the queue reports
            // it done. Nothing waits on this: a later frame takes a
            // fresh set of pools until one is released (#2191).
            #[cfg(all(unix, not(target_vendor = "apple")))]
            if let Some(completion) = tile_idle {
                self.queue
                    .on_submitted_work_done(move || completion.complete());
            }
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
    ///
    /// A timed-out wait abandons a submission that may never retire, and
    /// wgpu destroys a queue only after its work completes: dropping the
    /// queue's last handle waits for idle without a bound. The renderer
    /// therefore leaks one handle on a timeout, which keeps the queue
    /// alive until the process exits, so no later drop of the engine or
    /// of a host's [`SharedDevice`](crate::interop::SharedDevice) clones
    /// can wait on the abandoned submission (water-rs/waterui#3157).
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
        .inspect_err(|error| {
            if matches!(error, RenderError::Timeout { .. }) {
                std::mem::forget(self.queue.clone());
            }
        })
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
        // Deferred destroys run before `release.destroy` on the same
        // completion: work-done callbacks fire in registration order, and
        // an evicted set's `free_descriptor_sets` needs its pool alive —
        // the pool dies in `release.destroy` below, not before.
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
        self.queue.on_submitted_work_done(move || {
            for release in releases {
                release.destroy(&shared);
            }
        });
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

/// A registered image's texture of `size`, holding `data`'s texels in
/// their storage format.
///
/// The texture's allocation and the texel upload each run under an
/// out-of-memory error scope, so an allocation the device cannot back is
/// this call's error rather than an uncaptured device error — a panic in
/// wgpu's handler on native targets, a console message the browser
/// reports to no one. The scope resolves asynchronously: native targets
/// block on it, and the browser awaits it on its event loop. A failed
/// allocation is returned before anything uses its handle.
///
/// # Errors
/// [`ResourceError::Image`] when the device cannot back the texture or
/// its upload.
#[cfg_attr(
    target_arch = "wasm32",
    allow(
        clippy::future_not_send,
        reason = "the browser engine runs this future on the page's event loop; whether wgpu's handles are Send on wasm32 depends on the backends the build enables"
    )
)]
async fn upload_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height): (u32, u32),
    data: &image_storage::Upload<'_>,
) -> Result<GpuImage, ResourceError> {
    let (bw, bh) = data.format.block_dimensions();
    let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let (texture, view) = create_target(
        device,
        "image",
        (width.div_ceil(bw) * bw, height.div_ceil(bh) * bh),
        image_usage(device, data.format, (width, height)),
        data.format,
    );
    if let Some(error) = scope.pop().await {
        return Err(ResourceError::Image(format!(
            "{width}x{height} texture allocation failed: {error}"
        )));
    }
    diag::create(device, "image", image_storage::bytes(&texture));
    write_image(device, queue, &texture, (width, height), data).await?;
    Ok(GpuImage {
        revision: 0,
        texture,
        view,
        width,
        height,
        sampling_flags: data.flags,
    })
}

/// Writes `data`'s texels over the whole of an image texture of `size`
/// in `data`'s storage format.
///
/// The upload runs under an out-of-memory error scope, as in
/// [`upload_image`].
///
/// # Errors
/// [`ResourceError::Image`] when the device cannot back the upload's
/// staging memory; the texture keeps its previous texels.
#[cfg_attr(
    target_arch = "wasm32",
    allow(
        clippy::future_not_send,
        reason = "the browser engine runs this future on the page's event loop; whether wgpu's handles are Send on wasm32 depends on the backends the build enables"
    )
)]
async fn write_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
    data: &image_storage::Upload<'_>,
) -> Result<(), ResourceError> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    write_texels(
        device,
        queue,
        "image",
        texture,
        data.format,
        (width, height),
        &data.bytes,
    );
    scope.pop().await.map_or(Ok(()), |error| {
        Err(ResourceError::Image(format!(
            "{width}x{height} upload failed: {error}"
        )))
    })
}

/// [`write_image`]'s unscoped copy: the texture's whole `bytes`, an
/// image of `size` in the storage `format`, in the block-row bands
/// [`upload_bands`] decides, so no `write_texture` exceeds the device's
/// `max_buffer_size`. A browser stages a `writeTexture` through one
/// internal buffer of the upload's size and rejects it above that
/// limit, silently dropping the image (#2793); a texture under the
/// limit uploads in the single band it is one of.
fn write_texels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    texture: &wgpu::Texture,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    bytes: &[u8],
) {
    let (bw, bh) = format.block_dimensions();
    let row = block_row_bytes(format, width);
    let pitch = u32::try_from(row).expect("a texture row's bytes fit u32");
    let block_rows = height.div_ceil(bh);
    for (first, rows) in upload_bands(row, block_rows, device.limits().max_buffer_size) {
        let offset = usize::try_from(u64::from(first) * row).expect("band offset fits usize");
        let len = usize::try_from(u64::from(rows) * row).expect("band bytes fit usize");
        let y = first * bh;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes[offset..offset + len],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
                rows_per_image: Some(rows),
            },
            // A block format's texture is allocated in whole blocks, so
            // the band covers its blocks' full extent.
            wgpu::Extent3d {
                width: width.div_ceil(bw) * bw,
                height: rows * bh,
                depth_or_array_layers: 1,
            },
        );
        diag::upload(
            device,
            label,
            len as u64,
            Some((0, y, width, (rows * bh).min(height - y))),
        );
    }
}

/// The bytes of one row of `format`'s blocks across `width` texels.
fn block_row_bytes(format: wgpu::TextureFormat, width: u32) -> u64 {
    let (bw, _) = format.block_dimensions();
    u64::from(width.div_ceil(bw))
        * u64::from(
            format
                .block_copy_size(None)
                .expect("an image format is a colour format"),
        )
}

/// The usage of an image texture of `size` in `format` that
/// [`write_texels`] fills.
///
/// A texture that uploads in more than one band receives partial writes,
/// and an implementation zero-initializes a texture before its first
/// partial write. Without `RENDER_ATTACHMENT` the browser's
/// implementation clears it through one staging buffer the size of the
/// whole texture, which exceeds `max_buffer_size` exactly when banding is
/// needed and fails every later write on the device (#2793); with it, the
/// clear is a render pass.
///
/// # Panics
/// When a block-compressed image needs more than one band: such a format
/// cannot be a render attachment, so the browser could not clear it. It
/// stores one byte a texel, so one band holds any image whose texel count
/// fits `max_buffer_size` — every 16384 × 16384 image at WebGPU's default
/// 256 MiB.
fn image_usage(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
) -> wgpu::TextureUsages {
    let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
    let bands = upload_bands(
        block_row_bytes(format, width),
        height.div_ceil(format.block_dimensions().1),
        device.limits().max_buffer_size,
    );
    if bands.count() > 1 {
        assert!(
            !format.is_compressed(),
            "a {width}x{height} {format:?} image needs a banded upload, which a \
             block-compressed texture cannot be cleared for"
        );
        usage | wgpu::TextureUsages::RENDER_ATTACHMENT
    } else {
        usage
    }
}

/// The `(first_row, rows)` bands an upload of `rows` rows of `row` bytes
/// each splits into so that every band stays within `limit` staging
/// bytes: each band is the largest whole number of rows that fits, and
/// the last takes what remains. A row is a row of blocks for a
/// block-compressed format.
///
/// # Panics
/// When a single row exceeds `limit` — unreachable for an image
/// [`cherenkov::ImageLimits`] admits, whose widest row is its largest
/// dimension in eight-byte texels (128 KiB at 16384), far under the
/// smallest `max_buffer_size` a WebGPU device reports.
fn upload_bands(row: u64, rows: u32, limit: u64) -> impl Iterator<Item = (u32, u32)> {
    assert!(
        0 < row && row <= limit,
        "a {row}-byte row exceeds the {limit}-byte staging limit"
    );
    let band = u32::try_from(limit / row).unwrap_or(u32::MAX);
    (0..rows.div_ceil(band)).map(move |i| {
        let first = i * band;
        (first, band.min(rows - first))
    })
}

fn create_gpu_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    (width, height): (u32, u32),
    data: &image_storage::Upload<'_>,
) -> GpuImage {
    let (texture, view) = create_target(
        device,
        label,
        (width, height),
        image_usage(device, data.format, (width, height)),
        data.format,
    );
    write_texels(
        device,
        queue,
        label,
        &texture,
        data.format,
        (width, height),
        &data.bytes,
    );
    GpuImage {
        revision: 0,
        sampling_flags: data.flags,
        texture,
        view,
        width,
        height,
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
                    Target::Scratch(_) => surface.scratch[&surface
                        .composition
                        .scratch_slot(*pass, surface.frame.passes[*pass].target)]
                        .texture
                        .format(),
                    Target::Backdrop { group, region } => surface.backdrop_groups[&group].captures
                        [region as usize]
                        .target
                        .texture
                        .format(),
                    Target::Part(_) | Target::Plane(_) => surface.format,
                    Target::Projected(_) => WORKING_FORMAT,
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
                        pipelines: &self.filter_pipelines,
                    },
                )
                .await?;
        }
        Ok(())
    }

    /// wasm: the lazily built pipelines a frame's lowered passes need —
    /// core `(variant, format, replace)` cells for `ShaderVariant::Union`
    /// and `ShaderVariant::Image` ranges,
    /// and every backdrop effect slot they draw that registration did not
    /// build (a union-shaped or replace pipeline) — prepared ahead of
    /// encode, which cannot await pipeline creation. The union engine
    /// module materializes here too, on the first frame that actually
    /// draws a union or outer-band member.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn prepare_lazy_pipelines(&mut self, surface: SurfaceId) -> Result<(), RenderError> {
        let surface = &self.surfaces[&surface];
        // (variant index, format slot, replace) demand-built core cells
        // and (shader id, union, replace, format slot) effect slots the
        // frame draws.
        let mut core = FxHashSet::default();
        let mut effects = FxHashSet::default();
        // Format slots whose simple-module coverage pair a coverage pass
        // draws with and init did not build.
        let mut coverage = FxHashSet::default();
        for (index, pass) in surface.frame.passes.iter().enumerate() {
            let format = match pass.target {
                Target::Scratch(_) => {
                    if surface.composition.materialization(index).is_none() {
                        continue;
                    }
                    surface.scratch[&surface.composition.scratch_slot(index, pass.target)]
                        .texture
                        .format()
                }
                Target::Backdrop { group, region } => surface.backdrop_groups[&group].captures
                    [region as usize]
                    .target
                    .texture
                    .format(),
                Target::Part(_) | Target::Plane(_) => surface.format,
                Target::Projected(_) => WORKING_FORMAT,
            };
            let format_i = format_slot(format);
            // Init builds every eager cell of the working and scratch
            // formats; a surface's own format has none of them.
            let eager = format == WORKING_FORMAT || format == self.scratch_format;
            if coverage_order(pass) {
                if !eager {
                    coverage.insert((format_i, false));
                }
                if pass.ranges.iter().any(|range| range.gpu.is_some()) {
                    coverage.insert((format_i, true));
                }
            }
            for range in &pass.ranges {
                let union = range.variant == ShaderVariant::Union;
                let module = variant_index(range.module());
                if let PipelineKind::Effect { id, replace } = range.pipeline {
                    effects.insert((id, union, replace, format_i));
                } else if !EAGER_CORE_VARIANTS.contains(&module)
                    || (!eager
                        && !matches!(range.pipeline, PipelineKind::Projective { .. })
                        && !matches!(range.image, Some(lower::ImageSource::Content(_))))
                {
                    core.insert((module, format_i, range.pipeline == PipelineKind::Replace));
                }
            }
        }
        for (variant, format_i, replace) in core {
            self.prepare_lazy_core(variant, format_i, replace).await?;
        }
        for (format_i, gpu) in coverage {
            self.prepare_lazy_coverage(format_i, gpu).await?;
        }
        for (id, union, replace, format_i) in effects {
            let Some(entry) = self.backdrop_shaders.get_mut(&id) else {
                continue;
            };
            let replace_i = usize::from(replace);
            let missing = if union {
                entry.union[replace_i][format_i].is_none()
            } else {
                entry.plain[replace_i][format_i].is_none()
            };
            if !missing {
                continue;
            }
            let module = if union {
                &*entry.union_module.get_or_insert_with(|| {
                    self.device
                        .create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("backdrop effect (union)"),
                            source: wgpu::ShaderSource::Wgsl((&*entry.union_source).into()),
                        })
                })
            } else {
                &entry.plain_module
            };
            let built = create_pipeline(
                &self.device,
                &self.config,
                &self.pipeline_layout,
                self.pipeline_cache.handle.as_ref(),
                &shaders::wgsl_engine_programs(module),
                RENDER_FORMATS[format_i],
                CoveragePass::Painter(replace),
            )
            .await
            .map_err(|e| RenderError::Render(e.to_string()))?;
            let cells = if union {
                &mut entry.union
            } else {
                &mut entry.plain
            };
            cells[replace_i][format_i] = Some(built);
        }
        Ok(())
    }

    /// wasm: the demand-built core cell `(variant, format slot, replace)`
    /// not built at initialization, with its engine module.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn prepare_lazy_core(
        &mut self,
        variant: usize,
        format_i: usize,
        replace: bool,
    ) -> Result<(), RenderError> {
        let cell = &mut self.pipelines[format_i][usize::from(replace)][variant];
        if cell.is_some() {
            return Ok(());
        }
        let module = self
            .lazy_modules
            .entry(variant)
            .or_insert_with(|| self.shader_delivery.engine_programs(&self.device, variant));
        *cell = Some(
            create_pipeline(
                &self.device,
                &self.config,
                &self.pipeline_layout,
                self.pipeline_cache.handle.as_ref(),
                module,
                RENDER_FORMATS[format_i],
                CoveragePass::Painter(replace),
            )
            .await
            .map_err(|e| RenderError::Render(e.to_string()))?,
        );
        Ok(())
    }

    /// wasm: a `[opaque, partial]` coverage pair not built at initialization.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn prepare_lazy_coverage(
        &mut self,
        format_i: usize,
        gpu: bool,
    ) -> Result<(), RenderError> {
        if self.coverage_pipelines[format_i][usize::from(gpu)]
            .iter()
            .all(Option::is_some)
        {
            return Ok(());
        }
        let variant = variant_index(if gpu {
            ShaderVariant::Cell
        } else {
            ShaderVariant::Simple
        });
        let module = self
            .lazy_modules
            .entry(variant)
            .or_insert_with(|| self.shader_delivery.engine_programs(&self.device, variant));
        for phase in [CoveragePhase::Opaque, CoveragePhase::Partial] {
            self.coverage_pipelines[format_i][usize::from(gpu)][phase as usize] = Some(
                create_pipeline(
                    &self.device,
                    &self.config,
                    &self.pipeline_layout,
                    self.pipeline_cache.handle.as_ref(),
                    module,
                    RENDER_FORMATS[format_i],
                    CoveragePass::Coverage(phase),
                )
                .await
                .map_err(|e| RenderError::Render(e.to_string()))?,
            );
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

fn notify_texture(
    sender: Option<&std::sync::mpsc::Sender<wgpu::Texture>>,
    texture: &wgpu::Texture,
) {
    if let Some(sender) = sender {
        let _ = sender.send(texture.clone());
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

    /// A registered backdrop effect validates through the same path as a
    /// material registration: the composed module calls `quantizeToF16`,
    /// which needs the engine's capability floor at runtime as in `build.rs`.
    #[test]
    fn backdrop_effect_registration_validates() {
        let source = cherenkov::BackdropShaderSource::wgsl(
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return backdrop_sample(px.p);
            }",
        );
        GpuRenderer::validate_backdrop_shader(&source)
            .expect("a minimal backdrop effect passes runtime validation");
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

    /// A commit's emptied `CellWrites` shell keeps its list capacities
    /// for reuse; `trim` replaces the container so a pressured engine
    /// retains none of them (#2711).
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn trim_releases_the_reused_commit_lists() {
        let mut renderer = match init(GpuConfig::default()) {
            Ok((renderer, _)) => renderer,
            Err(EngineError::Backend(_)) => return,
            Err(e) => panic!("engine init failed: {e}"),
        };
        renderer.commit_writes.push_arena(vec![0xAB; 64]);
        assert!(renderer.commit_writes.heap_bytes() > 0);
        renderer.trim(Pressure::Moderate);
        assert_eq!(
            renderer.commit_writes.heap_bytes(),
            0,
            "a trimmed engine retains no commit_writes capacity"
        );
    }

    /// The browser stages a `writeTexture` through one buffer of the
    /// upload's size, so [`upload_bands`] keeps every band within the
    /// staging limit (#2793).
    #[test]
    fn upload_bands_stay_within_the_staging_limit() {
        let mib = 1024 * 1024;
        let limit = 256 * mib;
        // An image under the limit is one band.
        assert_eq!(
            upload_bands(4096 * 8, 4096, limit).collect::<Vec<_>>(),
            [(0, 4096)]
        );
        // The issue's case: 8192×8192 f16 texels (512 MiB) at a 256 MiB
        // limit is two bands of 4096 rows.
        assert_eq!(
            upload_bands(8192 * 8, 8192, limit).collect::<Vec<_>>(),
            [(0, 4096), (4096, 4096)]
        );
        // An odd height leaves a short last band.
        assert_eq!(
            upload_bands(8192 * 8, 4097, limit).collect::<Vec<_>>(),
            [(0, 4096), (4096, 1)]
        );
        // Bands cover every row exactly once.
        let mut covered = 0;
        for (first_row, rows) in upload_bands(4096 * 8, 20011, mib) {
            assert_eq!(first_row, covered);
            covered += rows;
        }
        assert_eq!(covered, 20011);
    }
}
