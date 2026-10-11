//! `cherenkov-bench`: cross-engine correctness and performance runner.
//!
//! The corpora (`scenes/corpus`, `scenes/perf`) and fonts
//! (`scenes/fonts`) are generated, not committed — run
//! `python3 scenes/tools/generate.py` first.
//!
//! - `render --engine E --scene DIR --out FILE` renders one scene against
//!   the oracle and writes the engine's image, an error heatmap and a
//!   metrics JSON. `--corpus DIR --out-dir DIR` sweeps a corpus.
//! - `measure --engine E --scene DIR --frames N --warmup W --out FILE`
//!   records raw per-frame CPU encode, submit and GPU seconds (real
//!   timestamps only) plus p50/p90/p99. `--cpu LIST` pins the run to a
//!   CPU set — required for meaningful numbers on big.LITTLE hosts
//!   (Linux and Android only); the report records placement either way.
//!   `--rate HZ` paces the measured frames to a fixed rate instead of
//!   running flat out; `--energy` brackets the window with the
//!   platform's power meter (Android ODPM rails, root via `su -c`;
//!   macOS `sudo -n powermetrics`) and reports joules per frame.
//!   `--pause-at FRAME` pauses before the zero-based encode frame
//!   (warmup included) until one line is read from stdin.
//!   `--native WxH` renders into a `W`×`H` surface — the device's own
//!   resolution — with the scene drawn under the uniform scale
//!   `W / scene_width`, like a device pixel ratio.
//! - `capacity --engine E [--engine E2 ...] --corpus scenes/perf
//!   --budget-ms 8.333 --out-dir DIR` sweeps each perf scene's load:
//!   the draw list is repeated `k` times (`transform::repeated`),
//!   doubling `k` per interleaved engine round until p99 frame time
//!   exceeds the budget, then binary-searching the largest `k` within
//!   it. The report records each engine's max sustained `k`, the p99
//!   at `k` and at `k + 1`.

use std::collections::BTreeMap;
use std::ffi::OsString;
#[cfg(unix)]
use std::ffi::{CStr, OsStr, c_char, c_int};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::convert;
use crate::memory::{MemoryReport, MemorySnapshot, SampleDetail};
use crate::refcache;
use crate::report::{
    CapacityProbe, CapacityReport, CapacityResult, CpuUse, FrameSample, MeasureReport,
    NativeResolution, Pacing, PassPercentiles, Percentiles, PhasePercentiles, Placement,
    RenderReport, UnsupportedReport, percentiles,
};
use crate::{
    BenchError, DeviceInfo, EncodeInput, Engine, affinity, conditions, create_engine, energy,
    engine_names, transform,
};
use cherenkov_oracle::{F32Image, Renderer, metrics};
use cherenkov_scene::Scene;
use clap::{Parser, Subcommand};

/// Command line.
#[derive(Parser)]
#[command(
    name = "cherenkov-bench",
    about = "Cross-engine correctness and performance suite"
)]
struct Cli {
    /// Subcommand.
    #[command(subcommand)]
    cmd: Sub,
}

/// `present-cost --pattern` choices; see [`crate::present_cost`].
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum PresentPattern {
    /// Every pixel a fully out-of-gamut P3 primary or secondary.
    Oog,
    /// Left half in-gamut, right half out-of-gamut.
    Mixed,
}

/// `external-cost --path` choices (#168): `e` is external-frame import,
/// `c` is copy-and-convert.
///
/// See [`crate::external_cost`].
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ExternalPath {
    /// Cherenkov external-frame import + composite.
    #[value(name = "e")]
    External,
    /// Copy the planes + convert in a `GpuContent` pass.
    #[value(name = "c")]
    Copy,
}

/// `external-cost --size` choices.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ExternalSize {
    /// 1920x1080.
    #[value(name = "1080p")]
    P1080,
    /// 3840x2160.
    #[value(name = "4k")]
    P4k,
}

/// `external-cost --transfer` choices.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ExternalTransfer {
    /// BT.709 video-range 8-bit (NV12).
    Sdr,
    /// BT.2020 PQ video-range 10-bit (P010).
    Pq,
}

/// The `external-cost` options, assembled by [`run`] for
/// `crate::external_cost::run` (#168). Absent without the `cherenkov`
/// feature: the stub command never reads it.
#[cfg(feature = "cherenkov")]
pub(crate) struct ExternalCostArgs {
    /// `e`/`c` — the measured path.
    pub(crate) path: ExternalPath,
    /// Frame size.
    pub(crate) size: ExternalSize,
    /// Color transfer/layout.
    pub(crate) transfer: ExternalTransfer,
    /// Measured frames.
    pub(crate) frames: u32,
    /// Warmup frames.
    pub(crate) warmup: u32,
    /// Pacing rate in Hz. The CLI always supplies one (`default_value_t`).
    pub(crate) rate: f64,
    /// Measure energy.
    pub(crate) energy: bool,
    /// Pinned CPUs.
    pub(crate) cpu: Option<Vec<u32>>,
    /// Report JSON path.
    pub(crate) out: PathBuf,
}

#[derive(Subcommand)]
enum Sub {
    /// Render scene(s) and report correctness metrics vs the oracle.
    Render {
        /// Adapter key (see `cherenkov-bench engines`).
        #[arg(long)]
        engine: String,
        /// One scene directory (`scene.json` + `resources/`).
        #[arg(long, conflicts_with = "corpus", required_unless_present = "corpus")]
        scene: Option<PathBuf>,
        /// Corpus directory; every child holding a `scene.json` is run.
        #[arg(long)]
        corpus: Option<PathBuf>,
        /// Metrics JSON path (with `--scene`).
        #[arg(long, conflicts_with = "out_dir", required_unless_present = "out_dir")]
        out: Option<PathBuf>,
        /// Output directory (with `--corpus`).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Render through the backend's presentation pass into this output
        /// kind instead of reading back the working-space target, and
        /// compare against the oracle's matching presentation of the
        /// scene's `f64` reference. Backends without a presentation step
        /// fail.
        #[arg(long, value_enum)]
        present: Option<crate::PresentKind>,
        /// Directory of pre-rendered `<scene>.ref` oracle images (built by
        /// `cherenkov-bench reference`) to compare against instead of
        /// re-rendering the `f64` reference in this pass. A file whose
        /// fingerprint does not match the scene's current inputs fails.
        #[arg(long, value_name = "DIR")]
        reference: Option<PathBuf>,
    },
    /// Render each scene's `f64` oracle image once into `<scene>.ref`
    /// files that `render --reference` passes share — the oracle's inputs
    /// are the scene alone, so the reference need not be recomputed per
    /// engine or presentation kind. Scenes render in parallel across all
    /// cores.
    Reference {
        /// One scene directory (`scene.json` + `resources/`).
        #[arg(long, conflicts_with = "corpus", required_unless_present = "corpus")]
        scene: Option<PathBuf>,
        /// Corpus directory; every child holding a `scene.json` is cached.
        #[arg(long)]
        corpus: Option<PathBuf>,
        /// Cache directory for the `<scene>.ref` files.
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Measure encode/submit/GPU frame times.
    Measure {
        /// Adapter key.
        #[arg(long)]
        engine: String,
        /// One scene directory.
        #[arg(long, conflicts_with = "corpus", required_unless_present = "corpus")]
        scene: Option<PathBuf>,
        /// Corpus directory.
        #[arg(long)]
        corpus: Option<PathBuf>,
        /// Measured frame count (after warmup).
        #[arg(long, default_value_t = 60)]
        frames: u32,
        /// Warmup frames discarded before measuring.
        #[arg(long, default_value_t = 5)]
        warmup: u32,
        /// Pause before this zero-based encode frame (warmup included)
        /// until one line is read from stdin.
        #[arg(long, value_name = "FRAME")]
        pause_at: Option<u32>,
        /// Report JSON path (with `--scene`).
        #[arg(long, conflicts_with = "out_dir", required_unless_present = "out_dir")]
        out: Option<PathBuf>,
        /// Output directory (with `--corpus`).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Pin the measurement to these CPUs — a list like `7`, `4-6` or
        /// `1,3,5-7`. Required for meaningful numbers on big.LITTLE
        /// hardware; Linux and Android only.
        #[arg(long, value_name = "LIST")]
        cpu: Option<String>,
        /// Pace the measured frames to this rate in Hz — each frame
        /// starts on the deadline `start + n/rate` rather than running
        /// flat out. Energy per frame is only comparable at a fixed
        /// rate.
        #[arg(long, value_name = "HZ")]
        rate: Option<f64>,
        /// Measure energy over the measured window: every ODPM rail on
        /// Android (root — run via `su -c`), `sudo -n powermetrics` on
        /// macOS. Fails when the meter cannot be read rather than
        /// reporting nothing.
        #[arg(long)]
        energy: bool,
        /// Render at the device's native resolution instead of the
        /// scene's own size: the engine draws into a `WxH` surface —
        /// the value a device host's window supplies, given here for an
        /// offscreen run — with the scene under the uniform scale
        /// `W / scene_width`, like a device pixel ratio. Pair with
        /// `--rate` to pace at the panel's refresh (e.g. `--native
        /// 2752x2064 --rate 120` for an iPad Pro).
        #[arg(long, value_name = "WxH")]
        native: Option<String>,
    },
    /// Record the #169 allocation-event diagnostic for one scene.
    ///
    /// Runs the scene through the `cherenkov` adapter with
    /// `GpuConfig::alloc_diag` installed and writes every allocation,
    /// growth, upload and submission boundary — with the wgpu
    /// allocator's state at each — as JSON Lines to `--out`.
    /// Diagnostics only: the per-event allocator snapshot is expensive,
    /// so this mode never joins timed or energy runs.
    AllocDiag {
        /// One scene directory.
        #[arg(long)]
        scene: PathBuf,
        /// Measured frame count (after warmup).
        #[arg(long, default_value_t = 60)]
        frames: u32,
        /// Warmup frames discarded before measuring.
        #[arg(long, default_value_t = 5)]
        warmup: u32,
        /// JSON Lines report path.
        #[arg(long)]
        out: PathBuf,
        /// Render at the device's native resolution (`WxH`; see
        /// `measure --native`).
        #[arg(long, value_name = "WxH")]
        native: Option<String>,
        /// Pace the frames to this rate in Hz (see `measure --rate`).
        #[arg(long, value_name = "HZ")]
        rate: Option<f64>,
        /// Pin the run to these CPUs (see `measure --cpu`).
        #[arg(long, value_name = "LIST")]
        cpu: Option<String>,
    },
    /// Sweep scene load to find each engine's sustained capacity.
    ///
    /// The draw list is repeated `k` times, doubling `k` until a probe's
    /// p99 frame time exceeds `--budget-ms`, then binary-searching the
    /// largest `k` that stays within it. Every probe is one interleaved
    /// round across the given engines, in `--engine` order, with the
    /// same warmup and frame counts as `measure`.
    Capacity {
        /// Adapter key; repeat for each engine in the round.
        #[arg(long, required = true)]
        engine: Vec<String>,
        /// One scene directory.
        #[arg(long, conflicts_with = "corpus", required_unless_present = "corpus")]
        scene: Option<PathBuf>,
        /// Corpus directory — typically `scenes/perf`.
        #[arg(long)]
        corpus: Option<PathBuf>,
        /// Measured frame count per probe (after warmup).
        #[arg(long, default_value_t = 60)]
        frames: u32,
        /// Warmup frames discarded before each probe's measurement.
        #[arg(long, default_value_t = 5)]
        warmup: u32,
        /// Frame-time budget in milliseconds a probe must satisfy: the
        /// 120 fps budget is 8.333 ms at p99.
        #[arg(long, default_value_t = 8.333)]
        budget_ms: f64,
        /// Largest repetition factor probed; an engine still within
        /// budget at `max_k` is reported saturated rather than swept
        /// further.
        #[arg(long, default_value_t = 1024)]
        max_k: u32,
        /// Report JSON path (with `--scene`).
        #[arg(long, conflicts_with = "out_dir", required_unless_present = "out_dir")]
        out: Option<PathBuf>,
        /// Output directory (with `--corpus`).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Pin the measurement to these CPUs (see `measure --cpu`);
        /// Linux and Android only.
        #[arg(long, value_name = "LIST")]
        cpu: Option<String>,
        /// Probe at a `WxH` native surface size rather than the scene's
        /// own (see `measure --native`).
        #[arg(long, value_name = "WxH")]
        native: Option<String>,
    },
    /// Time the presentation pass alone — one `Presenter::texture_timed`
    /// call per frame into an offscreen texture of the `--present`
    /// kind's format, bracketed by pass-boundary GPU timestamps (#96).
    /// Requires the `cherenkov` feature. On the M1 and iPad this is the
    /// gamut-map and encode cost evidence; on a shared VM it is a
    /// sanity check only.
    PresentCost {
        /// Destination size, `WxH` — the iPad-class 2752x2064 by default.
        #[arg(long, default_value = "2752x2064", value_name = "WxH")]
        size: String,
        /// Measured frames (after warmup).
        #[arg(long, default_value_t = 60)]
        frames: u32,
        /// Warmup frames (pipeline creation included) discarded.
        #[arg(long, default_value_t = 5)]
        warmup: u32,
        /// Source pattern: `oog` maps every pixel, `mixed` is half
        /// in-gamut.
        #[arg(long, value_enum, default_value = "oog")]
        pattern: PresentPattern,
        /// Presentation output kind timed — the same kinds as
        /// `render --present` (#98 added the wide-gamut and HDR kinds).
        #[arg(long, value_enum, default_value = "srgb-hw", alias = "encode")]
        present: crate::PresentKind,
        /// Display headroom presented to (#97). Above 1 exercises the
        /// tone-map shoulder on the `oog` pattern's HDR channels.
        #[arg(long, default_value_t = 4.0)]
        headroom: f32,
        /// Report JSON path.
        #[arg(long)]
        out: PathBuf,
    },
    /// Measure the external-frame hand-off (#168): path `e` composites
    /// a produced platform video buffer in place; path `c` copies its
    /// planes into engine textures and converts on the GPU — the
    /// video-gpu model. Requires the `cherenkov` feature and a platform
    /// buffer API (Apple or Android).
    ExternalCost {
        /// `e` external-frame import, `c` copy-and-convert.
        #[arg(long, value_enum)]
        path: ExternalPath,
        /// Frame size.
        #[arg(long, value_enum)]
        size: ExternalSize,
        /// SDR BT.709 (NV12) or HDR BT.2020 PQ (P010).
        #[arg(long, value_enum)]
        transfer: ExternalTransfer,
        /// Measured frames (after warmup).
        #[arg(long, default_value_t = 60)]
        frames: u32,
        /// Warmup frames discarded before measuring.
        #[arg(long, default_value_t = 30)]
        warmup: u32,
        /// Pace the measured frames to this rate in Hz — the 120 Hz
        /// pacing `measure` and `present-cost` share.
        #[arg(long, value_name = "HZ", default_value_t = 120.0)]
        rate: f64,
        /// Measure energy over the measured window (see
        /// `measure --energy`).
        #[arg(long)]
        energy: bool,
        /// Pin the run to these CPUs (see `measure --cpu`); Linux and
        /// Android only.
        #[arg(long, value_name = "LIST")]
        cpu: Option<String>,
        /// Report JSON path.
        #[arg(long)]
        out: PathBuf,
    },
    /// Sweep the P3 gamut boundary and report each candidate map's `ΔE_OK`
    /// and hue shift against the CSS Color 4 reference (#96). No engine —
    /// pure oracle `f64` math.
    GamutSweep {
        /// Report text path; stdout when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Sweep the presentation tone-map candidates over an HDR ramp at
    /// headrooms 1/2/4/8 and report monotonicity, `C1` knee continuity,
    /// plateau onset and hue preservation (#97). No engine — pure oracle
    /// `f64` math.
    ToneSweep {
        /// Report text path; stdout when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Snapshot memory at each engine-creation boundary — the issue-#170
    /// B1 creation ledger. Requires the `cherenkov` feature.
    ///
    /// Rows run process baseline → every creation phase → first surface,
    /// first submission and first completed frame → teardown; `--cycles`
    /// repeats create/drop inside one process, and `--stop-after PHASE`
    /// aborts creation just past a boundary for one-factor ablation.
    Creation {
        /// Report JSON path.
        #[arg(long)]
        out: PathBuf,
        /// Create + first frame + drop cycles inside one process.
        #[arg(long, default_value_t = 1)]
        cycles: u32,
        /// Abort creation just past this phase: `instance`, `adapter`,
        /// `device`, `layouts`, `shader-modules`, `core-pipelines`,
        /// `buffers`, `atlas`, `bind-groups`, `timestamps`,
        /// `shadow-blur`, `external-native`, `complete`.
        #[arg(long, value_name = "PHASE")]
        stop_after: Option<String>,
    },
    /// Render projective scenes in the oracle with the specified model and
    /// with two successively refined quality references, and report the
    /// model's distance from the finest reference and the references'
    /// convergence (#84). No engine — pure oracle `f64` math.
    ProjectiveQuality {
        /// Scene directories.
        #[arg(long = "scene", required = true)]
        scenes: Vec<PathBuf>,
        /// Report text path; stdout when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// List compiled-in adapter keys.
    Engines,
}

/// Runs one `cherenkov-bench` invocation — the single code path behind
/// both the `cherenkov-bench` binary and the C entry point the iOS
/// host calls (`cherenkov_bench_run`).
///
/// `args` is the full argv, program name first. Returns the
/// process-style exit code: 0 on success, 1 on a run failure, or the
/// `clap` code (0 for `--help`/`--version`, 2 for a parse error).
#[must_use]
pub fn run_args(args: &[OsString]) -> i32 {
    // The iOS host calls `cherenkov_bench_run` once per argument list
    // inside one process, so a later call finds the global subscriber
    // already installed; `try_init` keeps re-entry clean. Diagnostics
    // go to stderr (the host captures it per run into `run-<n>.log`).
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return e.exit_code();
        }
    };
    let code = match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    };
    // The host keeps this process alive for the next argument list and
    // may redirect our fds per call — leave no partial line buffered.
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    code
}

/// C entry point for hosts that cannot spawn a process.
///
/// The iOS bench app links the `cherenkov-bench` static library and
/// calls this once per argument list; it runs the same [`run_args`] the
/// binary's `main` does.
///
/// Returns the process-style exit code (0 on success, 101 on panic —
/// unwinding cannot cross `extern "C"`, so a panicking run would
/// otherwise abort the host and every queued run with it).
///
/// # Panics
/// `argc` is a count, so a negative value — outside the
/// `main(argc, argv)` contract — panics.
///
/// # Safety
/// `argv` must point to `argc` non-null pointers, each to a valid
/// NUL-terminated C string — the `main(argc, argv)` contract.
#[cfg(unix)]
#[expect(clippy::similar_names, reason = "the argc/argv C contract")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cherenkov_bench_run(argc: c_int, argv: *const *const c_char) -> c_int {
    use std::os::unix::ffi::OsStrExt as _;
    let argc = usize::try_from(argc).expect("argc is non-negative");
    // SAFETY: the caller guarantees `argv` points to `argc` pointers,
    // each to a NUL-terminated C string that outlives this call.
    let args: Vec<OsString> = unsafe {
        std::slice::from_raw_parts(argv, argc)
            .iter()
            .map(|&arg| OsStr::from_bytes(CStr::from_ptr(arg).to_bytes()).to_os_string())
            .collect()
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_args(&args))).unwrap_or(101)
}

#[expect(
    clippy::too_many_lines,
    reason = "one match arm per subcommand; the run dispatch table"
)]
fn run(cli: Cli) -> Result<(), BenchError> {
    match cli.cmd {
        Sub::Engines => {
            for e in engine_names() {
                tracing::info!(engine = e, "adapter");
            }
            Ok(())
        }
        Sub::Render {
            engine,
            scene,
            corpus,
            out,
            out_dir,
            present,
            reference,
        } => render_cmd(
            &engine,
            scene.as_deref(),
            corpus.as_deref(),
            out.as_deref(),
            out_dir.as_deref(),
            present,
            reference.as_deref(),
        ),
        Sub::Reference {
            scene,
            corpus,
            out_dir,
        } => reference_cmd(scene.as_deref(), corpus.as_deref(), &out_dir),
        Sub::Measure {
            engine,
            scene,
            corpus,
            frames,
            warmup,
            pause_at,
            out,
            out_dir,
            cpu,
            rate,
            energy,
            native,
        } => measure_cmd(
            &engine,
            MeasureOpts {
                scene: scene.as_deref(),
                corpus: corpus.as_deref(),
                frames,
                warmup,
                pause_at,
                out: out.as_deref(),
                out_dir: out_dir.as_deref(),
                cpu: cpu.as_deref(),
                rate,
                energy,
                native: native.as_deref(),
            },
        ),
        Sub::AllocDiag {
            scene,
            frames,
            warmup,
            out,
            native,
            rate,
            cpu,
        } => alloc_diag_cmd(
            &scene,
            frames,
            warmup,
            native.as_deref(),
            rate,
            cpu.as_deref(),
            &out,
        ),
        Sub::Capacity {
            engine,
            scene,
            corpus,
            frames,
            warmup,
            budget_ms,
            max_k,
            out,
            out_dir,
            cpu,
            native,
        } => capacity_cmd(
            &engine,
            CapacityOpts {
                scene: scene.as_deref(),
                corpus: corpus.as_deref(),
                frames,
                warmup,
                budget_ms,
                max_k,
                out: out.as_deref(),
                out_dir: out_dir.as_deref(),
                cpu: cpu.as_deref(),
                native: native.as_deref(),
            },
        ),
        Sub::PresentCost {
            size,
            frames,
            warmup,
            pattern,
            present,
            headroom,
            out,
        } => present_cost_cmd(
            size.as_str(),
            frames,
            warmup,
            pattern,
            present,
            headroom,
            &out,
        ),
        Sub::ExternalCost {
            path,
            size,
            transfer,
            frames,
            warmup,
            rate,
            energy,
            cpu,
            out,
        } => external_cost_cmd(
            path,
            size,
            transfer,
            frames,
            warmup,
            rate,
            energy,
            cpu.as_deref(),
            &out,
        ),
        Sub::GamutSweep { out } => crate::gamut_sweep::run(out.as_deref()),
        Sub::ToneSweep { out } => crate::tone_sweep::run(out.as_deref()),
        Sub::Creation {
            out,
            cycles,
            stop_after,
        } => creation_cmd(&out, cycles, stop_after.as_deref()),
        Sub::ProjectiveQuality { scenes, out } => {
            crate::projective_quality::run(&scenes, out.as_deref())
        }
    }
}

/// The `creation` subcommand, gated on the `cherenkov` adapter feature.
#[cfg(feature = "cherenkov")]
fn creation_cmd(out: &Path, cycles: u32, stop_after: Option<&str>) -> Result<(), BenchError> {
    let phase = stop_after
        .map(|name| {
            crate::creation::parse_phase(name)
                .ok_or_else(|| BenchError::Engine(format!("unknown creation phase {name:?}")))
        })
        .transpose()?;
    crate::creation::run(cycles, phase, out)
}

#[cfg(not(feature = "cherenkov"))]
fn creation_cmd(_out: &Path, _cycles: u32, _stop_after: Option<&str>) -> Result<(), BenchError> {
    Err(BenchError::Engine(
        "creation needs the `cherenkov` adapter feature".into(),
    ))
}

/// `present-cost` needs the GPU adapter's shared device and Presenter.
#[cfg(feature = "cherenkov")]
fn present_cost_cmd(
    size: &str,
    frames: u32,
    warmup: u32,
    pattern: PresentPattern,
    present: crate::PresentKind,
    headroom: f32,
    out: &Path,
) -> Result<(), BenchError> {
    let size = parse_native(Some(size))?.expect("size is required");
    crate::present_cost::run(size, frames, warmup, pattern, present, headroom, out)
}

#[cfg(not(feature = "cherenkov"))]
fn present_cost_cmd(
    _size: &str,
    _frames: u32,
    _warmup: u32,
    _pattern: PresentPattern,
    _present: crate::PresentKind,
    _headroom: f32,
    _out: &Path,
) -> Result<(), BenchError> {
    Err(BenchError::Engine(
        "present-cost needs the `cherenkov` adapter feature".into(),
    ))
}

/// `external-cost` needs the GPU adapter and platform interop (#168).
#[cfg(feature = "cherenkov")]
#[expect(
    clippy::too_many_arguments,
    reason = "the stub below takes the same fields; ExternalCostArgs exists only with this feature"
)]
fn external_cost_cmd(
    path: ExternalPath,
    size: ExternalSize,
    transfer: ExternalTransfer,
    frames: u32,
    warmup: u32,
    rate: f64,
    energy: bool,
    cpu: Option<&str>,
    out: &Path,
) -> Result<(), BenchError> {
    crate::external_cost::run(&ExternalCostArgs {
        path,
        size,
        transfer,
        frames,
        warmup,
        rate,
        energy,
        cpu: cpu.map(affinity::parse_cpu_list).transpose()?,
        out: out.to_path_buf(),
    })
}

#[cfg(not(feature = "cherenkov"))]
#[expect(
    clippy::too_many_arguments,
    reason = "matches the cherenkov command; ExternalCostArgs is not built without that feature"
)]
fn external_cost_cmd(
    _path: ExternalPath,
    _size: ExternalSize,
    _transfer: ExternalTransfer,
    _frames: u32,
    _warmup: u32,
    _rate: f64,
    _energy: bool,
    _cpu: Option<&str>,
    _out: &Path,
) -> Result<(), BenchError> {
    Err(BenchError::Engine(
        "external-cost needs the `cherenkov` adapter feature".into(),
    ))
}

/// The `alloc-diag` subcommand: a fresh-process allocation event trace
/// for issue #169, at the issue's resolution and pacing.
#[cfg(feature = "cherenkov")]
fn alloc_diag_cmd(
    dir: &Path,
    frames: u32,
    warmup: u32,
    native: Option<&str>,
    rate: Option<f64>,
    cpu: Option<&str>,
    out: &Path,
) -> Result<(), BenchError> {
    let native = parse_native(native)?;
    if let Some(rate) = rate {
        if !(rate.is_finite() && rate > 0.0) {
            return Err(BenchError::Engine(format!(
                "--rate must be a positive, finite Hz value, got {rate}"
            )));
        }
        pacing(rate, frames)?;
    }
    if let Some(cpus) = cpu.map(affinity::parse_cpu_list).transpose()? {
        affinity::pin_current_thread(&cpus)?;
    }
    let sink = cherenkov_gpu::diag::Sink::new();
    let mut engine = crate::cherenkov_ad::Cherenkov::with_alloc_diag(sink.clone())?;
    let (scene, _native) = load_scene(dir, native)?;
    let blobs = convert::load_blobs(&scene, dir)?;
    let input = EncodeInput {
        scene: &scene,
        blobs: &blobs,
    };
    engine.prepare(&input)?;
    let (period, window_hint) = match rate {
        Some(hz) => pacing(hz, frames).map(|(p, w)| (Some(p), w))?,
        None => (None, Duration::from_secs(30)),
    };
    run_frames(
        &mut engine,
        &input,
        frames,
        warmup,
        FrameLoopOptions {
            pause_at: None,
            period,
            window_hint,
            measure_energy: false,
        },
    )?;
    engine.alloc_diag_teardown();
    drop(engine);
    // #169 A5: the allocation-event high-water observation. Frame-boundary
    // snapshots can miss the event that caused a permanently retained
    // block, so this scans the per-event snapshots, not the lifecycle.
    let events = sink.take();
    let count = cherenkov_gpu::diag::write_events(&events, out)
        .map_err(|e| BenchError::Engine(format!("writing {}: {e}", out.display())))?;
    let mut allocated = 0u64;
    let mut reserved = 0u64;
    let mut blocks = 0u64;
    let mut retired_in_flight = 0u64;
    let mut staging = 0u64;
    let mut first_two_block_seq = None;
    for event in &events {
        allocated = allocated.max(event.alloc.allocated);
        reserved = reserved.max(event.alloc.reserved);
        blocks = blocks.max(event.alloc.blocks);
        retired_in_flight = retired_in_flight.max(event.retired_in_flight);
        staging = staging.max(event.alloc.staging.1);
        if event.alloc.blocks >= 2 && first_two_block_seq.is_none() {
            first_two_block_seq = Some(event.seq);
        }
    }
    let summary = serde_json::json!({
        "high_water": {
            "allocated_bytes": allocated,
            "reserved_bytes": reserved,
            "blocks": blocks,
            "retired_in_flight_bytes": retired_in_flight,
            "staging_bytes": staging,
            "first_two_block_seq": first_two_block_seq,
            "events": events.len(),
        }
    });
    {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(out)
            .map_err(|e| BenchError::Engine(format!("appending to {}: {e}", out.display())))?;
        writeln!(file, "{summary}")
            .map_err(|e| BenchError::Engine(format!("writing {}: {e}", out.display())))?;
    }
    println!("alloc-diag high water: {summary}");
    tracing::info!(
        events = count,
        out = %out.display(),
        "alloc-diag"
    );
    Ok(())
}

#[cfg(not(feature = "cherenkov"))]
fn alloc_diag_cmd(
    _scene: &Path,
    _frames: u32,
    _warmup: u32,
    _native: Option<&str>,
    _rate: Option<f64>,
    _cpu: Option<&str>,
    _out: &Path,
) -> Result<(), BenchError> {
    Err(BenchError::Engine(
        "alloc-diag needs the `cherenkov` adapter feature".into(),
    ))
}

/// The `measure` subcommand's fields, borrowed to avoid cloning paths.
#[derive(Clone, Copy)]
struct MeasureOpts<'a> {
    scene: Option<&'a Path>,
    corpus: Option<&'a Path>,
    frames: u32,
    warmup: u32,
    pause_at: Option<u32>,
    out: Option<&'a Path>,
    out_dir: Option<&'a Path>,
    cpu: Option<&'a str>,
    rate: Option<f64>,
    energy: bool,
    native: Option<&'a str>,
}

/// Wall-clock seconds a `render` scene (or a whole pass) spent in each
/// phase. `render` and `readback` split `Engine::submit` using the
/// adapter-reported halves; an adapter that does not split reports its
/// whole submit under `render`.
#[derive(Clone, Copy, Debug, Default)]
struct PhaseTiming {
    /// `Scene::load` plus the blob reads.
    load: f64,
    /// The oracle `f64` render and, under `--present`, the reference's
    /// presentation of it.
    reference: f64,
    /// `Engine::prepare`.
    prepare: f64,
    /// `Engine::encode`.
    encode: f64,
    /// `Engine::submit` minus its readback half.
    render: f64,
    /// `Engine::submit`'s pixel readback.
    readback: f64,
    /// `Engine::trim`, the memory snapshots and the counters read.
    trim: f64,
    /// `metrics::compare` (FLIP, max local error, heatmap).
    compare: f64,
    /// `write_render` / `write_unsupported` (PNGs plus the report JSON).
    write: f64,
}

impl std::ops::AddAssign for PhaseTiming {
    fn add_assign(&mut self, rhs: Self) {
        self.load += rhs.load;
        self.reference += rhs.reference;
        self.prepare += rhs.prepare;
        self.encode += rhs.encode;
        self.render += rhs.render;
        self.readback += rhs.readback;
        self.trim += rhs.trim;
        self.compare += rhs.compare;
        self.write += rhs.write;
    }
}

/// The `render` subcommand: every scene in the corpus, or the one
/// `--scene`, against the oracle.
fn render_cmd(
    engine: &str,
    scene: Option<&Path>,
    corpus: Option<&Path>,
    out: Option<&Path>,
    out_dir: Option<&Path>,
    present: Option<crate::PresentKind>,
    reference: Option<&Path>,
) -> Result<(), BenchError> {
    let pass_at = Instant::now();
    let mut engine = create_engine(engine)?;
    let idle_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    if let Some(kind) = present {
        engine.present(kind)?;
    }
    let setup_seconds = pass_at.elapsed().as_secs_f64();
    tracing::info!(
        engine = engine.info().name,
        setup_s = setup_seconds,
        "adapter ready"
    );
    let mut totals = PhaseTiming::default();
    let mut scenes = 0u32;
    for dir in scene_dirs(scene, corpus)? {
        scenes += 1;
        let out_path = match (out, out_dir) {
            (Some(o), None) => o.to_path_buf(),
            (None, Some(d)) => d.join(render_report_name(engine.info().name, present, &dir)),
            _ => return Err(BenchError::Engine("--out or --out-dir required".into())),
        };
        let mut t = PhaseTiming::default();
        match render_scene(
            &mut *engine,
            &dir,
            idle_memory.clone(),
            present,
            reference,
            &mut t,
        ) {
            Ok(rendered) => {
                let write_at = Instant::now();
                write_render(&rendered, &out_path)?;
                t.write = write_at.elapsed().as_secs_f64();
                totals += t;
                tracing::info!(
                    scene = %dir.display(),
                    flip_mean = rendered.report.metrics.flip_mean,
                    flip_max = rendered.report.metrics.flip_max,
                    max_local_error = rendered.report.metrics.max_local_error,
                    load_s = t.load,
                    reference_s = t.reference,
                    prepare_s = t.prepare,
                    encode_s = t.encode,
                    render_s = t.render,
                    readback_s = t.readback,
                    compare_s = t.compare,
                    write_s = t.write,
                    out = %out_path.display(),
                    "render"
                );
            }
            Err(BenchError::Unsupported { feature, api, .. }) => {
                let write_at = Instant::now();
                write_unsupported(
                    &*engine,
                    &dir,
                    feature.clone(),
                    api,
                    idle_memory.clone(),
                    &out_path,
                )?;
                t.write = write_at.elapsed().as_secs_f64();
                totals += t;
                tracing::warn!(
                    scene = %dir.display(),
                    ?feature,
                    load_s = t.load,
                    reference_s = t.reference,
                    prepare_s = t.prepare,
                    out = %out_path.display(),
                    "unsupported"
                );
            }
            Err(e) => return Err(e),
        }
    }
    tracing::info!(
        engine = engine.info().name,
        scenes,
        wall_s = pass_at.elapsed().as_secs_f64(),
        load_s = totals.load,
        reference_s = totals.reference,
        prepare_s = totals.prepare,
        encode_s = totals.encode,
        render_s = totals.render,
        readback_s = totals.readback,
        trim_s = totals.trim,
        compare_s = totals.compare,
        write_s = totals.write,
        "render pass totals"
    );
    Ok(())
}

/// The `reference` subcommand: every scene's oracle `f64` image into
/// `<scene>.ref` files under `--out-dir`, in parallel across the
/// available cores — the per-scene oracle render is independent.
fn reference_cmd(
    scene: Option<&Path>,
    corpus: Option<&Path>,
    out_dir: &Path,
) -> Result<(), BenchError> {
    let dirs = scene_dirs(scene, corpus)?;
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let pass_at = Instant::now();
    let next = AtomicUsize::new(0);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let outcome = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..threads.min(dirs.len()) {
            handles.push(s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(dir) = dirs.get(i) else {
                        return Ok(());
                    };
                    if let Err(e) = reference_scene(dir, out_dir) {
                        stop.store(true, Ordering::Relaxed);
                        return Err(e);
                    }
                }
                Ok(())
            }));
        }
        let mut first_err = None;
        for h in handles {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        first_err
    });
    if let Some(e) = outcome {
        return Err(e);
    }
    tracing::info!(
        scenes = dirs.len(),
        threads,
        wall_s = pass_at.elapsed().as_secs_f64(),
        "reference pass totals"
    );
    Ok(())
}

/// One scene's oracle `f64` render into `out_dir`'s `<scene>.ref`.
fn reference_scene(dir: &Path, out_dir: &Path) -> Result<(), BenchError> {
    let at = Instant::now();
    let scene = Scene::load(dir)?;
    let blobs = convert::load_blobs(&scene, dir)?;
    let image =
        Renderer::new(scene.width as usize, scene.height as usize).render_image(&scene, dir)?;
    let scene_json = std::fs::read(dir.join("scene.json"))?;
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    refcache::write(
        out_dir,
        &name,
        refcache::fingerprint(&scene_json, &blobs),
        &image,
    )?;
    tracing::info!(
        scene = %dir.display(),
        reference_s = at.elapsed().as_secs_f64(),
        "reference"
    );
    Ok(())
}

/// A `--native` value: the `WxH` surface a device host's window would
/// report, given on the command line for an offscreen run.
fn parse_native(arg: Option<&str>) -> Result<Option<(u32, u32)>, BenchError> {
    let Some(arg) = arg else { return Ok(None) };
    let Some((w, h)) = arg.split_once(['x', 'X']) else {
        return Err(BenchError::Engine(format!(
            "--native wants a WxH size like 2752x2064, got {arg:?}"
        )));
    };
    let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else {
        return Err(BenchError::Engine(format!(
            "--native wants a WxH size like 2752x2064, got {arg:?}"
        )));
    };
    if w == 0 || h == 0 {
        return Err(BenchError::Engine(format!(
            "--native wants a non-zero WxH size, got {arg:?}"
        )));
    }
    Ok(Some((w, h)))
}

fn measure_cmd(engine: &str, opts: MeasureOpts<'_>) -> Result<(), BenchError> {
    if let Some(rate) = opts.rate {
        if !(rate.is_finite() && rate > 0.0) {
            return Err(BenchError::Engine(format!(
                "--rate must be a positive, finite Hz value, got {rate}"
            )));
        }
        pacing(rate, opts.frames)?;
    }
    if opts.energy {
        if opts.frames == 0 {
            return Err(BenchError::Engine(
                "--energy requires at least one measured frame (--frames > 0)".into(),
            ));
        }
        // Mandatory: fail here, before the engine and scenes, rather
        // than writing reports without energy.
        energy::Meter::probe()?;
        if opts.rate.is_none() {
            tracing::warn!(
                "--energy without --rate: energy per frame is only comparable at a fixed rate"
            );
        }
    }
    let native = parse_native(opts.native)?;
    let pinned = opts.cpu.map(affinity::parse_cpu_list).transpose()?;
    if let Some(cpus) = &pinned {
        // Pin before the adapter is created so every thread it spawns
        // inherits the mask.
        affinity::pin_current_thread(cpus)?;
    }
    let mut engine = create_engine(engine)?;
    let idle_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    for dir in scene_dirs(opts.scene, opts.corpus)? {
        let out_path = match (opts.out, opts.out_dir) {
            (Some(o), None) => o.to_path_buf(),
            (None, Some(d)) => d.join(format!(
                "measure-{}-{}.json",
                engine.info().name,
                dir.file_name().unwrap_or_default().to_string_lossy()
            )),
            _ => return Err(BenchError::Engine("--out or --out-dir required".into())),
        };
        match measure_scene(
            &mut *engine,
            &dir,
            MeasureSceneOptions {
                frames: opts.frames,
                warmup: opts.warmup,
                pause_at: opts.pause_at,
                pinned: pinned.as_deref(),
                rate: opts.rate,
                measure_energy: opts.energy,
                idle_memory: idle_memory.clone(),
                native,
            },
        ) {
            Ok(report) => {
                write_json(&report, &out_path)?;
                log_measure_details(&dir, &report, &out_path);
            }
            Err(BenchError::Unsupported { feature, api, .. }) => {
                write_unsupported(
                    &*engine,
                    &dir,
                    feature.clone(),
                    api,
                    idle_memory.clone(),
                    &out_path,
                )?;
                tracing::warn!(
                    scene = %dir.display(),
                    ?feature,
                    out = %out_path.display(),
                    "unsupported"
                );
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn log_measure_details(dir: &Path, report: &MeasureReport, out_path: &Path) {
    tracing::info!(
        scene = %dir.display(),
        prepare_s = report.prepare_seconds,
        encode_p50 = report.percentiles.encode_seconds[0],
        submit_p50 = report.percentiles.submit_seconds[0],
        gpu_p50 = ?report.percentiles.gpu_seconds.map(|g| g[0]),
        engine_memory = ?report.memory.steady.engine,
        out = %out_path.display(),
        "measure"
    );
    for pass in &report.percentiles.passes {
        tracing::info!(
            scene = %dir.display(),
            pass = %pass.name,
            size = %format!("{}x{}", pass.width, pass.height),
            format = %pass.format,
            gpu_p50 = ?pass.gpu_seconds.map(|g| g[0]),
            "pass"
        );
    }
    for phase in &report.percentiles.phases {
        tracing::info!(
            scene = %dir.display(),
            phase = %phase.name,
            p50 = phase.seconds[0],
            p99 = phase.seconds[2],
            "phase"
        );
    }
}

/// `--scene` gives one dir; `--corpus` gives every child holding
/// `scene.json`, sorted by name.
fn scene_dirs(scene: Option<&Path>, corpus: Option<&Path>) -> Result<Vec<PathBuf>, BenchError> {
    if let Some(d) = scene {
        return Ok(vec![d.to_path_buf()]);
    }
    let corpus = corpus.ok_or_else(|| BenchError::Engine("no scene or corpus".into()))?;
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(corpus)? {
        let dir = entry?.path();
        if dir.is_dir() && dir.join("scene.json").is_file() {
            dirs.push(dir);
        }
    }
    dirs.sort();
    if dirs.is_empty() {
        return Err(BenchError::Engine(format!(
            "no scenes under {}",
            corpus.display()
        )));
    }
    Ok(dirs)
}

/// The per-scene `render` report file name; `present` kinds carry the kind
/// in the name so they do not collide with the normal corpus.
fn render_report_name(engine: &str, present: Option<crate::PresentKind>, dir: &Path) -> String {
    let scene = dir.file_name().unwrap_or_default().to_string_lossy();
    let kind = present.map_or_else(String::new, |k| format!("{}-", k.name()));
    format!("render-{engine}-{kind}{scene}.json")
}

/// Rendered image + heatmap, written next to the metrics JSON.
struct RenderOutput {
    /// The metrics report.
    report: RenderReport,
    /// Engine image.
    image: F32Image,
    /// Error heatmap, RGB8.
    heatmap: Vec<u8>,
}

/// The oracle's f64 image through the matching presentation function,
/// quantized to the destination's storage and lifted back into the
/// working space where both sides of the comparison live.
fn present_image(
    kind: crate::PresentKind,
    headroom: f64,
    image: &cherenkov_oracle::Image,
) -> cherenkov_oracle::F32Image {
    use crate::PresentKind as K;
    let presented = match kind {
        K::LinearP3 => cherenkov_oracle::present::present_linear_p3(headroom, image),
        K::SrgbHw | K::SrgbShader => cherenkov_oracle::present::present_srgb(headroom, image),
        K::DisplayP3Hw | K::DisplayP3Shader => {
            cherenkov_oracle::present::present_display_p3(headroom, image)
        }
        K::Scrgb => cherenkov_oracle::present::present_extended_srgb_linear(headroom, image),
        K::ExtendedSrgb => cherenkov_oracle::present::present_extended_srgb(headroom, image),
        K::ExtendedP3 => cherenkov_oracle::present::present_extended_display_p3(headroom, image),
        K::Pq => cherenkov_oracle::present::present_pq(headroom, image),
        K::Hlg => cherenkov_oracle::present::present_hlg(headroom, image),
    };
    // The ideal presented image is what the destination stores: the
    // unorm-8 kinds quantize the encoded channels so the metric measures
    // the pass, not the format's floor.
    let lift: fn([f64; 4]) -> [f64; 4] = match kind {
        K::LinearP3 => |p| p,
        K::SrgbHw | K::SrgbShader => cherenkov_oracle::present::presented_srgb_to_working,
        K::DisplayP3Hw | K::DisplayP3Shader => {
            cherenkov_oracle::present::presented_display_p3_to_working
        }
        K::Scrgb => |p| cherenkov_oracle::present::presented_extended_srgb_to_working(false, p),
        K::ExtendedSrgb => {
            |p| cherenkov_oracle::present::presented_extended_srgb_to_working(true, p)
        }
        K::ExtendedP3 => cherenkov_oracle::present::presented_extended_p3_to_working,
        K::Pq => cherenkov_oracle::present::presented_pq_to_working,
        K::Hlg => cherenkov_oracle::present::presented_hlg_to_working,
    };
    let unorm8 = matches!(
        kind,
        K::SrgbHw | K::SrgbShader | K::DisplayP3Hw | K::DisplayP3Shader
    );
    let stored = if unorm8 {
        cherenkov_oracle::present::quantize_unorm8(&presented)
    } else {
        presented
    };
    let working = cherenkov_oracle::Image {
        width: stored.width,
        height: stored.height,
        pixels: stored.pixels.iter().map(|&p| lift(p)).collect(),
    };
    cherenkov_oracle::F32Image::from_f64(&working)
}

fn render_scene(
    engine: &mut dyn Engine,
    dir: &Path,
    idle_memory: MemorySnapshot,
    present: Option<crate::PresentKind>,
    reference_dir: Option<&Path>,
    t: &mut PhaseTiming,
) -> Result<RenderOutput, BenchError> {
    let at = Instant::now();
    let scene = Scene::load(dir)?;
    let blobs = convert::load_blobs(&scene, dir)?;
    t.load = at.elapsed().as_secs_f64();
    let at = Instant::now();
    let image = match reference_dir {
        Some(cache) => refcache::read(
            cache,
            &dir.file_name().unwrap_or_default().to_string_lossy(),
            refcache::fingerprint(&std::fs::read(dir.join("scene.json"))?, &blobs),
            (scene.width as usize, scene.height as usize),
        )?,
        None => {
            Renderer::new(scene.width as usize, scene.height as usize).render_image(&scene, dir)?
        }
    };
    let reference = match present {
        None => F32Image::from_f64(&image),
        Some(kind) => present_image(kind, scene.present_headroom, &image),
    };
    t.reference = at.elapsed().as_secs_f64();
    let input = EncodeInput {
        scene: &scene,
        blobs: &blobs,
    };
    let at = Instant::now();
    engine.prepare(&input)?;
    t.prepare = at.elapsed().as_secs_f64();
    let prepare_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    let at = Instant::now();
    engine.encode(&input)?;
    t.encode = at.elapsed().as_secs_f64();
    let submit_at = Instant::now();
    let submit = engine.submit(0, true)?;
    let submit_seconds = submit_at.elapsed().as_secs_f64();
    t.readback = submit.readback_seconds.unwrap_or(0.0);
    t.render = submit.render_seconds.unwrap_or(submit_seconds - t.readback);
    let at = Instant::now();
    let steady_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    // Counters embed a memory snapshot; take them at steady state,
    // before retirement shrinks what the engine reports.
    let counters = engine.counters();
    // #169 A5: the post-retirement observation follows the engine's
    // explicit retirement pass, after the window and its submission.
    engine.trim()?;
    let post_retire_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    t.trim = at.elapsed().as_secs_f64();
    let test = submit
        .image
        .ok_or_else(|| BenchError::Engine("adapter returned no image".into()))?;
    let at = Instant::now();
    let (metrics_v, heatmap) = metrics::compare(&reference, &test);
    t.compare = at.elapsed().as_secs_f64();
    Ok(RenderOutput {
        report: RenderReport {
            engine: engine.info().name,
            info: engine.info().clone(),
            scene: dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            width: scene.width,
            height: scene.height,
            present: present.map(|kind| crate::report::PresentInfo {
                kind: kind.name(),
                headroom: scene.present_headroom,
            }),
            metrics: metrics_v,
            counters,
            memory: MemoryReport::new(
                idle_memory,
                &[prepare_memory, steady_memory],
                Some(post_retire_memory),
            ),
            device: engine.device(),
        },
        image: test,
        heatmap,
    })
}

/// Per-CPU use counts over the samples plus the prepare call.
fn cpu_use(prepare_cpu: Option<u32>, samples: &[FrameSample]) -> BTreeMap<u32, CpuUse> {
    let mut cpus = BTreeMap::<u32, CpuUse>::new();
    let mut count = |cpu| {
        if let Some(cpu) = cpu {
            cpus.entry(cpu)
                .or_insert_with(|| CpuUse {
                    count: 0,
                    max_freq_khz: affinity::cpu_max_freq_khz(cpu),
                })
                .count += 1;
        }
    };
    count(prepare_cpu);
    for sample in samples {
        count(sample.cpu_start);
        count(sample.cpu_end);
    }
    cpus
}

/// What [`run_frames`] collected.
struct Window {
    samples: Vec<FrameSample>,
    /// Warmup-frame captures only — measured frames sample nothing.
    memory_samples: Vec<MemorySnapshot>,
    meter: Option<energy::Meter>,
    start: Instant,
    missed_deadlines: u32,
}

/// How far past a deadline a frame may start before it counts as
/// missed — scheduler overshoot under a millisecond is noise.
const PACING_TOLERANCE: Duration = Duration::from_millis(1);

#[derive(Clone, Copy)]
struct FrameLoopOptions {
    pause_at: Option<u32>,
    period: Option<Duration>,
    window_hint: Duration,
    measure_energy: bool,
}

/// The warmup + measured frame loop.
///
/// With `--rate` each measured frame starts on `start + n / rate` —
/// the one sleep in the bench is frame pacing — and the energy meter
/// wraps exactly the measured window. Memory is sampled after warmup
/// frames only: the measured window holds pacing, encode, submit and
/// GPU-timing attribution and nothing else — on Android a capture
/// walks `/proc/self/smaps_rollup` and a full snapshot spawns `dumpsys
/// gpu --gpumem`, which would tax the pacing and energy it brackets (#162).
fn run_frames(
    engine: &mut dyn Engine,
    input: &EncodeInput<'_>,
    frames: u32,
    warmup: u32,
    options: FrameLoopOptions,
) -> Result<Window, BenchError> {
    let FrameLoopOptions {
        pause_at,
        period,
        window_hint,
        measure_energy,
    } = options;
    let mut meter = None;
    let mut start = Instant::now();
    let mut missed_deadlines = 0u32;
    let mut samples = Vec::with_capacity(frames as usize);
    let total_frames = warmup + frames;
    let mut memory_samples = Vec::with_capacity(warmup as usize);
    for frame in 0..total_frames {
        if frame == warmup {
            // The energy window and the pacing clock both open
            // immediately before the first measured frame.
            if measure_energy {
                meter = Some(energy::Meter::begin(window_hint)?);
            }
            start = Instant::now();
        }
        if frame >= warmup
            && let Some(period) = period
            && let Some(deadline) = period
                .checked_mul(frame - warmup)
                .and_then(|d| start.checked_add(d))
        {
            let now = Instant::now();
            if now < deadline {
                std::thread::sleep(deadline - now);
                // A wake-up past the tolerance still started the frame
                // late — a missed deadline all the same.
                if Instant::now().saturating_duration_since(deadline) > PACING_TOLERANCE {
                    missed_deadlines += 1;
                }
            } else if frame > warmup {
                missed_deadlines += 1;
            }
        }
        let cpu_start = affinity::current_cpu();
        let t0 = Instant::now();
        if pause_at == Some(frame) {
            pause_before_frame(frame)?;
        }
        engine.encode(input)?;
        let t1 = Instant::now();
        let submit = engine.submit(u64::from(frame), false)?;
        let t2 = Instant::now();
        let cpu_end = affinity::current_cpu();
        if frame < warmup {
            memory_samples.push(MemorySnapshot::capture(
                engine.memory(),
                SampleDetail::Frame,
            ));
        }
        if frame >= warmup {
            samples.push(FrameSample {
                encode_seconds: t1.duration_since(t0).as_secs_f64(),
                submit_seconds: t2.duration_since(t1).as_secs_f64(),
                gpu_seconds: None,
                cpu_start,
                cpu_end,
                migrated: matches!((cpu_start, cpu_end), (Some(a), Some(b)) if a != b),
                passes: Vec::new(),
                phases: submit.phases.map(phase_samples).unwrap_or_default(),
            });
        }
        attribute_gpu(&mut samples, warmup, submit.gpu);
    }
    attribute_gpu(&mut samples, warmup, engine.finish_gpu()?);
    Ok(Window {
        samples,
        memory_samples,
        meter,
        start,
        missed_deadlines,
    })
}

/// The render-thread phase timings of one frame as named samples.
fn phase_samples(phases: crate::Phases) -> Vec<crate::PhaseSample> {
    [
        ("lower", phases.lower),
        ("encode", phases.encode),
        ("stamp", phases.stamp),
        ("wait", phases.wait),
    ]
    .into_iter()
    .map(|(name, seconds)| crate::PhaseSample {
        name: name.to_string(),
        seconds,
    })
    .collect()
}

fn pause_before_frame(frame: u32) -> Result<(), BenchError> {
    let stdout = std::io::stdout();
    let mut stdout_lock = stdout.lock();
    writeln!(stdout_lock, "cherenkov-bench: paused before frame {frame}")?;
    stdout_lock.flush()?;
    drop(stdout_lock);
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut byte = [0];
    loop {
        match stdin.read(&mut byte)? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "stdin closed before resuming measure",
                )
                .into());
            }
            _ if byte[0] == b'\n' => break,
            _ => {}
        }
    }
    Ok(())
}

/// Stores each GPU timing on the measured sample of the frame it times;
/// warmup frames' timings are dropped.
fn attribute_gpu(samples: &mut [FrameSample], warmup: u32, gpu: Vec<crate::GpuSample>) {
    for timing in gpu {
        let Some(index) = timing.frame.checked_sub(u64::from(warmup)) else {
            continue;
        };
        let sample = &mut samples[usize::try_from(index).expect("a frame index fits usize")];
        sample.gpu_seconds = timing.gpu_seconds;
        sample.passes = timing.passes;
    }
}

/// `--rate` validation + derivation: the per-frame period must fit a
/// `Duration` (a rate near zero overflows it) and the paced window
/// the pacing clock, else deadlines would silently break mid-run.
/// Returns the period and the whole paced window.
fn pacing(rate: f64, frames: u32) -> Result<(Duration, Duration), BenchError> {
    let period = Duration::try_from_secs_f64(1.0 / rate).map_err(|_| {
        BenchError::Engine(format!(
            "--rate {rate} Hz gives an unrepresentable pacing period"
        ))
    })?;
    let window = period.checked_mul(frames).ok_or_else(|| {
        BenchError::Engine(format!(
            "--rate {rate} Hz over --frames {frames} overflows the pacing clock"
        ))
    })?;
    Ok((period, window))
}

struct MeasureSceneOptions<'a> {
    frames: u32,
    warmup: u32,
    pause_at: Option<u32>,
    pinned: Option<&'a [u32]>,
    rate: Option<f64>,
    measure_energy: bool,
    idle_memory: MemorySnapshot,
    native: Option<(u32, u32)>,
}

/// Loads `dir`'s scene, retargeted to `native` when given. Returns the
/// scene the engines see and the report's `native` record.
fn load_scene(
    dir: &Path,
    native: Option<(u32, u32)>,
) -> Result<(Scene, Option<NativeResolution>), BenchError> {
    let scene = Scene::load(dir)?;
    let Some((w, h)) = native else {
        return Ok((scene, None));
    };
    let (scene, scale) = transform::at_native(&scene, w, h);
    Ok((
        scene,
        Some(NativeResolution {
            width: w,
            height: h,
            scale,
        }),
    ))
}

/// The run's pacing summary, when a rate was requested: requested Hz,
/// achieved Hz over the measured window and missed deadlines.
fn pacing_report(
    requested_hz: f64,
    frames: u32,
    missed_deadlines: u32,
    window_seconds: f64,
) -> Pacing {
    Pacing {
        requested_hz,
        achieved_hz: if window_seconds > 0.0 {
            f64::from(frames) / window_seconds
        } else {
            0.0
        },
        missed_deadlines,
        window_seconds,
    }
}

fn measure_scene(
    engine: &mut dyn Engine,
    dir: &Path,
    options: MeasureSceneOptions<'_>,
) -> Result<MeasureReport, BenchError> {
    let MeasureSceneOptions {
        frames,
        warmup,
        pause_at,
        pinned,
        rate,
        measure_energy,
        idle_memory,
        native,
    } = options;
    let (scene, native) = load_scene(dir, native)?;
    let blobs = convert::load_blobs(&scene, dir)?;
    let input = EncodeInput {
        scene: &scene,
        blobs: &blobs,
    };
    let heterogeneous = affinity::cpu_freqs_differ();
    if pinned.is_none() && heterogeneous {
        tracing::warn!(
            "CPUs report differing max frequencies; this run's placement is uncontrolled — \
             pass --cpu (e.g. --cpu 7) to pin to one cluster"
        );
    }
    let prepare_cpu = affinity::current_cpu();
    let t_prepare = Instant::now();
    engine.prepare(&input)?;
    let prepare_seconds = t_prepare.elapsed().as_secs_f64();
    let prepare_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    let (period, window_hint) = match rate {
        Some(hz) => pacing(hz, frames).map(|(p, w)| (Some(p), w))?,
        None => (None, Duration::from_secs(30)),
    };
    let window = run_frames(
        &mut *engine,
        &input,
        frames,
        warmup,
        FrameLoopOptions {
            pause_at,
            period,
            window_hint,
            measure_energy,
        },
    )?;
    let window_end = Instant::now();
    let window_seconds = window_end.duration_since(window.start).as_secs_f64();
    let energy_outcome = window
        .meter
        .map(|m| m.finish(window.start, window_end, frames))
        .transpose()?;
    // The steady snapshot lands after the pacing window and the meter
    // close, so its capture is never charged to a measured frame.
    let steady_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    // Counters embed a memory snapshot; take them at steady state,
    // before retirement shrinks what the engine reports.
    let counters = engine.counters();
    // #169 A5: retire, then take the post-retirement observation.
    engine.trim()?;
    let post_retire_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    let memory_samples = memory_samples(prepare_memory, &window.memory_samples, steady_memory);
    let missed_deadlines = window.missed_deadlines;
    let samples = window.samples;
    let pacing = rate.map(|hz| pacing_report(hz, frames, missed_deadlines, window_seconds));
    let conditions = conditions::collect(
        energy_outcome
            .as_ref()
            .and_then(|o| o.thermal_pressure.clone()),
    );
    let placement = measure_placement(pinned, heterogeneous, prepare_cpu, &samples);
    let percentiles = frame_percentiles(&samples);
    Ok(MeasureReport {
        engine: engine.info().name,
        info: engine.info().clone(),
        scene: dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        width: scene.width,
        height: scene.height,
        native,
        prepare_seconds,
        warmup_frames: warmup,
        samples,
        placement,
        percentiles,
        pacing,
        energy: energy_outcome.map(|o| o.report),
        conditions,
        counters,
        memory: MemoryReport::new(idle_memory, &memory_samples, Some(post_retire_memory)),
        device: engine.device(),
    })
}

/// The sample list a report's peak folds over: prepare, the warmup
/// captures `run_frames` returned — measured frames sample nothing —
/// and the steady snapshot taken after the window and meter closed.
fn memory_samples(
    prepare: MemorySnapshot,
    warmup: &[MemorySnapshot],
    steady: MemorySnapshot,
) -> Vec<MemorySnapshot> {
    let mut samples = Vec::with_capacity(warmup.len() + 2);
    samples.push(prepare);
    samples.extend(warmup.iter().cloned());
    samples.push(steady);
    samples
}

fn frame_percentiles(samples: &[FrameSample]) -> Percentiles {
    let enc: Vec<f64> = samples.iter().map(|s| s.encode_seconds).collect();
    let sub: Vec<f64> = samples.iter().map(|s| s.submit_seconds).collect();
    let gpu: Vec<f64> = samples.iter().filter_map(|s| s.gpu_seconds).collect();
    Percentiles {
        encode_seconds: percentiles(&enc).unwrap_or([0.0; 3]),
        submit_seconds: percentiles(&sub).unwrap_or([0.0; 3]),
        gpu_seconds: percentiles(&gpu),
        passes: pass_percentiles(samples),
        phases: phase_percentiles(samples),
    }
}

fn measure_placement(
    pinned: Option<&[u32]>,
    heterogeneous: bool,
    prepare_cpu: Option<u32>,
    samples: &[FrameSample],
) -> Placement {
    Placement {
        requested: pinned.map(<[u32]>::to_vec),
        controlled: pinned.is_some(),
        heterogeneous,
        prepare_cpu,
        cpus: cpu_use(prepare_cpu, samples),
        migrated: u32::try_from(samples.iter().filter(|s| s.migrated).count()).unwrap_or(u32::MAX),
    }
}

/// Percentiles per pass index, over the frames with the modal pass
/// count.
fn pass_percentiles(samples: &[FrameSample]) -> Vec<PassPercentiles> {
    // The modal pass count; frames with a different count are skipped.
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for s in samples {
        *counts.entry(s.passes.len()).or_default() += 1;
    }
    let Some(mode) = counts.into_iter().max_by_key(|(_, n)| *n).map(|(n, _)| n) else {
        return Vec::new();
    };
    if mode == 0 {
        return Vec::new();
    }
    let frames: Vec<&FrameSample> = samples.iter().filter(|s| s.passes.len() == mode).collect();
    (0..mode)
        .map(|i| {
            let times: Vec<f64> = frames
                .iter()
                .filter_map(|s| s.passes[i].gpu_seconds)
                .collect();
            PassPercentiles {
                name: frames[0].passes[i].name.clone(),
                width: frames[0].passes[i].width,
                height: frames[0].passes[i].height,
                format: frames[0].passes[i].format.clone(),
                gpu_seconds: percentiles(&times),
            }
        })
        .collect()
}

/// Percentiles per phase index, over the frames with the modal phase
/// count.
fn phase_percentiles(samples: &[FrameSample]) -> Vec<PhasePercentiles> {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for s in samples {
        *counts.entry(s.phases.len()).or_default() += 1;
    }
    let Some(mode) = counts.into_iter().max_by_key(|(_, n)| *n).map(|(n, _)| n) else {
        return Vec::new();
    };
    if mode == 0 {
        return Vec::new();
    }
    let frames: Vec<&FrameSample> = samples.iter().filter(|s| s.phases.len() == mode).collect();
    (0..mode)
        .map(|i| {
            let times: Vec<f64> = frames.iter().map(|s| s.phases[i].seconds).collect();
            PhasePercentiles {
                name: frames[0].phases[i].name.clone(),
                seconds: percentiles(&times).unwrap_or([0.0; 3]),
            }
        })
        .collect()
}

/// The `capacity` subcommand's fields, borrowed to avoid cloning paths.
#[derive(Clone, Copy)]
struct CapacityOpts<'a> {
    scene: Option<&'a Path>,
    corpus: Option<&'a Path>,
    frames: u32,
    warmup: u32,
    budget_ms: f64,
    max_k: u32,
    out: Option<&'a Path>,
    out_dir: Option<&'a Path>,
    cpu: Option<&'a str>,
    native: Option<&'a str>,
}

/// One engine's sweep state on one scene.
///
/// The doubling phase grows `lo` — the largest repetition factor whose
/// probe stayed within budget — from 1 until a probe lands over budget,
/// recording that factor as `hi`. The binary phase then halves the
/// `(lo, hi)` gap until they are adjacent: `lo` is the max sustained k
/// and `hi = lo + 1` the first over-budget level.
#[derive(Default)]
struct Sweep {
    /// Largest probed `k` within budget; `0` until one fits.
    lo: u32,
    /// Smallest probed `k` over budget.
    hi: Option<u32>,
    /// Every probe, in the order it ran.
    probes: Vec<CapacityProbe>,
    /// The feature that stopped the sweep before its first probe.
    unsupported: Option<(cherenkov_scene::Feature, Option<&'static str>)>,
    /// The probe error that stopped the sweep mid-run, if any.
    error: Option<String>,
    /// Prepare, warmup-frame and post-window snapshots across every
    /// probe, in run order.
    memory_samples: Vec<MemorySnapshot>,
    /// The sweep converged or saturated — or never ran (unsupported).
    done: bool,
}

impl Sweep {
    /// The next repetition factor to probe: `1`, then doubling `lo`
    /// (capped at `max_k`), then the midpoint of `(lo, hi)`. `None` when
    /// the sweep is finished.
    fn next_k(&self, max_k: u32) -> Option<u32> {
        if self.done {
            return None;
        }
        if let Some(hi) = self.hi {
            let mid = self.lo + (hi - self.lo) / 2;
            return (mid > self.lo).then_some(mid);
        }
        if self.lo == 0 && self.probes.is_empty() {
            return Some(1);
        }
        let next = self.lo.saturating_mul(2).min(max_k);
        (next > self.lo).then_some(next)
    }

    /// Records a probe's p99 and the bound it moved. The sweep is done
    /// once `lo` and `hi` sit adjacent.
    fn record(&mut self, k: u32, p99_seconds: f64, budget_seconds: f64) {
        self.probes.push(CapacityProbe { k, p99_seconds });
        if p99_seconds <= budget_seconds {
            self.lo = k;
        } else {
            self.hi = Some(self.hi.map_or(k, |h| h.min(k)));
        }
        if let Some(hi) = self.hi {
            self.done = hi.saturating_sub(self.lo) <= 1;
        }
    }
}

/// A sample's frame seconds for the capacity budget: the measuring
/// thread's wall time (encode + submit), raised to the GPU time where
/// the backend reports it — a pipelined renderer's sustained rate is
/// limited by its slowest stage.
fn frame_seconds(s: &FrameSample) -> f64 {
    let cpu = s.encode_seconds + s.submit_seconds;
    s.gpu_seconds.map_or(cpu, |gpu| cpu.max(gpu))
}

/// One capacity probe: prepare `scene` (already at factor `k`) and run
/// the same warmup + measured frame loop `measure` runs, flat out.
/// Returns the p99 frame seconds and the probe's memory snapshots:
/// prepare, one per warmup frame, and a full snapshot after the
/// window.
fn probe_once(
    engine: &mut dyn Engine,
    scene: &Scene,
    blobs: &convert::Blobs,
    frames: u32,
    warmup: u32,
) -> Result<(f64, Vec<MemorySnapshot>), BenchError> {
    let input = EncodeInput { scene, blobs };
    engine.prepare(&input)?;
    let prepare_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    let window = run_frames(
        engine,
        &input,
        frames,
        warmup,
        FrameLoopOptions {
            pause_at: None,
            period: None,
            window_hint: Duration::ZERO,
            measure_energy: false,
        },
    )?;
    let steady_memory = MemorySnapshot::capture(engine.memory(), SampleDetail::Full);
    let memory = memory_samples(prepare_memory, &window.memory_samples, steady_memory);
    let times: Vec<f64> = window.samples.iter().map(frame_seconds).collect();
    let p99 = percentiles(&times).map(|p| p[2]).ok_or_else(|| {
        BenchError::Engine("capacity: --frames 0 leaves nothing to measure".into())
    })?;
    Ok((p99, memory))
}

fn capacity_cmd(engines: &[String], opts: CapacityOpts<'_>) -> Result<(), BenchError> {
    if !(opts.budget_ms.is_finite() && opts.budget_ms > 0.0) {
        return Err(BenchError::Engine(format!(
            "--budget-ms must be a positive, finite ms value, got {}",
            opts.budget_ms
        )));
    }
    if opts.frames == 0 {
        return Err(BenchError::Engine(
            "capacity: --frames must be at least 1 for a p99".into(),
        ));
    }
    if opts.max_k == 0 {
        return Err(BenchError::Engine("--max-k must be at least 1".into()));
    }
    let native = parse_native(opts.native)?;
    let pinned = opts.cpu.map(affinity::parse_cpu_list).transpose()?;
    if let Some(cpus) = &pinned {
        affinity::pin_current_thread(cpus)?;
    }
    // Engines persist across scenes and probes — like `measure`, which
    // runs a whole corpus on one instance — while each scene's sweep
    // state resets.
    let mut engines = engines
        .iter()
        .map(|name| create_engine(name))
        .collect::<Result<Vec<_>, _>>()?;
    // The engines' idle baseline, captured once before any scene
    // prepares — the same snapshot `measure` records per engine.
    let idle_memory: Vec<MemorySnapshot> = engines
        .iter()
        .map(|e| MemorySnapshot::capture(e.memory(), SampleDetail::Full))
        .collect();
    for dir in scene_dirs(opts.scene, opts.corpus)? {
        let out_path = match (opts.out, opts.out_dir) {
            (Some(o), None) => o.to_path_buf(),
            (None, Some(d)) => d.join(format!(
                "capacity-{}.json",
                dir.file_name().unwrap_or_default().to_string_lossy()
            )),
            _ => return Err(BenchError::Engine("--out or --out-dir required".into())),
        };
        let report = sweep_scene(&mut engines, &dir, &opts, native, &idle_memory)?;
        write_json(&report, &out_path)?;
        for result in &report.results {
            tracing::info!(
                scene = %dir.display(),
                engine = result.engine,
                max_k = result.max_k,
                p99_seconds = ?result.p99_seconds,
                p99_seconds_next = ?result.p99_seconds_next,
                out = %out_path.display(),
                "capacity"
            );
        }
    }
    Ok(())
}

/// Sweeps one scene: interleaved rounds across `engines` — each active
/// engine probes its own next `k` once per round, in `--engine` order —
/// until every sweep converges or saturates.
fn sweep_scene(
    engines: &mut [Box<dyn Engine>],
    dir: &Path,
    opts: &CapacityOpts<'_>,
    native: Option<(u32, u32)>,
    idle_memory: &[MemorySnapshot],
) -> Result<CapacityReport, BenchError> {
    let (base_scene, native_report) = load_scene(dir, native)?;
    // Blobs key on content hashes that `repeated` leaves untouched.
    let blobs = convert::load_blobs(&base_scene, dir)?;
    let budget_seconds = opts.budget_ms / 1000.0;
    let mut sweeps: Vec<Sweep> = engines.iter().map(|_| Sweep::default()).collect();
    loop {
        let mut active = false;
        for (engine, sweep) in engines.iter_mut().zip(sweeps.iter_mut()) {
            let Some(k) = sweep.next_k(opts.max_k) else {
                sweep.done = true;
                continue;
            };
            active = true;
            let scene = transform::repeated(&base_scene, k);
            match probe_once(&mut **engine, &scene, &blobs, opts.frames, opts.warmup) {
                Ok((p99, memory)) => {
                    sweep.memory_samples.extend(memory);
                    sweep.record(k, p99, budget_seconds);
                    tracing::info!(
                        scene = %dir.display(),
                        engine = engine.info().name,
                        k,
                        p99_seconds = p99,
                        "probe"
                    );
                }
                Err(BenchError::Unsupported { feature, api, .. }) => {
                    sweep.unsupported = Some((feature.clone(), api));
                    sweep.done = true;
                    tracing::warn!(
                        scene = %dir.display(),
                        engine = engine.info().name,
                        ?feature,
                        "unsupported"
                    );
                }
                Err(e) => {
                    // A probe error is the engine's own cap on this scene
                    // (e.g. the bounded glyph atlas exhausting): it bounds
                    // the sweep like an over-budget probe would, ending it
                    // at the largest `k` already sustained. It must not
                    // abort the interleaved sweep for the other engines.
                    tracing::warn!(
                        scene = %dir.display(),
                        engine = engine.info().name,
                        k,
                        error = %e,
                        "probe failed; sweep ends at the last sustained k"
                    );
                    sweep.error = Some(e.to_string());
                    sweep.done = true;
                }
            }
        }
        if !active {
            break;
        }
    }
    let results = engines
        .iter()
        .zip(sweeps.iter())
        .zip(idle_memory.iter())
        .map(|((engine, sweep), idle)| {
            let (unsupported, missing_api) = sweep
                .unsupported
                .clone()
                .map_or((None, None), |(f, api)| (Some(f), api));
            CapacityResult {
                engine: engine.info().name,
                info: engine.info().clone(),
                unsupported,
                missing_api,
                error: sweep.error.clone(),
                max_k: sweep.lo,
                p99_seconds: sweep
                    .probes
                    .iter()
                    .find(|p| p.k == sweep.lo)
                    .map(|p| p.p99_seconds),
                p99_seconds_next: sweep
                    .probes
                    .iter()
                    .find(|p| p.k == sweep.lo + 1)
                    .map(|p| p.p99_seconds),
                probes: sweep.probes.clone(),
                memory: MemoryReport::new(idle.clone(), &sweep.memory_samples, None),
            }
        })
        .collect();
    Ok(CapacityReport {
        scene: dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        budget_ms: opts.budget_ms,
        frames: opts.frames,
        warmup: opts.warmup,
        native: native_report,
        results,
        device: engines
            .first()
            .map_or_else(DeviceInfo::default, |e| e.device()),
    })
}

/// Record a scene the adapter cannot execute faithfully.
fn write_unsupported(
    engine: &dyn Engine,
    dir: &Path,
    feature: cherenkov_scene::Feature,
    api: Option<&'static str>,
    memory_idle: MemorySnapshot,
    out: &Path,
) -> Result<(), BenchError> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json(
        &UnsupportedReport {
            engine: engine.info().name,
            info: engine.info().clone(),
            scene: dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            unsupported: feature,
            missing_api: api,
            memory_idle,
        },
        out,
    )
}

fn write_render(rendered: &RenderOutput, out: &Path) -> Result<(), BenchError> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stem = out.parent().map_or_else(
        || out.with_extension(""),
        |p| p.join(out.file_stem().unwrap_or_default()),
    );
    rendered
        .image
        .write_png(&stem.with_extension("engine.png"))?;
    metrics::write_heatmap(
        &rendered.heatmap,
        rendered.image.width,
        rendered.image.height,
        &stem.with_extension("heatmap.png"),
    )?;
    write_json(&rendered.report, out)
}

fn write_json<T: serde::Serialize>(v: &T, path: &Path) -> Result<(), BenchError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(v)
        .map_err(|e| BenchError::Engine(format!("json serialize: {e}")))?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}
