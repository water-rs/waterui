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
    /// Submission wall time excluding explicitly measured GPU waits. For CPU
    /// engines this includes rasterization. This is not process CPU time.
    pub submit_seconds: f64,
    /// Explicit GPU completion waits inside submission, excluded above.
    pub gpu_wait_seconds: f64,
    /// GPU seconds from real GPU timestamps where the backend provides
    /// them; `null` otherwise. Never estimated.
    pub gpu_seconds: Option<f64>,
    /// Encode-to-completion wall time, only when this run waits for each frame.
    /// This is synchronous latency, not inferred pipelined throughput.
    pub latency_seconds: Option<f64>,
    /// When this frame's rendering completed, in host seconds since the
    /// measured window opened, from the adapter's completion signal
    /// ([`MeasureReport::completion_signal`] says which, and how
    /// precisely); `null` where the adapter has none. The signal is armed
    /// after every timed interval of the frame, so no stage time and no
    /// `latency_seconds` includes it. Under `wait_for_completion` it is
    /// armed only after the frame's latency wait returned: a signal that
    /// fires on a later callback (wgpu, the Skia Metal marker) then
    /// reports an instant after that wait, as does a Skia Vulkan sync file
    /// the driver exports already signaled; a signal observed inside
    /// submit (CPU engines, Vello classic) or a recorded fence time (Skia
    /// Vulkan) keeps its own instant.
    pub completed_seconds: Option<f64>,
    /// Host seconds from the previous measured frame's completion to this
    /// one's: the measured pipelined frame time. `null` for the first
    /// measured frame and where the adapter has no completion signal.
    pub completion_interval_seconds: Option<f64>,
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
    /// What this frame's composition kept on chip and why the rest was
    /// materialized (`measure --composition`, issue #2547); omitted when
    /// the run collected no attribution.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub composition: Option<crate::Composition>,
    /// The Mali hardware counters this frame's GPU work added
    /// (`measure --gpu-counters`, [`crate::gpu_counters`]); omitted when
    /// the run read none.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gpu_counters: Option<GpuCounterSample>,
}

/// One frame's Mali hardware counters: what accumulated between a sample
/// taken with the GPU idle before the frame and one taken with the GPU
/// idle after it. Cycles are counted at each block's own clock.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct GpuCounterSample {
    /// External memory read beats, summed over L2 slices; not bytes.
    pub external_read_beats: u64,
    /// External memory write beats, summed over L2 slices; not bytes.
    pub external_write_beats: u64,
    /// Cycles the shader cores were executing work, summed over cores —
    /// the frame's cost.
    pub shader_core_active_cycles: u64,
    /// Shader-core cycles running fragment work, summed over cores.
    pub fragment_active_cycles: u64,
    /// Shader-core cycles running compute and vertex work, summed over
    /// cores.
    pub non_fragment_active_cycles: u64,
    /// Fragment warps the shader cores ran.
    pub fragment_warps: u64,
    /// Cycles the GPU front end was active.
    pub gpu_active_cycles: u64,
    /// Cycles the tiler was active.
    pub tiler_active_cycles: u64,
}

/// The GPU a `measure --gpu-counters` run read its counters from.
#[derive(Clone, Debug, Serialize)]
pub struct GpuCountersInfo {
    /// The GPU product whose counter layout was read.
    pub gpu: &'static str,
    /// Shader cores the counters are summed over.
    pub shader_cores: u32,
    /// L2 slices included in the external-memory counters.
    pub l2_slices: u32,
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
    /// Joules per measured frame; absent for a zero-submission stopped interval.
    pub joules_per_frame: Option<f64>,
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
    /// `total_joules` per measured frame; absent for a zero-submission stopped interval.
    pub joules_per_frame: Option<f64>,
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

/// Time the GPU spent powered at each clock over a measured window.
#[derive(Clone, Debug, Serialize)]
pub struct GpuClockReport {
    /// One entry per clock the driver lists, in its order.
    pub residency: Vec<ClockResidency>,
}

/// Time at one GPU clock.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClockResidency {
    /// The clock in kHz.
    pub khz: u64,
    /// Powered milliseconds at this clock within the window.
    pub milliseconds: u64,
}

/// A headless engine retained without submitting frames after a scroll sweep.
/// This excludes application scheduling and system display-compositor work.
#[derive(Debug, Serialize)]
pub struct StoppedInterval {
    /// Wall time after the final GPU completion.
    pub seconds: f64,
    /// Rendering submissions made during the interval.
    pub submissions: u32,
    /// Energy over this interval, without a per-frame normalization.
    pub energy: Option<EnergyReport>,
}

/// `measure` output.
#[derive(Debug, Serialize)]
pub struct MeasureReport {
    /// Adapter key.
    pub engine: &'static str,
    /// The adapter's pinned provenance.
    pub info: EngineInfo,
    /// Why GPU execution time is unavailable for this adapter, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_timing_unavailable: Option<&'static str>,
    /// The backend signal each sample's `completed_seconds` comes from and
    /// its precision; omitted where the adapter has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_signal: Option<&'static str>,
    /// Why this adapter cannot report frame completion; completion fields
    /// are then `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_unavailable: Option<&'static str>,
    /// Whether every frame was explicitly completed before the next encode.
    pub wait_for_completion: bool,
    /// Scene directory name.
    pub scene: String,
    /// Scene pixel size — the native surface size under `--native`.
    pub width: u32,
    /// Scene pixel size — the native surface size under `--native`.
    pub height: u32,
    /// The `--native` retarget of this run; `null` when the run rendered
    /// at the scene's own size.
    pub native: Option<NativeResolution>,
    /// Root-content scroll endpoint in logical scene coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_extent: Option<[f64; 2]>,
    /// The no-submission interval following a scroll sweep.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<StoppedInterval>,
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
    /// The GPU's clock residency over the measured window
    /// ([`crate::gpu_clock`]); `null` where the platform keeps no
    /// residency table.
    pub gpu_clock: Option<GpuClockReport>,
    /// The GPU the per-frame hardware counters came from
    /// (`--gpu-counters`); omitted when the run read none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_counters: Option<GpuCountersInfo>,
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
    /// `completion_interval_seconds` percentiles `[p50, p90, p99]`; `null`
    /// when the adapter has no completion signal or fewer than two frames
    /// were measured.
    pub completion_interval_seconds: Option<[f64; 3]>,
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

/// One load-level probe of a `capacity` sweep: p99 synchronous completion
/// latency at repetition factor `k`.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CapacityProbe {
    /// The draw-list repetition factor probed.
    pub k: u32,
    /// p99 synchronous encode-to-completion seconds at `k`.
    /// Each probe waits for the frame to finish; this is not an estimate
    /// of pipelined throughput from CPU/GPU stage times.
    pub p99_seconds: f64,
}

/// One engine's `capacity` result on one scene: the largest repetition
/// factor whose p99 synchronous completion latency stayed within the budget.
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
