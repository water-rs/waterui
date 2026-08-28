//! Benchmark report wire format shared by `waterui-testing` and the `water` CLI.
//!
//! `#[waterui::bench]` tests run under `cargo nextest`; in full-run mode each
//! bench serializes one [`BenchReport`] JSON file into the directory named by
//! [`BENCH_REPORT_DIR_ENV`]. `water bench` sets the environment, runs nextest,
//! and deserializes the same types, so field renames break the build instead of
//! silently desynchronizing the two halves.

use serde::{Deserialize, Serialize};

/// Environment variable selecting the recorded frame count per measurement.
///
/// Setting any of the three run-shape variables switches benches from smoke
/// mode into a full measurement run.
pub const BENCH_SAMPLES_ENV: &str = "WATERUI_BENCH_SAMPLES";
/// Environment variable selecting the unrecorded warmup frame count.
pub const BENCH_WARMUPS_ENV: &str = "WATERUI_BENCH_WARMUPS";
/// Environment variable selecting the independent measurement repetitions.
pub const BENCH_REPETITIONS_ENV: &str = "WATERUI_BENCH_REPETITIONS";
/// Environment variable naming the directory that receives one
/// [`BenchReport`] JSON file per executed bench in full-run mode.
pub const BENCH_REPORT_DIR_ENV: &str = "WATERUI_BENCH_REPORT_DIR";

/// Environment variable capping every bench's p95 frame-time budget, in microseconds.
pub const BENCH_MAX_P95_US_ENV: &str = "WATERUI_BENCH_MAX_P95_US";
/// Environment variable capping every bench's mean frame-time budget, in microseconds.
pub const BENCH_MAX_MEAN_US_ENV: &str = "WATERUI_BENCH_MAX_MEAN_US";
/// Environment variable capping every bench's rebuild-ratio budget.
pub const BENCH_MAX_REBUILD_RATIO_ENV: &str = "WATERUI_BENCH_MAX_REBUILD_RATIO";
/// Environment variable capping every bench's compositor scene-layer budget.
pub const BENCH_MAX_SCENE_LAYERS_ENV: &str = "WATERUI_BENCH_MAX_SCENE_LAYERS";
/// Environment variable capping every bench's embedded GPU-surface-layer budget.
pub const BENCH_MAX_GPU_SURFACE_LAYERS_ENV: &str = "WATERUI_BENCH_MAX_GPU_SURFACE_LAYERS";
/// Environment variable capping every bench's Vello clip-layer budget.
pub const BENCH_MAX_CLIP_LAYERS_ENV: &str = "WATERUI_BENCH_MAX_CLIP_LAYERS";

/// File name of one bench's report inside the report directory.
#[must_use]
pub fn bench_report_file_name(crate_name: &str, bench_name: &str) -> String {
    format!("{crate_name}__{bench_name}.json")
}

/// One executed bench: its identity, run shape, budgets, and measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    /// Package that declared the bench.
    pub crate_name: String,
    /// Bench function name (without the `waterui_bench_` prefix).
    pub bench_name: String,
    /// Run shape the measurements were recorded with.
    pub config: BenchRunConfig,
    /// Effective budgets the run was judged against (attribute budgets merged
    /// with environment caps), echoed so reporters can show headroom.
    pub budgets: BenchBudgets,
    /// Recorded measurements in insertion order.
    pub measurements: Vec<PerfMeasurement>,
}

/// Frame-run shape of one bench execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchRunConfig {
    /// Unrecorded frames run before sampling.
    pub warmups: u32,
    /// Recorded frames per measurement.
    pub samples: u32,
    /// Independent measurement repetitions.
    pub repetitions: u32,
}

/// Optional per-metric ceilings applied to every measurement a bench records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchBudgets {
    /// Maximum allowed 95th-percentile frame time, in microseconds.
    pub max_p95_us: Option<u64>,
    /// Maximum allowed mean frame time, in microseconds.
    pub max_mean_us: Option<u64>,
    /// Maximum allowed share of sampled frames that rebuilt (0.0..=1.0).
    pub max_rebuild_ratio: Option<f64>,
    /// Maximum allowed compositor scene layers submitted by one frame.
    pub max_scene_layers: Option<u64>,
    /// Maximum allowed embedded GPU surface layers submitted by one frame.
    pub max_gpu_surface_layers: Option<u64>,
    /// Maximum allowed Vello clip layers pushed by one frame.
    pub max_clip_layers: Option<u64>,
}

/// Aggregated statistics of one perf measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfMeasurement {
    /// Scenario name.
    pub name: String,
    /// Recorded sample count.
    pub samples: u64,
    /// Mean frame time in microseconds.
    pub mean_us: u64,
    /// Median frame time in microseconds.
    pub median_us: u64,
    /// 95th-percentile frame time in microseconds.
    pub p95_us: u64,
    /// Minimum frame time in microseconds.
    pub min_us: u64,
    /// Maximum frame time in microseconds.
    pub max_us: u64,
    /// Frames that performed a structural rebuild.
    pub rebuilt_frames: u64,
    /// Frames that rendered.
    pub rendered_frames: u64,
    /// Frames that did no work.
    pub idle_frames: u64,
    /// Mean frame time of rendered frames in microseconds.
    pub rendered_mean_us: u64,
    /// 95th-percentile frame time of rendered frames in microseconds.
    pub rendered_p95_us: u64,
    /// Maximum frame time of rendered frames in microseconds.
    pub rendered_max_us: u64,
    /// Frames exceeding the 120 fps budget.
    pub missed_120fps_frames: u64,
    /// Frames exceeding the 60 fps budget.
    pub missed_60fps_frames: u64,
    /// Measurement-cache hits across the run.
    pub measurement_cache_hits: u64,
    /// Measurement-cache misses across the run.
    pub measurement_cache_misses: u64,
    /// Compositor scene layers submitted.
    pub scene_layers: u64,
    /// Vello scene layers submitted.
    pub vello_scene_layers: u64,
    /// GPU surface layers submitted.
    pub gpu_surface_layers: u64,
    /// Clip layers pushed.
    pub clip_layers: u64,
    /// Maximum clip depth reached.
    pub max_clip_depth: u64,
    /// Applied filter count.
    pub applied_filter_count: u64,
    /// Applied filter capture time in microseconds.
    pub applied_filter_capture_us: u64,
    /// Applied filter effect time in microseconds.
    pub applied_filter_effect_us: u64,
    /// Frame phase aggregates.
    pub phases: PerfPhases,
    /// Per-frame samples.
    pub frames: Vec<PerfFrame>,
}

/// Frame phase aggregates of a perf measurement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerfPhases {
    /// Mean rebuild phase time in microseconds.
    pub rebuild_mean_us: u64,
    /// 95th-percentile rebuild phase time in microseconds.
    pub rebuild_p95_us: u64,
    /// Mean content-build phase time in microseconds.
    pub build_content_mean_us: u64,
    /// 95th-percentile content-build phase time in microseconds.
    pub build_content_p95_us: u64,
    /// Mean scene-dispatch phase time in microseconds.
    pub scene_dispatch_mean_us: u64,
    /// 95th-percentile scene-dispatch phase time in microseconds.
    pub scene_dispatch_p95_us: u64,
    /// Mean scene-finish phase time in microseconds.
    pub scene_finish_mean_us: u64,
    /// 95th-percentile scene-finish phase time in microseconds.
    pub scene_finish_p95_us: u64,
    /// Mean render phase time in microseconds.
    pub render_mean_us: u64,
    /// 95th-percentile render phase time in microseconds.
    pub render_p95_us: u64,
    /// Mean animation phase time in microseconds.
    pub animation_mean_us: u64,
    /// Mean input phase time in microseconds.
    pub input_mean_us: u64,
}

/// One recorded perf frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfFrame {
    /// Frame index within the measurement.
    pub index: u64,
    /// Total frame time in microseconds.
    pub total_us: u64,
    /// Rebuild phase time in microseconds.
    pub rebuild_us: u64,
    /// Content-build phase time in microseconds.
    pub build_content_us: u64,
    /// Scene-dispatch phase time in microseconds.
    pub scene_dispatch_us: u64,
    /// Scene-finish phase time in microseconds.
    pub scene_finish_us: u64,
    /// Render phase time in microseconds.
    pub render_us: u64,
    /// Surface acquire time in microseconds.
    pub acquire_us: u64,
    /// Present time in microseconds.
    pub present_us: u64,
    /// Animation phase time in microseconds.
    pub animation_us: u64,
    /// Input phase time in microseconds.
    pub input_us: u64,
    /// Executor drain time before the frame in microseconds.
    pub executor_before_us: u64,
    /// Executor drain time after the frame in microseconds.
    pub executor_after_us: u64,
    /// Whether the frame performed a structural rebuild.
    pub rebuilt: bool,
    /// Whether the frame rendered.
    pub rendered: bool,
    /// Whether the frame captured a snapshot.
    pub captured_snapshot: bool,
    /// Process CPU usage in percent.
    pub cpu_percent: f64,
    /// Process memory footprint in bytes.
    pub memory_bytes: u64,
    /// GPU frame time (acquire + render + present) in microseconds.
    pub gpu_frame_us: u64,
    /// Measurement-cache hits this frame.
    pub measurement_cache_hits: u64,
    /// Measurement-cache misses this frame.
    pub measurement_cache_misses: u64,
    /// Compositor scene layers this frame.
    pub scene_layers: u64,
    /// Vello scene layers this frame.
    pub vello_scene_layers: u64,
    /// GPU surface layers this frame.
    pub gpu_surface_layers: u64,
    /// Clip layers pushed this frame.
    pub clip_layers: u64,
    /// Maximum clip depth this frame.
    pub max_clip_depth: u64,
    /// Applied filter count this frame.
    pub applied_filter_count: u64,
    /// Applied filter capture time this frame in microseconds.
    pub applied_filter_capture_us: u64,
    /// Applied filter effect time this frame in microseconds.
    pub applied_filter_effect_us: u64,
}
