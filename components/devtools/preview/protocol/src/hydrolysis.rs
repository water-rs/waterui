//! Offscreen Hydrolysis preview protocol.
//!
//! The CLI drives the generated Hydrolysis preview binary with a single JSON
//! run configuration (passed as a file path through
//! [`PREVIEW_RUN_CONFIG_ENV`]) and reads structured results back: perf runs
//! emit one [`PERF_REPORT_LINE_PREFIX`]-prefixed JSON line on stdout.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Environment variable carrying the path of the JSON-encoded
/// [`PreviewRunConfig`] for the generated preview binary.
pub const PREVIEW_RUN_CONFIG_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_RUN_CONFIG";

/// Stdout line prefix carrying the JSON-encoded `Vec<PerfMeasurement>` of a
/// perf run.
pub const PERF_REPORT_LINE_PREFIX: &str = "waterui-perf-report-json ";

/// One offscreen Hydrolysis preview invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewRunConfig {
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// What the run produces.
    pub mode: PreviewRunMode,
}

/// What a preview run produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreviewRunMode {
    /// A single PNG capture after the view tree has mounted.
    Image {
        /// Destination PNG path.
        output: PathBuf,
    },
    /// A timeline capture: frames at `captures_ms` with `events` replayed at
    /// their timestamps.
    Scenario {
        /// Directory receiving `frame-XXXXms.png` captures.
        output_dir: PathBuf,
        /// Capture timestamps in milliseconds from scenario start.
        captures_ms: Vec<u64>,
        /// Input events sorted by timestamp.
        events: Vec<ScenarioEvent>,
    },
    /// Semantic accessibility-tree assertions (no render target).
    Semantic,
    /// GPU perf measurement.
    Perf(PerfRunConfig),
}

/// Configuration of a perf measurement run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerfRunConfig {
    /// Warmup frames before sampling.
    pub warmups: u32,
    /// Recorded frames per measurement.
    pub samples: u32,
    /// Independent measurement repetitions.
    pub repetitions: u32,
    /// CPU flamegraph SVG destination, when profiling is requested.
    pub flamegraph: Option<PathBuf>,
    /// Flamegraph sampling frequency in Hertz.
    pub flamegraph_frequency: i32,
}

/// One input event in a preview scenario timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScenarioEvent {
    /// Event timestamp in milliseconds from scenario start.
    pub at_ms: u64,
    /// Event kind.
    pub kind: ScenarioEventKind,
    /// Pointer x coordinate in logical units.
    pub x: f32,
    /// Pointer y coordinate in logical units.
    pub y: f32,
    /// Pointer button for down/up events.
    pub button: ScenarioPointerButton,
    /// Scroll delta along the x axis for scroll events.
    pub dx: f32,
    /// Scroll delta along the y axis for scroll events.
    pub dy: f32,
    /// Whether scroll delta values are line units instead of logical units.
    pub is_line_delta: bool,
}

/// Event kind in a preview scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScenarioEventKind {
    /// Move the pointer without pressing.
    PointerMove,
    /// Press a pointer button.
    PointerDown,
    /// Release a pointer button.
    PointerUp,
    /// Cancel the active pointer.
    PointerCancel,
    /// Dispatch a wheel or trackpad scroll event.
    Scroll,
}

/// Pointer button identifier for scenario events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScenarioPointerButton {
    /// The primary button.
    #[default]
    Primary,
    /// The secondary button.
    Secondary,
    /// The middle button.
    Middle,
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
