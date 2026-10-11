//! Engine adapters and the `cherenkov-bench` CLI for the Cherenkov
//! cross-engine correctness and performance suite.
//!
//! Each adapter renders a [`cherenkov_scene::Scene`] offscreen and returns a
//! linear `f32` RGBA image in the suite working space (premultiplied linear
//! Display P3). Colour conversion is explicit and identical everywhere —
//! see [`convert`] — and every adapter records what the engine actually
//! does in its [`EngineInfo`]:
//!
//! | adapter | crate | route | output format |
//! |---------|-------|-------|---------------|
//! | `vello-cpu` | `vello_cpu` | CPU raster (`f32` pipeline) | rgba8 premul pixmap |
//! | `vello-classic` | `vello` | wgpu (Vulkan/Metal/D3D12) | `Rgba8Unorm`/`Bgra8Unorm` texture, picked from queried capabilities |
//! | `vello-hybrid` | `vello_hybrid` | CPU strips + wgpu colour pass | `Rgba8Unorm`/`Bgra8Unorm` texture, picked from queried capabilities |
//! | `skia-cpu` | `skia-safe` | CPU raster (`SkRasterPipeline`) | `RGBAF16` premul surface, linear-P3 colours |
//! | `skia-vulkan` | `skia-safe` | Ganesh/Vulkan | `RGBAF16` premul surface, linear-P3 colours |
//! | `skia-metal` | `skia-safe` | Graphite/Metal | `RGBAF16` premul render target, linear-P3 colours |
//!
//! GPU time is reported only where a real GPU timestamp source exists.
//! The external wgpu adapters and skia-vulkan bracket the engine submission with
//! timestamps written in standalone submissions after a full queue drain
//! (see `wgpu_ctx::drain_and_stamp` when those adapters are enabled); skia-metal brackets Graphite's
//! submission with empty `MTLCommandBuffer` markers on the same serial
//! queue, each preceded by a drain (`commit` + `waitUntilCompleted`),
//! and reads their `GPUStartTime`/`GPUEndTime`. This **serializes CPU
//! and GPU** for the measured frame — a synchronous probe, not a
//! pipelined frame rate. Where no timestamp source exists the field is
//! `null` — it is never estimated. The Cherenkov GPU adapter instead collects
//! timestamps from completed earlier submissions without draining each frame.

pub mod affinity;
#[cfg(feature = "cherenkov")]
pub mod capture;
#[cfg(feature = "cherenkov")]
pub mod cherenkov_ad;
#[cfg(feature = "cherenkov-cpu")]
pub mod cherenkov_cpu_ad;
pub mod cli;
pub mod conditions;
pub mod convert;
#[cfg(feature = "cherenkov")]
pub mod creation;
pub mod energy;
#[cfg(feature = "cherenkov")]
pub mod external_cost;
pub mod gamut_sweep;
pub mod memory;
#[cfg(any(feature = "cherenkov", feature = "cherenkov-cpu"))]
pub mod motion;
#[cfg(feature = "cherenkov")]
pub mod present_cost;
pub mod projective_quality;
pub mod refcache;
pub mod report;
#[cfg(any(feature = "cherenkov", feature = "cherenkov-cpu"))]
pub mod timing;
pub mod tone_sweep;
pub mod transform;
#[cfg(any(
    feature = "vello-classic",
    feature = "vello-cpu",
    feature = "vello-hybrid"
))]
pub mod vello_like;
#[cfg(any(feature = "vello-classic", feature = "vello-hybrid"))]
pub mod wgpu_ctx;

#[cfg(any(feature = "skia", feature = "skia-metal"))]
pub mod skia_ad;
#[cfg(feature = "vello-classic")]
pub mod vello_classic_ad;
#[cfg(feature = "vello-cpu")]
pub mod vello_cpu_ad;
#[cfg(feature = "vello-hybrid")]
pub mod vello_hybrid_ad;

use std::collections::BTreeSet;

use cherenkov_oracle::F32Image;
use cherenkov_scene::{Feature, Scene, SceneError};
use serde::Serialize;

use crate::convert::Blobs;
use crate::memory::AdapterMemory;

#[cfg(unix)]
pub use cli::cherenkov_bench_run;

/// Errors an adapter or the CLI can produce.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// The scene declares a feature this adapter does not implement; the
    /// scene is reported unsupported rather than emulated.
    #[error("{engine}: unsupported scene feature {feature:?}")]
    Unsupported {
        /// Adapter name.
        engine: &'static str,
        /// The missing feature.
        feature: Feature,
        /// The upstream API the engine lacks, when known.
        api: Option<&'static str>,
    },
    /// Scene loading / resource errors.
    #[error(transparent)]
    Scene(#[from] SceneError),
    /// Oracle render errors.
    #[error(transparent)]
    Oracle(#[from] cherenkov_oracle::RenderError),
    /// I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// GPU setup or submission failure.
    #[error("gpu: {0}")]
    Gpu(String),
    /// Engine-specific failure.
    #[error("{0}")]
    Engine(String),
}

/// Provenance pinned into every result JSON — what the engine actually is
/// and does, not a slogan.
#[derive(Clone, Debug, Serialize)]
pub struct EngineInfo {
    /// Adapter key used on the CLI (`vello-classic`, `skia-vulkan`, ...).
    pub name: &'static str,
    /// The underlying crate name.
    pub engine_crate: &'static str,
    /// Exact crate version resolved in `Cargo.lock`.
    pub crate_version: &'static str,
    /// Commit sha the crate resolves to when a workspace `[patch.crates-io]`
    /// git pin supplies it (the `lexoliu/vello` fork revs Hydrolysis runs).
    /// `None` for plain registry packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_rev: Option<String>,
    /// The output texture/pixmap format the engine renders into (a runtime
    /// value for adapters that pick the format from queried capabilities).
    pub output_format: String,
    /// The engine's documented internal arithmetic precision.
    pub precision: &'static str,
    /// The route taken (`cpu-raster`, `vulkan`, `hybrid`, ...).
    pub route: &'static str,
    /// How the engine treats colour.
    pub color_note: &'static str,
    /// Exactly what the timed `measure` *encode* phase covers for this
    /// engine — the engine's own recording/encoding API calls for the
    /// frame, against resources built once in `prepare`.
    pub encode_scope: &'static str,
}

/// Cheap per-frame counters the adapter can report truthfully.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Counters {
    /// Draw commands issued to the engine (fills, strokes, images, glyph
    /// runs, shadows).
    pub draw_commands: u32,
    /// Layers visited (including the root).
    pub layers: u32,
    /// Bytes uploaded to the GPU this submission, when the adapter itself
    /// performs the upload (image textures). `null` when the engine owns
    /// uploads internally.
    pub bytes_uploaded: Option<u64>,
    /// Dispatch/draw calls submitted, where the backend exposes them.
    pub dispatches: Option<u32>,
    /// Render/dispatch pass count, where the backend exposes it.
    pub passes: Option<u32>,
    /// `Engine::memory()` GPU bytes at the end of the run, where the adapter
    /// exposes it; `None` for other engines.
    pub memory_gpu_bytes: Option<u64>,
    /// `Engine::memory()` CPU bytes at the end of the run, where the adapter
    /// exposes it; `None` for other engines.
    pub memory_cpu_bytes: Option<u64>,
    /// `Engine::memory()` backdrop-group capture bytes at the end of the
    /// run; `None` for other engines.
    pub memory_backdrop_capture_bytes: Option<u64>,
    /// `Engine::memory()` backdrop-capture texture format at the end of the
    /// run (`null` while no capture is allocated); `None` for other
    /// engines.
    pub memory_backdrop_capture_format: Option<&'static str>,
}

/// Device and thermal metadata captured at measure time.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceInfo {
    /// GPU adapter name (wgpu/Skia), when a GPU is involved.
    pub adapter: Option<String>,
    /// Graphics backend (`vulkan`, `metal`, `dx12`, `gl`).
    pub backend: Option<String>,
    /// Driver name.
    pub driver: Option<String>,
    /// Driver info string.
    pub driver_info: Option<String>,
    /// PCI vendor id.
    pub vendor: Option<u32>,
    /// PCI device id.
    pub device: Option<u32>,
    /// The render target texture format chosen from queried adapter
    /// capabilities (`vello-*` GPU adapters), `None` otherwise.
    pub target_format: Option<String>,
    /// Host CPU model (sysinfo brand; `/proc/cpuinfo` `Hardware` on
    /// Android), always populated.
    pub cpu: Option<String>,
    /// Thermal zone temperature in °C, when readable.
    pub thermal_celsius: Option<f64>,
}

/// One timed render pass of a submitted frame.
#[derive(Clone, Debug, Serialize)]
pub struct PassSample {
    /// The pass's name (`"surface"`, `"scratch{n}"`, ...).
    pub name: String,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target texture format (`"rgba16float"`, `"rgba8unorm"`, ...).
    pub format: String,
    /// GPU seconds the pass took; `null` when the backend's end
    /// timestamp does not exceed its start.
    pub gpu_seconds: Option<f64>,
}

/// The GPU timing of one submitted frame.
#[derive(Debug)]
pub struct GpuSample {
    /// The `frame` passed to the [`Engine::submit`] that submitted it.
    pub frame: u64,
    /// GPU seconds from real GPU timestamps; `None` when the backend's
    /// timestamps do not span the frame. Never estimated.
    pub gpu_seconds: Option<f64>,
    /// Per-pass GPU timings, in submission order; empty when the backend
    /// exposes none.
    pub passes: Vec<PassSample>,
}

impl GpuSample {
    /// What a backend that times whole frames synchronously reports:
    /// `frame`'s own timing, when it has one.
    #[must_use]
    pub fn whole_frame(frame: u64, gpu_seconds: Option<f64>) -> Vec<Self> {
        gpu_seconds
            .map(|gpu_seconds| Self {
                frame,
                gpu_seconds: Some(gpu_seconds),
                passes: Vec::new(),
            })
            .into_iter()
            .collect()
    }
}

/// One render-thread CPU phase of a submitted frame.
#[derive(Clone, Debug, Serialize)]
pub struct PhaseSample {
    /// The phase's name (`"lower"`, `"encode"`, `"stamp"`, `"wait"`).
    pub name: String,
    /// Wall-clock seconds the render thread spent in it.
    pub seconds: f64,
}

/// Render-thread CPU seconds of each phase of one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phases {
    /// CPU lowering of dirty surfaces.
    pub lower: f64,
    /// Command encoding and queue submission.
    pub encode: f64,
    /// Timestamp drain and resolve work.
    pub stamp: f64,
    /// The final blocking queue drain.
    pub wait: f64,
}

/// The result of [`Engine::submit`].
#[derive(Debug)]
pub struct Submit {
    /// The rendered image in the working space, when readback was
    /// requested.
    pub image: Option<F32Image>,
    /// GPU timings returned with this submission. Cherenkov adapters
    /// return their timings from [`Engine::finish_gpu`].
    pub gpu: Vec<GpuSample>,
    /// Render-thread CPU timings for Cherenkov GPU; `None` when the adapter
    /// does not expose phase timings.
    pub phases: Option<Phases>,
    /// Wall-clock seconds this call spent rasterizing before any readback
    /// began (queue drains and timestamp markers included), where the
    /// adapter separates the two stages; `None` otherwise.
    pub render_seconds: Option<f64>,
    /// Wall-clock seconds this call spent reading the pixels back;
    /// `None` when `readback` was false.
    pub readback_seconds: Option<f64>,
}

/// Resources an adapter needs to encode one scene.
#[derive(Debug)]
pub struct EncodeInput<'a> {
    /// The scene.
    pub scene: &'a Scene,
    /// Its resource blobs (fonts, images).
    pub blobs: &'a Blobs,
}

/// A presentation output kind for `render --present` — the destination a
/// backend's presentation pass writes and the adapter reads back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum PresentKind {
    /// sRGB texture format (`Rgba8UnormSrgb`); the hardware transfer encodes.
    SrgbHw,
    /// Unorm texture (`Rgba8Unorm`); `present.wgsl` applies the transfer.
    SrgbShader,
    /// `Rgba16Float` extended linear Display P3
    /// (`OutputColor::LinearDisplayP3`).
    LinearP3,
    /// Display P3 SDR on an sRGB texture format — hardware transfer,
    /// P3 destination gamut (#98).
    DisplayP3Hw,
    /// Display P3 SDR on `Rgba8Unorm`; `present.wgsl` applies the
    /// transfer (#98).
    DisplayP3Shader,
    /// `Rgba16Float` extended linear sRGB — scRGB
    /// (`OutputColor::ExtendedSrgbLinear`, #98).
    Scrgb,
    /// `Rgba16Float` encoded extended sRGB
    /// (`OutputColor::ExtendedSrgb`, #98).
    ExtendedSrgb,
    /// `Rgba16Float` encoded extended Display P3
    /// (`OutputColor::ExtendedDisplayP3`, #98).
    ExtendedP3,
    /// `Rgba16Float` BT.2100 PQ signal (`OutputColor::Bt2100Pq`, #98).
    Pq,
    /// `Rgba16Float` BT.2100 HLG signal (`OutputColor::Bt2100Hlg`, #98).
    Hlg,
}

impl PresentKind {
    /// The CLI value and report/file-name token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SrgbHw => "srgb-hw",
            Self::SrgbShader => "srgb-shader",
            Self::LinearP3 => "linear-p3",
            Self::DisplayP3Hw => "display-p3-hw",
            Self::DisplayP3Shader => "display-p3-shader",
            Self::Scrgb => "scrgb",
            Self::ExtendedSrgb => "extended-srgb",
            Self::ExtendedP3 => "extended-p3",
            Self::Pq => "pq",
            Self::Hlg => "hlg",
        }
    }
}

/// A rendering-engine adapter.
///
/// Lifecycle: [`Engine::prepare`] runs once per scene outside the timed
/// loop — it checks features, registers and creates every resource the
/// engine would cache in a real app (fonts/typefaces, images including
/// GPU uploads, immutable shaders) and converts the scene into the
/// pre-resolved form the adapter drives the engine from. The timed
/// per-frame [`Engine::encode`] then covers only the engine's own
/// recording/encoding API calls for the frame (see each adapter's
/// `encode_scope` provenance). [`Engine::submit`] rasterizes it (`wgpu`
/// submission + device poll for GPU engines, `render()` for CPU ones)
/// and optionally reads the pixels back.
pub trait Engine {
    /// Provenance pinned into every result JSON.
    fn info(&self) -> &EngineInfo;
    /// The feature set this adapter can execute faithfully.
    fn supported(&self) -> BTreeSet<Feature>;
    /// Prepare the scene's resources once, outside the timed loop.
    ///
    /// # Errors
    /// [`BenchError::Unsupported`] for unimplemented features, or an
    /// engine-level error.
    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError>;
    /// Encode the scene into engine-native state, using only the
    /// resources [`Engine::prepare`] built — no per-frame font parsing,
    /// image decoding, or GPU uploads.
    ///
    /// # Errors
    /// [`BenchError::Unsupported`] for unimplemented features, or an
    /// engine-level error.
    fn encode(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError>;
    /// Puts the adapter into presentation mode: [`Engine::submit`] then
    /// returns the image after the backend's real presentation pass into a
    /// `kind` output. Called once, right after construction and before
    /// `prepare`. Backends without a presentation step fail rather than
    /// compare a raw working-space readback.
    ///
    /// # Errors
    /// [`BenchError::Engine`] when the backend has no presentation step.
    fn present(&mut self, kind: PresentKind) -> Result<(), BenchError> {
        let _ = kind;
        Err(BenchError::Engine(format!(
            "{} renders into the working space directly; it has no presentation step to measure",
            self.info().name
        )))
    }
    /// Rasterize the encoded scene as `frame`, the tag its
    /// [`GpuSample`] carries; with `readback`, return the pixels.
    ///
    /// # Errors
    /// [`BenchError`] on engine or GPU failure.
    fn submit(&mut self, frame: u64, readback: bool) -> Result<Submit, BenchError>;
    /// Wait for the GPU timings of every submitted frame not yet
    /// returned by [`Engine::submit`]. Only a backend whose timestamps
    /// resolve after its submit returns has any.
    ///
    /// # Errors
    /// [`BenchError`] on engine or GPU failure.
    fn finish_gpu(&mut self) -> Result<Vec<GpuSample>, BenchError> {
        Ok(Vec::new())
    }
    /// Explicitly retires what the backend can release outside a frame
    /// (scratch targets, backdrops, caches) and returns once the
    /// retirement is processed. The report's post-retirement snapshot
    /// follows this call; a backend with no retirement step still
    /// reports what it holds afterwards — that is a finding, not an
    /// error.
    ///
    /// # Errors
    /// [`BenchError`] on engine or GPU failure.
    fn trim(&mut self) -> Result<(), BenchError> {
        Ok(())
    }
    /// Counters describing what [`Engine::encode`] issued.
    fn counters(&self) -> Counters;
    /// Memory sources the adapter can report at this point in the run.
    fn memory(&self) -> AdapterMemory;
    /// Device/thermal metadata.
    fn device(&self) -> DeviceInfo;
}

/// All adapter keys compiled into this binary.
#[must_use]
pub fn engine_names() -> Vec<&'static str> {
    vec![
        #[cfg(feature = "vello-cpu")]
        vello_cpu_ad::VelloCpu::NAME,
        #[cfg(feature = "vello-classic")]
        vello_classic_ad::VelloClassic::NAME,
        #[cfg(feature = "vello-hybrid")]
        vello_hybrid_ad::VelloHybrid::NAME,
        #[cfg(feature = "skia")]
        skia_ad::SkiaCpu::NAME,
        #[cfg(all(feature = "skia", any(target_os = "linux", target_os = "android")))]
        skia_ad::SkiaVk::NAME,
        #[cfg(all(feature = "skia-metal", target_vendor = "apple"))]
        skia_ad::SkiaMtl::NAME,
        #[cfg(feature = "cherenkov")]
        cherenkov_ad::Cherenkov::NAME,
        #[cfg(feature = "cherenkov-cpu")]
        cherenkov_cpu_ad::Cherenkov::NAME,
    ]
}

/// Instantiates an adapter by key.
///
/// # Errors
/// [`BenchError::Engine`] for an unknown or uncompiled adapter name, and
/// whatever the adapter's setup reports (e.g. no GPU adapter).
pub fn create_engine(name: &str) -> Result<Box<dyn Engine>, BenchError> {
    match name {
        #[cfg(feature = "vello-cpu")]
        vello_cpu_ad::VelloCpu::NAME => Ok(Box::new(vello_cpu_ad::VelloCpu::new())),
        #[cfg(feature = "vello-classic")]
        vello_classic_ad::VelloClassic::NAME => {
            vello_classic_ad::VelloClassic::new().map(|e| Box::new(e) as Box<dyn Engine>)
        }
        #[cfg(feature = "vello-hybrid")]
        vello_hybrid_ad::VelloHybrid::NAME => {
            vello_hybrid_ad::VelloHybrid::new().map(|e| Box::new(e) as Box<dyn Engine>)
        }
        #[cfg(feature = "skia")]
        skia_ad::SkiaCpu::NAME => Ok(Box::new(skia_ad::SkiaCpu::new())),
        #[cfg(all(feature = "skia", any(target_os = "linux", target_os = "android")))]
        skia_ad::SkiaVk::NAME => skia_ad::SkiaVk::new().map(|e| Box::new(e) as Box<dyn Engine>),
        #[cfg(all(feature = "skia-metal", target_vendor = "apple"))]
        skia_ad::SkiaMtl::NAME => skia_ad::SkiaMtl::new().map(|e| Box::new(e) as Box<dyn Engine>),
        #[cfg(feature = "cherenkov")]
        cherenkov_ad::Cherenkov::NAME => {
            cherenkov_ad::Cherenkov::new().map(|e| Box::new(e) as Box<dyn Engine>)
        }
        #[cfg(feature = "cherenkov-cpu")]
        cherenkov_cpu_ad::Cherenkov::NAME => {
            cherenkov_cpu_ad::Cherenkov::new().map(|e| Box::new(e) as Box<dyn Engine>)
        }
        _ => Err(BenchError::Engine(format!(
            "unknown or uncompiled engine {name:?}; available: {:?}",
            engine_names()
        ))),
    }
}

/// Reads the host CPU model name, best-effort.
///
/// On Android `/proc/cpuinfo` has no `model name` line and sysinfo's
/// brand is empty, so the `Hardware` line is used; everywhere else
/// sysinfo's CPU brand answers.
#[must_use]
pub fn cpu_model() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in cpuinfo.lines() {
                if let Some(v) = line
                    .strip_prefix("Hardware")
                    .and_then(|s| s.split(':').nth(1))
                {
                    return Some(v.trim().to_owned());
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "android"))]
    {
        sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::everything()),
        )
        .cpus()
        .first()
        .map(|cpu| cpu.brand().to_owned())
        .filter(|brand| !brand.is_empty())
    }
}

/// Reads the first thermal zone temperature, in °C. Best-effort; `None`
/// where unavailable.
#[must_use]
pub fn thermal_celsius() -> Option<f64> {
    let rd = std::fs::read_dir("/sys/class/thermal").ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("thermal_zone") {
            continue;
        }
        if let Ok(t) = std::fs::read_to_string(entry.path().join("temp"))
            && let Ok(millideg) = t.trim().parse::<f64>()
        {
            return Some(millideg / 1000.0);
        }
    }
    None
}
