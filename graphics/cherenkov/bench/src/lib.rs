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
//! | `vello-hybrid` | `vello_gpu` | CPU strips + wgpu colour pass | `Rgba8Unorm`/`Bgra8Unorm` texture, picked from queried capabilities |
//! | `skia-cpu` | `skia-safe` | CPU raster (`SkRasterPipeline`) | `RGBAF16` premul surface, linear-P3 colours |
//! | `skia-vulkan` | `skia-safe` | Ganesh/Vulkan | `RGBAF16` premul surface, linear-P3 colours |
//! | `skia-metal` | `skia-safe` | Graphite/Metal | `RGBAF16` premul render target, linear-P3 colours |
//!
//! GPU execution time is separate from host submission time and synchronous
//! encode-to-completion latency. Unavailable GPU time remains `null`, with
//! the adapter's reason in the measurement report. Skia Graphite Metal,
//! Ganesh Vulkan and Vello hybrid lack usable GPU timing in the pinned versions.
//! The Cherenkov GPU adapter collects timestamps from completed submissions.
//!
//! A frame's completion instant comes from each adapter's own backend signal
//! ([`Engine::completion_signal`]); the measured pipelined frame time is the
//! interval between consecutive completions, never derived from stage times.
#![expect(
    clippy::exhaustive_enums,
    reason = "publish = false: no semver surface until #2687 makes the crate publishable"
)]

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
pub mod gpu_clock;
pub mod gpu_counters;
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

use std::collections::{BTreeMap, BTreeSet};

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
    /// `Engine::memory()` filter-chain target bytes at the end of the run;
    /// `None` for other engines.
    pub memory_filter_target_bytes: Option<u64>,
    /// `Engine::memory()` filter-chain uploaded image bytes at the end of
    /// the run; `None` for other engines.
    pub memory_filter_image_bytes: Option<u64>,
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

/// Resolves an in-flight [`Completion`] once all submitted work is done.
pub type ResolveCompletion = Box<dyn FnOnce() -> Result<std::time::Instant, BenchError>>;

/// When one submitted frame's rendering completed, on the host's monotonic
/// clock ([`std::time::Instant`]), as the adapter's backend signals it.
pub enum Completion {
    /// Rendering finished at this instant, already known when
    /// [`Engine::completion`] runs: the end of a CPU engine's raster work
    /// or the return of a completion wait the adapter performs inside
    /// submission.
    Finished(std::time::Instant),
    /// Rendering is still in flight. The frame loop calls the resolver only
    /// after it has waited for all submitted work; it returns the instant
    /// the backend's signal recorded, or an error when the signal never
    /// arrived.
    InFlight(ResolveCompletion),
}

impl std::fmt::Debug for Completion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finished(at) => f.debug_tuple("Finished").field(at).finish(),
            Self::InFlight(_) => f.write_str("InFlight"),
        }
    }
}

impl Completion {
    /// A completion a backend callback reports, for backends that run the
    /// callback synchronously inside the drain (wgpu's device poll): by the
    /// time the frame loop resolves it the callback has run, or never will.
    #[must_use]
    pub fn callback() -> (std::sync::mpsc::SyncSender<std::time::Instant>, Self) {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let resolve = move || {
            receive.try_recv().map_err(|_| {
                BenchError::Engine(
                    "the completion callback had not run after the GPU drained".into(),
                )
            })
        };
        (send, Self::InFlight(Box::new(resolve)))
    }

    /// A completion a backend reports from its own thread some time after
    /// the GPU finished (Metal's completed handlers): resolving waits for
    /// that callback, which the backend runs for every committed command
    /// buffer, and fails if it is dropped without running.
    #[cfg(all(feature = "skia-metal", target_vendor = "apple"))]
    #[must_use]
    pub fn deferred_callback() -> (std::sync::mpsc::SyncSender<std::time::Instant>, Self) {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let resolve = move || {
            receive.recv().map_err(|_| {
                BenchError::Engine("the completion callback was dropped without running".into())
            })
        };
        (send, Self::InFlight(Box::new(resolve)))
    }
}

/// A completion instant an adapter observes inside [`Engine::submit`] —
/// where its rendering really ends — held until [`Engine::completion`]
/// hands it out as [`Completion::Finished`].
#[derive(Debug, Default)]
pub struct ObservedCompletion(Option<std::time::Instant>);

impl ObservedCompletion {
    /// Records the submitted frame's completion instant.
    pub const fn record(&mut self, at: std::time::Instant) {
        self.0 = Some(at);
    }

    /// The recorded instant of the last submitted frame, for
    /// [`Engine::completion`].
    ///
    /// # Errors
    /// [`BenchError::Engine`] when no submission recorded one since the
    /// last call.
    pub fn take(&mut self, engine: &str) -> Result<Option<Completion>, BenchError> {
        self.0
            .take()
            .map(|at| Some(Completion::Finished(at)))
            .ok_or_else(|| {
                BenchError::Engine(format!(
                    "{engine}: completion requested without a submitted frame"
                ))
            })
    }
}

#[cfg(any(
    feature = "cherenkov",
    feature = "vello-classic",
    feature = "vello-hybrid"
))]
impl Completion {
    /// The [`CompletionSignal::Measured`] text of adapters whose frames
    /// complete through [`Completion::on_submitted_work_done`].
    pub const WGPU_SIGNAL: &'static str = "wgpu Queue::on_submitted_work_done registered after the \
        frame's last submission, outside the timed submit; the instant is when wgpu ran the callback, \
        which it does only inside a later device poll or queue submission, so it bounds the GPU's \
        completion from above";

    /// The completion of everything submitted to `queue` so far: a
    /// [`wgpu::Queue::on_submitted_work_done`] callback that stamps the
    /// instant it runs. Registering it records no command and waits for
    /// nothing; wgpu runs it in whichever device poll or queue submission
    /// first sees that work done.
    #[must_use]
    pub fn on_submitted_work_done(queue: &wgpu::Queue) -> Self {
        let (send, completion) = Self::callback();
        queue.on_submitted_work_done(move || {
            // A dropped receiver means the run already failed; the frame
            // loop reports a missing instant itself.
            let _ = send.send(std::time::Instant::now());
        });
        completion
    }
}

/// How an adapter's [`Engine::completion`] is obtained, or why it is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionSignal {
    /// [`Engine::completion`] returns a [`Completion`] for every
    /// submission; the text names the backend signal and its precision.
    Measured(&'static str),
    /// [`Engine::completion`] returns none; the text says why. Completion
    /// fields are then `null`, never substituted.
    Unavailable(&'static str),
}

/// One CPU phase of a submitted frame, as the adapter names it.
#[derive(Clone, Debug, Serialize)]
pub struct PhaseSample {
    /// The phase's name: `"lower"`, `"encode"`, `"stamp"` and `"wait"` for
    /// Cherenkov GPU; `"lower"`, `"glyph"` and `"shade"` for Cherenkov CPU.
    pub name: String,
    /// Wall-clock seconds the engine spent in it.
    pub seconds: f64,
}

impl PhaseSample {
    /// One sample per `(name, seconds)`, in the order given.
    #[must_use]
    pub fn list(phases: &[(&str, f64)]) -> Vec<Self> {
        phases
            .iter()
            .map(|&(name, seconds)| Self {
                name: name.to_owned(),
                seconds,
            })
            .collect()
    }
}

/// What one frame's composition kept on chip and what it materialized
/// (issue #2547).
///
/// Summed over the frame's encoded surfaces, so the counters sit beside
/// that frame's timing sample. Only an adapter whose planner decides
/// on-chip composition reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Composition {
    /// Tile epochs the frame encoded.
    pub epochs: u32,
    /// Canonical passes the frame's plan held.
    pub passes: u32,
    /// Logical images an epoch kept in an attachment slot.
    pub on_chip_images: u32,
    /// Logical bytes those images would have cost had they been
    /// materialized.
    pub on_chip_logical_bytes: u64,
    /// Logical images written to device memory.
    pub materialized_images: u32,
    /// Logical bytes of the materialized images' regions.
    pub materialized_logical_bytes: u64,
    /// Per-reason counts of the materialized images, keyed by the
    /// planner's stable reason names (`"neighborhood-read"`,
    /// `"slot-limit"`, ...). Sorted, so the JSON is stable.
    pub reasons: BTreeMap<&'static str, u32>,
    /// Peak tile attachment bytes the frame reserved — storage it
    /// expects tile memory, not device memory, to hold.
    pub peak_reserved_bytes: u64,
    /// What the device reports it actually committed for those
    /// attachments. `None` where the platform has no such query
    /// (Apple's memoryless textures), so the reserved figure is all
    /// that is known.
    pub peak_committed_bytes: Option<u64>,
    /// Peak device-memory attachment bytes the composition plan governs.
    pub peak_backed_bytes: u64,
    /// Logical bytes written into intermediates.
    pub logical_bytes_written: u64,
    /// Logical bytes sampled from intermediates.
    pub logical_bytes_read: u64,
    /// Logical bytes copied between images (snapshots and captures).
    pub logical_bytes_copied: u64,
    /// Read-after-write boundaries the backend had to order.
    pub barriers: u32,
    /// Tile composition pipelines created on first use in this frame.
    pub pipelines_created: u32,
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
    /// The frame's CPU phases in render order; empty when the adapter does
    /// not expose phase timings.
    pub phases: Vec<PhaseSample>,
    /// What this frame's composition kept on chip and why it materialized
    /// the rest; `None` when the adapter reports no composition
    /// attribution (#2547).
    pub composition: Option<Composition>,
    /// Wall-clock seconds this call spent rasterizing before any readback
    /// began (completion waits included), where the
    /// adapter separates the two stages; `None` otherwise.
    pub render_seconds: Option<f64>,
    /// Seconds spent explicitly waiting for GPU completion during submission.
    /// Excluded from CPU submission work, retained in synchronous frame latency.
    pub gpu_wait_seconds: f64,
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
    /// Additional root-content scroll in scene coordinates for this frame.
    /// `None` retains the static workload's recording behavior.
    pub scroll_offset: Option<kurbo::Vec2>,
}

#[cfg(any(
    feature = "skia",
    feature = "skia-metal",
    feature = "vello-classic",
    feature = "vello-cpu",
    feature = "vello-hybrid"
))]
impl EncodeInput<'_> {
    /// Map a root-content scroll into the device-space recording transform.
    pub(crate) fn scroll_transform(&self) -> Option<kurbo::Affine> {
        self.scroll_offset.map(|offset| {
            let root = self.scene.root.transform;
            root * kurbo::Affine::translate(-offset) * root.inverse()
        })
    }
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
    /// Why this adapter cannot report GPU execution time using engine APIs.
    fn gpu_timing_unavailable(&self) -> Option<&'static str> {
        None
    }
    /// The backend signal [`Engine::completion`] comes from, or why the
    /// adapter has none.
    fn completion_signal(&self) -> CompletionSignal;
    /// The completion of the frame the last [`Engine::submit`] submitted.
    /// The frame loop calls it once per submission, after the submit's
    /// timed interval has closed, so the work a completion signal needs —
    /// registering a callback, committing a marker, exporting a fence,
    /// creating the next frame's semaphore — is never part of a timed
    /// stage. An instant the adapter observed inside submit (a CPU
    /// engine's raster end, a device wait's return) is recorded there and
    /// handed out here.
    ///
    /// Returns `None` exactly when [`Engine::completion_signal`] is
    /// [`CompletionSignal::Unavailable`].
    ///
    /// # Errors
    /// [`BenchError`] when no submission awaits its completion, or the
    /// backend fails to arm the signal.
    fn completion(&mut self) -> Result<Option<Completion>, BenchError>;
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
    /// Wait for all submitted rendering work to complete, then return GPU
    /// timings not yet returned by [`Engine::submit`]. GPU adapters must
    /// wait even when timestamps are unavailable. The default applies only
    /// to adapters whose submit already completes all rendering work.
    ///
    /// # Errors
    /// [`BenchError`] on engine or GPU failure.
    fn finish_gpu(&mut self) -> Result<Vec<GpuSample>, BenchError> {
        Ok(Vec::new())
    }
    /// Wait for submitted rendering to complete without collecting GPU timestamps.
    /// CPU adapters whose submit completes rendering return immediately.
    ///
    /// # Errors
    /// [`BenchError`] when rendering completion fails.
    fn wait_idle(&mut self) -> Result<(), BenchError>;
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
        vello_cpu_ad::VelloCpu::NAME => Ok(Box::new(vello_cpu_ad::VelloCpu::new()?)),
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
/// brand is empty, so the `Hardware` line is used. iOS uses only the
/// system-reported brand, without inferring a chip from its device model.
/// Other platforms use sysinfo's CPU brand.
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
    #[cfg(target_os = "ios")]
    {
        let name = c"machdep.cpu.brand_string";
        let mut length = 0;
        // SAFETY: the name is NUL-terminated and length points to writable
        // storage. A null output buffer requests the required byte count.
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                std::ptr::null_mut(),
                &raw mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return None;
        }
        let mut bytes = vec![0_u8; length];
        // SAFETY: bytes owns the output allocation sized by the same sysctl;
        // length describes its capacity. No new value is supplied.
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                &raw mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return None;
        }
        bytes.truncate(length);
        let brand = std::ffi::CStr::from_bytes_with_nul(&bytes)
            .ok()?
            .to_str()
            .ok()?;
        (!brand.is_empty()).then(|| brand.to_owned())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
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
