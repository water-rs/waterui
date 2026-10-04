//! Serialized report shapes for `render` and `measure` — always written
//! with serde, never concatenated.

use std::collections::BTreeMap;

use cherenkov_oracle::Metrics;
use serde::Serialize;

use crate::memory::{MemoryReport, MemorySnapshot};
use crate::{Counters, DeviceInfo, EngineInfo, PassSample, PhaseSample};

/// `render` output: the engine provenance, the correctness metrics against
/// the oracle, and the counters of what was submitted.
#[derive(Debug, Serialize)]
pub struct RenderReport {
    /// Adapter key.
    pub engine: &'static str,
    /// The adapter's pinned provenance.
    pub info: EngineInfo,
    /// Scene directory name.
    pub scene: String,
    /// Scene pixel size.
    pub width: u32,
    /// Scene pixel size.
    pub height: u32,
    /// The presentation this render went through (`render --present`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<PresentInfo>,
    /// Metrics against the oracle reference.
    pub metrics: Metrics,
    /// What the adapter issued.
    pub counters: Counters,
    /// Idle, steady-state and peak memory readings for this scene.
    pub memory: MemoryReport,
    /// Device/thermal metadata at render time.
    pub device: DeviceInfo,
}

/// The presentation a `render --present` run went through.
#[derive(Clone, Debug, Serialize)]
pub struct PresentInfo {
    /// The output kind (`srgb-hw`, `srgb-shader`, `linear-p3`).
    pub kind: &'static str,
    /// The scene's `present_headroom` the reference was presented at.
    pub headroom: f64,
}

/// One measured frame: raw samples, not percentiles.
#[derive(Clone, Debug, Serialize)]
pub struct FrameSample {
    /// Wall-clock seconds of the CPU encode phase.
    pub encode_seconds: f64,
    /// Wall-clock seconds of submit + device wait. For CPU engines this is
    /// the rasterization cost; for GPU engines it brackets queue submission
    /// and device poll (host-side wall time, not GPU time).
    pub submit_seconds: f64,
    /// GPU seconds from real GPU timestamps where the backend provides
    /// them; `null` otherwise. Never estimated.
    pub gpu_seconds: Option<f64>,
    /// CPU the measuring thread ran on when the timed phases started
    /// (`sched_getcpu`); `null` where the kernel does not report it.
    pub cpu_start: Option<u32>,
    /// CPU the measuring thread ran on when the timed phases ended.
    pub cpu_end: Option<u32>,
    /// True when the thread migrated between `cpu_start` and `cpu_end`.
    pub migrated: bool,
    /// Per-pass GPU timings for this frame, in submission order; omitted
    /// when the backend provides none.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub passes: Vec<PassSample>,
    /// Per-phase render-thread CPU timings for this frame, in render
    /// order; omitted when the backend provides none.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub phases: Vec<PhaseSample>,
}

/// CPU placement of a `measure` run.
///
/// On big.LITTLE hardware (differing `cpuinfo_max_freq` across CPUs) an
/// unpinned run is uncontrolled — samples spread across clusters and
/// percentiles blend them.
#[derive(Clone, Debug, Serialize)]
pub struct Placement {
    /// Sorted CPU ids passed via `--cpu`; `null` when unpinned.
    pub requested: Option<Vec<u32>>,
    /// Whether the measuring thread was pinned to `requested` before
    /// engine creation (threads spawned later inherit the mask).
    pub controlled: bool,
    /// True when the host's CPUs report differing `cpuinfo_max_freq` —
    /// placement matters; an uncontrolled run blends cluster speeds.
    pub heterogeneous: bool,
    /// CPU the one-time `prepare` phase started on.
    pub prepare_cpu: Option<u32>,
    /// Per-CPU counts: a sample counts once under each CPU it touched,
    /// keyed by CPU id, with that CPU's `cpuinfo_max_freq` kHz where
    /// sysfs exposes it.
    pub cpus: BTreeMap<u32, CpuUse>,
    /// Samples that migrated between their start and end CPU.
    pub migrated: u32,
}

/// Per-CPU tally in [`Placement::cpus`].
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CpuUse {
    /// Timed-phase endpoints observed on this CPU (a migrated sample
    /// contributes one to each of its start and end CPUs).
    pub count: u32,
    /// `cpuinfo_max_freq` kHz where sysfs exposes it.
    pub max_freq_khz: Option<u64>,
}

/// `render`/`measure` output when the scene declares a feature the
/// adapter does not implement — recorded, never emulated.
#[derive(Debug, Serialize)]
pub struct UnsupportedReport {
    /// Adapter key.
    pub engine: &'static str,
    /// The adapter's pinned provenance.
    pub info: EngineInfo,
    /// Scene directory name.
    pub scene: String,
    /// The scene feature the adapter cannot execute faithfully.
    pub unsupported: cherenkov_scene::Feature,
    /// The upstream API the engine lacks, when the adapter knows it
    /// (e.g. `peniko::Extend` has no `None` variant). `null` when the
    /// limitation is adapter coverage rather than a named upstream API.
    pub missing_api: Option<&'static str>,
    /// Engine and process memory before any scene was prepared.
    pub memory_idle: MemorySnapshot,
}

/// Frame pacing of a `measure` run at a fixed rate (`--rate`).
///
/// Measured frame `n` is started on the deadline `start + n / rate`;
/// a frame that begins after its deadline counts one miss. Only
/// comparable energy-per-frame numbers come from a paced run.
#[derive(Clone, Debug, Serialize)]
pub struct Pacing {
    /// Requested frame rate in Hz.
    pub requested_hz: f64,
    /// Measured frames per second actually achieved over the window.
    pub achieved_hz: f64,
    /// Measured frames that started after their deadline.
    pub missed_deadlines: u32,
    /// Seconds from the first measured frame's deadline to the end of
    /// the last measured frame.
    pub window_seconds: f64,
}

/// Energy drawn by one rail (Android ODPM) or power domain (macOS
/// `powermetrics`) over the measured window.
#[derive(Clone, Debug, Serialize)]
pub struct RailEnergy {
    /// Human-readable subsystem label from `enabled_rails` (e.g.
    /// `Cellular`); `null` where the meter names no subsystem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    /// Joules over the window.
    pub joules: f64,
    /// Joules per measured frame.
    pub joules_per_frame: f64,
    /// Average watts over the window.
    pub watts: f64,
}

/// Energy measured over the paced window (`--energy`).
///
/// On Android this is every ODPM rail under
/// `/sys/bus/iio/devices/iio:device*/` sampled once before the first
/// measured frame and once after the last; on macOS it is the
/// `powermetrics` CPU/GPU/ANE package energy over the same window.
/// On iOS it is the calling process's `ri_energy_nj` delta, excluding
/// display and compositor-server energy.
#[derive(Clone, Debug, Serialize)]
pub struct EnergyReport {
    /// Meter source: `odpm`, `powermetrics` or `proc_pid_rusage_v6`.
    /// The iOS source covers only the measured process.
    pub source: &'static str,
    /// Seconds the energy window covered.
    pub window_seconds: f64,
    /// Per-rail / per-domain energy, keyed by rail name (e.g.
    /// `S2M_VDD_CPUCL2`) or domain (`cpu`, `gpu`, `ane`, `dram`).
    pub rails: BTreeMap<String, RailEnergy>,
    /// Sum of all rails, in joules.
    pub total_joules: f64,
    /// `total_joules` per measured frame.
    pub joules_per_frame: f64,
    /// `total_joules` per second (average power).
    pub watts: f64,
}

/// Device conditions at measure time — everything that skews a
/// performance or energy number besides the engine itself.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Conditions {
    /// Thermal status word where readable: Android `dumpsys
    /// thermalservice` severity (`none`/`light`/`moderate`/`severe`/
    /// `critical`/`emergency`/`shutdown`) or macOS `powermetrics`
    /// `thermal_pressure`.
    pub thermal_status: Option<String>,
    /// First readable thermal zone temperature in °C
    /// (`/sys/class/thermal`).
    pub thermal_celsius: Option<f64>,
    /// Screen state where readable (`awake`, `dozing`, `asleep`, `on`,
    /// `off`, ...).
    pub screen_state: Option<String>,
    /// Screen brightness where readable (0–255 on Android).
    pub screen_brightness: Option<u32>,
}

/// The native-resolution retarget of a `measure`/`capacity` run
/// (`--native`).
///
/// The engine rendered into a `width`×`height` surface with the scene
/// drawn under the uniform `scale` — a device-pixel-ratio scale — instead
/// of the scene's own pixel size.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct NativeResolution {
    /// Surface width in pixels.
    pub width: u32,
    /// Surface height in pixels.
    pub height: u32,
    /// The uniform scale applied to the scene tree,
    /// `width / scene_width`.
    pub scale: f64,
}

/// `measure` output.
#[derive(Debug, Serialize)]
pub struct MeasureReport {
    /// Adapter key.
    pub engine: &'static str,
    /// The adapter's pinned provenance.
    pub info: EngineInfo,
    /// Scene directory name.
    pub scene: String,
    /// Scene pixel size — the native surface size under `--native`.
    pub width: u32,
    /// Scene pixel size — the native surface size under `--native`.
    pub height: u32,
    /// The `--native` retarget of this run; `null` when the run rendered
    /// at the scene's own size.
    pub native: Option<NativeResolution>,
    /// One-time scene preparation in seconds — resource creation and
    /// scene conversion done once outside the timed loop (fonts,
    /// images, GPU uploads, immutable shaders). Not a per-frame cost.
    pub prepare_seconds: f64,
    /// Warmup frames discarded.
    pub warmup_frames: u32,
    /// Raw per-frame samples (one per measured frame).
    pub samples: Vec<FrameSample>,
    /// CPU placement of the run (see [`Placement`]).
    pub placement: Placement,
    /// p50/p90/p99 over the raw samples, per field.
    pub percentiles: Percentiles,
    /// Frame pacing (`--rate`); `null` when the run was flat out.
    pub pacing: Option<Pacing>,
    /// Energy measured over the window (`--energy`); `null` when energy
    /// was not requested.
    pub energy: Option<EnergyReport>,
    /// Conditions that contextualise the numbers — thermal status,
    /// screen state and brightness, alongside [`Placement`].
    pub conditions: Conditions,
    /// What the adapter issued per frame.
    pub counters: Counters,
    /// Idle, steady-state and peak memory readings for this scene.
    pub memory: MemoryReport,
    /// Device/thermal metadata at measure time.
    pub device: DeviceInfo,
}

/// Percentiles of each measured field.
#[derive(Clone, Debug, Serialize)]
pub struct Percentiles {
    /// `encode_seconds` percentiles `[p50, p90, p99]`.
    pub encode_seconds: [f64; 3],
    /// `submit_seconds` percentiles `[p50, p90, p99]`.
    pub submit_seconds: [f64; 3],
    /// `gpu_seconds` percentiles `[p50, p90, p99]`; `null` when the backend
    /// provides no GPU timestamps.
    pub gpu_seconds: Option<[f64; 3]>,
    /// Per-pass GPU percentiles, grouped by pass index within the frame;
    /// omitted when the backend provides no per-pass timings.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub passes: Vec<PassPercentiles>,
    /// Per-phase CPU percentiles, grouped by phase index within the frame;
    /// omitted when the backend provides no phase timings.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub phases: Vec<PhasePercentiles>,
}

/// Percentiles of one render pass across the measured frames.
#[derive(Clone, Debug, Serialize)]
pub struct PassPercentiles {
    /// The pass's name (`"surface"`, `"scratch{n}"`, ...).
    pub name: String,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target texture format.
    pub format: String,
    /// `gpu_seconds` percentiles `[p50, p90, p99]` for this pass; `null`
    /// when no frame's timestamps span it.
    pub gpu_seconds: Option<[f64; 3]>,
}

/// Percentiles of one render-thread CPU phase across the measured frames.
#[derive(Clone, Debug, Serialize)]
pub struct PhasePercentiles {
    /// The phase's name (`"lower"`, `"encode"`, `"stamp"`, `"wait"`).
    pub name: String,
    /// `seconds` percentiles `[p50, p90, p99]` for this phase.
    pub seconds: [f64; 3],
}

/// One load-level probe of a `capacity` sweep: the p99 frame time the
/// engine sustained at repetition factor `k`.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CapacityProbe {
    /// The draw-list repetition factor probed.
    pub k: u32,
    /// p99 frame seconds at `k`. A frame's seconds are
    /// `encode_seconds + submit_seconds`, raised to the frame's
    /// `gpu_seconds` where the backend reports them — the throughput
    /// limit of a pipelined renderer is its slowest stage.
    pub p99_seconds: f64,
}

/// One engine's `capacity` result on one scene: the largest repetition
/// factor whose p99 frame time stayed within the budget.
#[derive(Debug, Serialize)]
pub struct CapacityResult {
    /// Adapter key.
    pub engine: &'static str,
    /// The adapter's pinned provenance.
    pub info: EngineInfo,
    /// The scene feature the adapter cannot execute, when the sweep never
    /// ran; `null` on a completed sweep.
    pub unsupported: Option<cherenkov_scene::Feature>,
    /// The upstream API the engine lacks, when known (see
    /// [`UnsupportedReport::missing_api`]).
    pub missing_api: Option<&'static str>,
    /// The probe error that stopped the sweep mid-run, when one did.
    /// `max_k` is then the largest `k` sustained before the failure, and
    /// `p99_seconds_next` the over-budget or failed level.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
    /// The largest probed `k` within the frame budget. `0` means even
    /// `k = 1` exceeded it.
    pub max_k: u32,
    /// p99 frame seconds at `max_k`; `null` when `max_k` is `0`.
    pub p99_seconds: Option<f64>,
    /// p99 frame seconds at `max_k + 1` — the first over-budget level;
    /// `null` when the sweep saturated at `--max-k` without exceeding the
    /// budget.
    pub p99_seconds_next: Option<f64>,
    /// Every probe of the sweep, in the order it ran.
    pub probes: Vec<CapacityProbe>,
    /// Idle, steady-state and peak memory across the sweep's probes —
    /// `peak` is the field-wise maximum over every probed `k`, so it
    /// reflects the heaviest level measured; `idle` is the engine's
    /// pre-scene baseline.
    pub memory: MemoryReport,
}

/// `capacity` output for one scene: the sweep result per engine.
#[derive(Debug, Serialize)]
pub struct CapacityReport {
    /// Scene directory name.
    pub scene: String,
    /// The frame-time budget in milliseconds (`--budget-ms`).
    pub budget_ms: f64,
    /// Measured frames per probe (`--frames`).
    pub frames: u32,
    /// Warmup frames discarded per probe (`--warmup`).
    pub warmup: u32,
    /// The `--native` retarget every probe ran under; `null` when probes
    /// rendered at the scene's own size.
    pub native: Option<NativeResolution>,
    /// One result per engine, in command-line order.
    pub results: Vec<CapacityResult>,
    /// Device/thermal metadata of the host.
    pub device: DeviceInfo,
}

/// Nearest-rank percentiles of `samples`, `[p50, p90, p99]`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "sample counts are small positive values"
)]
pub fn percentiles(samples: &[f64]) -> Option<[f64; 3]> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    let at = |p: f64| {
        let idx = (((v.len() as f64) * p).ceil() as usize).saturating_sub(1);
        v[idx.min(v.len() - 1)]
    };
    Some([at(0.50), at(0.90), at(0.99)])
}
