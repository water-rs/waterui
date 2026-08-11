//! Performance reporting for `water preview perf`.
//!
//! Parsing a perf run's output, judging it against a budget, and rendering it as
//! JSON, a Chrome trace or an HTML report is all analysis, not interaction — the
//! terminal layer only chooses an output format and prints the human summary.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{Result, bail};
use serde::Serialize;

use waterui_preview_protocol::hydrolysis::{
    PerfFrame as PreviewPerfFrame, PerfMeasurement as PreviewPerfMeasurement,
};

#[derive(Debug, Serialize)]
pub(crate) struct PreviewPerfOutput<'a> {
    reports: &'a [PreviewPerfReport],
}

/// One preview target's perf run: its measurements and optional flamegraph.
#[derive(Debug, Serialize)]
pub struct PreviewPerfReport {
    /// Preview target the run measured.
    pub target: String,
    /// Measurements recorded for the target.
    pub measurements: Vec<PreviewPerfMeasurement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Flamegraph captured alongside the run, when one was requested.
    pub flamegraph: Option<PathBuf>,
}
/// Parse the JSON a perf run writes to stdout into a report.
///
/// # Errors
/// Returns an error if the output is not a valid perf payload.
pub fn parse_preview_perf_output(target: String, output: &str) -> Result<PreviewPerfReport> {
    use waterui_preview_protocol::hydrolysis::PERF_REPORT_LINE_PREFIX;
    let json = output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(PERF_REPORT_LINE_PREFIX))
        .ok_or_else(|| color_eyre::eyre::eyre!("Hydrolysis preview perf emitted no perf report"))?;
    let measurements: Vec<PreviewPerfMeasurement> =
        serde_json::from_str(json).map_err(|error| {
            color_eyre::eyre::eyre!("Failed to parse hydrolysis preview perf report: {error}")
        })?;
    if measurements.is_empty() {
        bail!("Hydrolysis preview perf emitted no perf measurements");
    }
    Ok(PreviewPerfReport {
        target,
        measurements,
        flamegraph: None,
    })
}

/// Render a microsecond count the way the perf report displays it.
#[must_use]
pub fn micros_label(value: u64) -> String {
    format!("{value}us")
}

#[expect(
    clippy::cast_precision_loss,
    reason = "preview charts intentionally project integer telemetry into floating-point display coordinates"
)]
pub(crate) const fn metric_to_f64(value: u64) -> f64 {
    value as f64
}

#[expect(
    clippy::cast_precision_loss,
    reason = "preview chart sample counts are converted only for display averages and coordinates"
)]
pub(crate) const fn sample_count_to_f64(value: usize) -> f64 {
    value as f64
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "preview chart scales contain finite non-negative telemetry and labels use rounded integers"
)]
pub(crate) fn rounded_metric_to_u64(value: f64) -> u64 {
    assert!(
        value.is_finite() && value >= 0.0 && value <= metric_to_f64(u64::MAX),
        "preview chart metric must be finite, non-negative, and fit into u64"
    );
    value.round() as u64
}

/// Render a byte count the way the perf report displays it.
#[must_use]
pub fn bytes_label(value: u64) -> String {
    const MIB: f64 = 1_048_576.0;
    format!("{:.1}MiB", metric_to_f64(value) / MIB)
}

/// CPU, memory, GPU and scene-complexity aggregates over a measurement's frames.
#[derive(Debug, Clone, Copy)]
pub struct PreviewPerfResourceSummary {
    /// Mean CPU utilization across sampled frames.
    pub avg_cpu_percent: f64,
    /// Peak CPU utilization across sampled frames.
    pub max_cpu_percent: f64,
    /// Peak resident memory.
    pub max_memory_bytes: u64,
    /// Mean GPU frame time.
    pub avg_gpu_frame_us: u64,
    /// Peak GPU frame time.
    pub max_gpu_frame_us: u64,
    /// Mean number of scene layers.
    pub avg_scene_layers: f64,
    /// Peak number of scene layers.
    pub max_scene_layers: u64,
    /// Mean number of clip layers.
    pub avg_clip_layers: f64,
    /// Peak clip nesting depth.
    pub max_clip_depth: u64,
}

/// Aggregate a measurement's per-frame resource samples, if it recorded any.
///
/// # Panics
/// Panics if a measurement records frames whose sample count does not fit a `f64`.
#[must_use]
pub fn resource_summary(
    measurement: &PreviewPerfMeasurement,
) -> Option<PreviewPerfResourceSummary> {
    if measurement.frames.is_empty() {
        return None;
    }
    let sample_count = sample_count_to_f64(measurement.frames.len());
    let avg_cpu_percent = measurement
        .frames
        .iter()
        .map(|frame| frame.cpu_percent)
        .sum::<f64>()
        / sample_count;
    let avg_gpu_frame_us = measurement
        .frames
        .iter()
        .map(|frame| frame.gpu_frame_us)
        .sum::<u64>()
        / u64::try_from(measurement.frames.len()).expect("perf sample count should fit u64");
    Some(PreviewPerfResourceSummary {
        avg_cpu_percent,
        max_cpu_percent: measurement
            .frames
            .iter()
            .map(|frame| frame.cpu_percent)
            .fold(0.0, f64::max),
        max_memory_bytes: measurement
            .frames
            .iter()
            .map(|frame| frame.memory_bytes)
            .max()
            .unwrap_or_default(),
        avg_gpu_frame_us,
        max_gpu_frame_us: measurement
            .frames
            .iter()
            .map(|frame| frame.gpu_frame_us)
            .max()
            .unwrap_or_default(),
        avg_scene_layers: measurement
            .frames
            .iter()
            .map(|frame| metric_to_f64(frame.scene_layers))
            .sum::<f64>()
            / sample_count,
        max_scene_layers: measurement
            .frames
            .iter()
            .map(|frame| frame.scene_layers)
            .max()
            .unwrap_or_default(),
        avg_clip_layers: measurement
            .frames
            .iter()
            .map(|frame| metric_to_f64(frame.clip_layers))
            .sum::<f64>()
            / sample_count,
        max_clip_depth: measurement
            .frames
            .iter()
            .map(|frame| frame.max_clip_depth)
            .max()
            .unwrap_or_default(),
    })
}

/// Thresholds a perf run must stay within for the command to succeed.
#[derive(Clone, Copy, Debug)]
pub struct PreviewPerfBudget {
    /// Maximum allowed 95th-percentile frame time.
    pub p95_us: Option<u64>,
    /// Maximum allowed share of frames that rebuilt.
    pub rebuild_ratio: Option<f64>,
    /// Maximum allowed scene layer count.
    pub scene_layers: Option<u64>,
    /// Maximum allowed GPU surface layer count.
    pub gpu_surface_layers: Option<u64>,
    /// Maximum allowed clip layer count.
    pub clip_layers: Option<u64>,
}

/// Fail the run when any measurement exceeds the configured budget.
///
/// # Errors
/// Returns an error naming every threshold the report exceeded.
pub fn enforce_perf_budget(report: &PreviewPerfReport, budget: PreviewPerfBudget) -> Result<()> {
    for measurement in &report.measurements {
        if let Some(max_p95_us) = budget.p95_us
            && measurement.p95_us > max_p95_us
        {
            bail!(
                "Preview perf `{}` p95 {}us exceeded budget {}us",
                measurement.name,
                measurement.p95_us,
                max_p95_us
            );
        }
        if let Some(max_rebuild_ratio) = budget.rebuild_ratio {
            let rebuild_ratio =
                metric_to_f64(measurement.rebuilt_frames) / metric_to_f64(measurement.samples);
            if rebuild_ratio > max_rebuild_ratio {
                bail!(
                    "Preview perf `{}` rebuild ratio {} exceeded budget {}",
                    measurement.name,
                    rebuild_ratio,
                    max_rebuild_ratio
                );
            }
        }
        if let Some(max_scene_layers) = budget.scene_layers
            && measurement.scene_layers > max_scene_layers
        {
            bail!(
                "Preview perf `{}` scene layers {} exceeded budget {}",
                measurement.name,
                measurement.scene_layers,
                max_scene_layers
            );
        }
        if let Some(max_gpu_surface_layers) = budget.gpu_surface_layers
            && measurement.gpu_surface_layers > max_gpu_surface_layers
        {
            bail!(
                "Preview perf `{}` GPU surface layers {} exceeded budget {}",
                measurement.name,
                measurement.gpu_surface_layers,
                max_gpu_surface_layers
            );
        }
        if let Some(max_clip_layers) = budget.clip_layers
            && measurement.clip_layers > max_clip_layers
        {
            bail!(
                "Preview perf `{}` clip layers {} exceeded budget {}",
                measurement.name,
                measurement.clip_layers,
                max_clip_layers
            );
        }
    }
    Ok(())
}

/// Write one report as JSON.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub async fn write_preview_perf_json(path: &Path, report: &PreviewPerfReport) -> Result<()> {
    let json = serde_json::to_vec_pretty(report)?;
    smol::fs::write(path, json).await?;
    Ok(())
}

/// Write every report as JSON on stdout.
///
/// # Errors
/// Returns an error if stdout cannot be written.
pub fn write_preview_perf_stdout_json(reports: &[PreviewPerfReport]) -> Result<()> {
    let json = serde_json::to_vec_pretty(&PreviewPerfOutput { reports })?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&json)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

/// Write every report as JSON to a file.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub async fn write_preview_perf_output_json(
    path: &Path,
    reports: &[PreviewPerfReport],
) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        smol::fs::create_dir_all(parent).await?;
    }
    smol::fs::write(
        path,
        serde_json::to_vec_pretty(&PreviewPerfOutput { reports })?,
    )
    .await?;
    Ok(())
}

/// Write one report as a Chrome trace.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub async fn write_preview_perf_trace(path: &Path, report: &PreviewPerfReport) -> Result<()> {
    #[derive(Serialize)]
    struct Trace<'a> {
        #[serde(rename = "traceEvents")]
        trace_events: Vec<TraceEvent<'a>>,
    }

    #[derive(Serialize)]
    struct TraceEvent<'a> {
        name: &'a str,
        cat: &'a str,
        ph: &'static str,
        ts: u64,
        dur: u64,
        pid: u32,
        tid: u32,
        args: BTreeMap<&'a str, u64>,
    }

    let mut trace_events = Vec::new();
    let mut ts = 0;
    for measurement in &report.measurements {
        for (name, dur) in [
            ("rebuild", measurement.phases.rebuild_mean_us),
            ("render", measurement.phases.render_mean_us),
            ("animation", measurement.phases.animation_mean_us),
            ("input", measurement.phases.input_mean_us),
        ] {
            let mut args = BTreeMap::new();
            args.insert("samples", measurement.samples);
            args.insert("p95_us", measurement.p95_us);
            trace_events.push(TraceEvent {
                name,
                cat: measurement.name.as_str(),
                ph: "X",
                ts,
                dur,
                pid: 1,
                tid: 1,
                args,
            });
            ts += dur;
        }
    }
    smol::fs::write(path, serde_json::to_vec_pretty(&Trace { trace_events })?).await?;
    Ok(())
}

/// Render every report into a standalone HTML page.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub async fn write_preview_perf_html(path: &Path, reports: &[PreviewPerfReport]) -> Result<()> {
    let report_cards = reports
        .iter()
        .map(render_preview_perf_report_html)
        .collect::<String>();
    let worst_p95 = reports
        .iter()
        .flat_map(|report| &report.measurements)
        .map(|measurement| measurement.p95_us)
        .max()
        .unwrap_or_default();
    let missed_120 = reports
        .iter()
        .flat_map(|report| &report.measurements)
        .map(|measurement| measurement.missed_120fps_frames)
        .sum::<u64>();
    let html = render_perf_template(&PerfPageView {
        worst_p95: micros_label(worst_p95),
        missed_120: missed_120.to_string(),
        reports: report_cards,
    });
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        smol::fs::create_dir_all(parent).await?;
    }
    smol::fs::write(path, html).await?;
    Ok(())
}

/// One SVG sample dot in a perf chart, with the data attributes the report's
/// hover inspector reads. `build`/`dispatch`/`finish` are present only for the
/// frame-timeline charts; `value` only for the single-metric trend charts.
#[derive(Default)]
struct PerfSampleView {
    cx: String,
    cy: String,
    frame: u64,
    total: String,
    gpu: String,
    render: String,
    rebuild: String,
    cpu: String,
    memory: String,
    fps: String,
    layers: u64,
    clip_layers: u64,
    clip_depth: u64,
    build: Option<String>,
    dispatch: Option<String>,
    finish: Option<String>,
    value: Option<String>,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/diagnosis.html", escape = "html")]
struct DiagnosisView {
    p95: String,
    mean: String,
    median: String,
    rendered_p95: String,
    rendered_frames: u64,
    idle_frames: u64,
    worst_frame: String,
    samples: u64,
    bottleneck_name: &'static str,
    bottleneck_mean: String,
    rebuild_ratio: String,
    rebuilt_frames: u64,
    cache_hit_ratio: String,
    cache_hits: u64,
    cache_misses: u64,
}

struct PhaseSegmentView {
    class: &'static str,
    width: String,
    name: &'static str,
    label: String,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/phase_stack.html", escape = "html")]
struct PhaseStackView {
    segments: Vec<PhaseSegmentView>,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/metric_chart.html", escape = "html")]
struct MetricChartView {
    title: String,
    min_label: String,
    max_label: String,
    line_class: String,
    points: String,
    samples: Vec<PerfSampleView>,
}

struct BudgetLineView {
    class: &'static str,
    y: String,
}

struct TimingChartView {
    min_label: String,
    max_label: String,
    budget_lines: Vec<BudgetLineView>,
    total_points: String,
    gpu_points: String,
    render_points: String,
    rebuild_points: String,
    samples: Vec<PerfSampleView>,
}

struct FpsChartView {
    min_label: String,
    max_label: String,
    budget_lines: Vec<BudgetLineView>,
    fps_points: String,
    samples: Vec<PerfSampleView>,
}

#[derive(Template)]
#[template(
    path = "src/templates/preview_perf/frame_timeline.html",
    escape = "html"
)]
struct FrameTimelineView {
    timing: TimingChartView,
    fps: FpsChartView,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/resources.html", escape = "html")]
struct ResourcesView {
    avg_cpu: String,
    max_cpu: String,
    max_memory: String,
    avg_gpu: String,
    max_gpu: String,
    avg_layers: String,
    max_layers: u64,
    avg_clip: String,
    max_clip_depth: u64,
    charts: Vec<String>,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/flamegraph.html", escape = "html")]
struct FlamegraphView {
    path: String,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/measurement.html", escape = "html")]
struct MeasurementView {
    name: String,
    budget_label: &'static str,
    diagnosis: String,
    phase_stack: String,
    frame_timeline: String,
    resources: String,
    flamegraph: String,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf/report_card.html", escape = "html")]
struct ReportCardView {
    target: String,
    measurements: String,
}

#[derive(Template)]
#[template(path = "src/templates/preview_perf_report.html", escape = "html")]
struct PerfPageView {
    worst_p95: String,
    missed_120: String,
    reports: String,
}

/// Renders a template, treating any rendering error as a bug (the templates are
/// compile-checked and the data is plain owned values, so this is infallible).
fn render_perf_template<T: Template>(template: &T) -> String {
    template
        .render()
        .expect("preview perf report template rendering is infallible")
}

/// Builds the chart-agnostic part of a sample dot (position + the data
/// attributes every chart exposes). Callers fill in the chart-specific
/// `build`/`dispatch`/`finish` (frame timeline) or `value` (metric trend).
fn perf_common_sample(frame: &PreviewPerfFrame, cx: f64, cy: f64) -> PerfSampleView {
    PerfSampleView {
        cx: format!("{cx:.2}"),
        cy: format!("{cy:.2}"),
        frame: frame.index,
        total: micros_label(frame.total_us),
        gpu: micros_label(frame.gpu_frame_us),
        render: micros_label(frame.render_us),
        rebuild: micros_label(frame.rebuild_us),
        cpu: format!("{:.1}%", frame.cpu_percent),
        memory: bytes_label(frame.memory_bytes),
        fps: fps_label(preview_perf_throughput_fps(frame)),
        layers: frame.scene_layers,
        clip_layers: frame.clip_layers,
        clip_depth: frame.max_clip_depth,
        ..PerfSampleView::default()
    }
}

fn render_preview_perf_report_html(report: &PreviewPerfReport) -> String {
    let measurements = report
        .measurements
        .iter()
        .map(|measurement| {
            render_preview_perf_measurement_html(measurement, report.flamegraph.as_deref())
        })
        .collect::<String>();
    render_perf_template(&ReportCardView {
        target: report.target.clone(),
        measurements,
    })
}

fn render_preview_perf_flamegraph_html(flamegraph: Option<&Path>) -> String {
    let Some(flamegraph) = flamegraph else {
        return String::new();
    };
    render_perf_template(&FlamegraphView {
        path: flamegraph.to_string_lossy().into_owned(),
    })
}

fn render_preview_perf_measurement_html(
    measurement: &PreviewPerfMeasurement,
    flamegraph: Option<&Path>,
) -> String {
    let diagnosis = render_preview_perf_diagnosis_html(measurement);
    let resources = render_preview_perf_resource_timeline_html(measurement);
    let frame_timeline = render_preview_perf_frame_timeline_html(measurement);
    let phase_stack = render_preview_perf_phase_stack_html(measurement);
    let flamegraph = render_preview_perf_flamegraph_html(flamegraph);
    render_perf_template(&MeasurementView {
        name: measurement.name.clone(),
        budget_label: preview_perf_budget_label(measurement),
        diagnosis,
        phase_stack,
        frame_timeline,
        resources,
        flamegraph,
    })
}

const fn preview_perf_budget_label(measurement: &PreviewPerfMeasurement) -> &'static str {
    if measurement.p95_us > 16_666 {
        "misses 60fps"
    } else if measurement.p95_us > 8_333 {
        "misses 120fps"
    } else {
        "120fps ready"
    }
}

fn render_preview_perf_diagnosis_html(measurement: &PreviewPerfMeasurement) -> String {
    let worst = measurement.frames.iter().max_by_key(|frame| frame.total_us);
    let bottleneck = preview_perf_bottleneck(measurement);
    let rebuild_ratio = ratio_percent(measurement.rebuilt_frames, measurement.samples);
    let cache_total = measurement
        .measurement_cache_hits
        .saturating_add(measurement.measurement_cache_misses);
    let cache_hit_ratio = ratio_percent(measurement.measurement_cache_hits, cache_total);
    let worst_frame = worst.map_or_else(
        || "none".to_string(),
        |frame| format!("frame {} / {}", frame.index, micros_label(frame.total_us)),
    );
    render_perf_template(&DiagnosisView {
        p95: micros_label(measurement.p95_us),
        mean: micros_label(measurement.mean_us),
        median: micros_label(measurement.median_us),
        rendered_p95: micros_label(measurement.rendered_p95_us),
        rendered_frames: measurement.rendered_frames,
        idle_frames: measurement.idle_frames,
        worst_frame,
        samples: measurement.samples,
        bottleneck_name: bottleneck.name,
        bottleneck_mean: micros_label(bottleneck.mean_us),
        rebuild_ratio: format!("{rebuild_ratio:.1}"),
        rebuilt_frames: measurement.rebuilt_frames,
        cache_hit_ratio: format!("{cache_hit_ratio:.1}"),
        cache_hits: measurement.measurement_cache_hits,
        cache_misses: measurement.measurement_cache_misses,
    })
}

struct PreviewPerfBottleneck {
    name: &'static str,
    mean_us: u64,
}

fn preview_perf_bottleneck(measurement: &PreviewPerfMeasurement) -> PreviewPerfBottleneck {
    [
        ("render", measurement.phases.render_mean_us),
        ("rebuild", measurement.phases.rebuild_mean_us),
        ("build content", measurement.phases.build_content_mean_us),
        ("scene dispatch", measurement.phases.scene_dispatch_mean_us),
        ("scene finish", measurement.phases.scene_finish_mean_us),
        ("animation", measurement.phases.animation_mean_us),
        ("input", measurement.phases.input_mean_us),
    ]
    .into_iter()
    .max_by_key(|(_, value)| *value)
    .map(|(name, mean_us)| PreviewPerfBottleneck { name, mean_us })
    .expect("preview perf bottleneck phase list is non-empty")
}

fn render_preview_perf_resource_timeline_html(measurement: &PreviewPerfMeasurement) -> String {
    let Some(summary) = resource_summary(measurement) else {
        return String::new();
    };
    let cpu_chart = render_preview_perf_metric_chart_html(
        "CPU usage",
        "line-cpu",
        &measurement.frames,
        |frame| frame.cpu_percent,
        |value| format!("{value:.1}%"),
    );
    let memory_chart = render_preview_perf_metric_chart_html(
        "Memory",
        "line-memory",
        &measurement.frames,
        |frame| metric_to_f64(frame.memory_bytes) / 1_048_576.0,
        |value| format!("{value:.1} MiB"),
    );
    let gpu_chart = render_preview_perf_metric_chart_html(
        "GPU pipeline",
        "line-gpu",
        &measurement.frames,
        |frame| metric_to_f64(frame.gpu_frame_us),
        |value| micros_label(rounded_metric_to_u64(value)),
    );
    let layer_chart = render_preview_perf_metric_chart_html(
        "Compositor layers",
        "line-layers",
        &measurement.frames,
        |frame| metric_to_f64(frame.scene_layers),
        |value| format!("{value:.0}"),
    );
    let clip_chart = render_preview_perf_metric_chart_html(
        "Clip layers",
        "line-clip",
        &measurement.frames,
        |frame| metric_to_f64(frame.clip_layers),
        |value| format!("{value:.0}"),
    );
    render_perf_template(&ResourcesView {
        avg_cpu: format!("{:.1}", summary.avg_cpu_percent),
        max_cpu: format!("{:.1}", summary.max_cpu_percent),
        max_memory: bytes_label(summary.max_memory_bytes),
        avg_gpu: micros_label(summary.avg_gpu_frame_us),
        max_gpu: micros_label(summary.max_gpu_frame_us),
        avg_layers: format!("{:.1}", summary.avg_scene_layers),
        max_layers: summary.max_scene_layers,
        avg_clip: format!("{:.1}", summary.avg_clip_layers),
        max_clip_depth: summary.max_clip_depth,
        charts: vec![cpu_chart, memory_chart, gpu_chart, layer_chart, clip_chart],
    })
}

fn render_preview_perf_frame_timeline_html(measurement: &PreviewPerfMeasurement) -> String {
    if measurement.frames.len() < 2 {
        return String::new();
    }
    let timing_values = measurement
        .frames
        .iter()
        .flat_map(|frame| {
            [
                metric_to_f64(frame.total_us),
                metric_to_f64(frame.render_us),
                metric_to_f64(frame.rebuild_us),
                metric_to_f64(frame.gpu_frame_us),
            ]
        })
        .collect::<Vec<_>>();
    let timing_scale = PreviewPerfChartScale::new(timing_values.iter().copied());
    let fps_values = measurement
        .frames
        .iter()
        .map(preview_perf_throughput_fps)
        .map(|value| value.min(1_000.0))
        .collect::<Vec<_>>();
    let fps_scale = PreviewPerfChartScale::new(fps_values.iter().copied());
    let fps_points =
        render_preview_perf_float_polyline_points(&measurement.frames, fps_scale, |frame| {
            preview_perf_throughput_fps(frame).min(1_000.0)
        });
    let total_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.total_us)
        });
    let render_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.render_us)
        });
    let rebuild_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.rebuild_us)
        });
    let gpu_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.gpu_frame_us)
        });
    let timing_samples = measurement
        .frames
        .iter()
        .map(|frame| {
            let cx = frame_chart_x(frame.index, measurement.frames.len());
            let cy = timing_scale.y(metric_to_f64(frame.total_us));
            PerfSampleView {
                build: Some(micros_label(frame.build_content_us)),
                dispatch: Some(micros_label(frame.scene_dispatch_us)),
                finish: Some(micros_label(frame.scene_finish_us)),
                ..perf_common_sample(frame, cx, cy)
            }
        })
        .collect();
    let fps_samples = measurement
        .frames
        .iter()
        .map(|frame| {
            let cx = frame_chart_x(frame.index, measurement.frames.len());
            let cy = fps_scale.y(preview_perf_throughput_fps(frame).min(1_000.0));
            PerfSampleView {
                build: Some(micros_label(frame.build_content_us)),
                dispatch: Some(micros_label(frame.scene_dispatch_us)),
                finish: Some(micros_label(frame.scene_finish_us)),
                ..perf_common_sample(frame, cx, cy)
            }
        })
        .collect();
    render_perf_template(&FrameTimelineView {
        timing: TimingChartView {
            min_label: micros_label(rounded_metric_to_u64(timing_scale.min)),
            max_label: micros_label(rounded_metric_to_u64(timing_scale.max)),
            budget_lines: render_preview_perf_budget_lines(timing_scale),
            total_points,
            gpu_points,
            render_points,
            rebuild_points,
            samples: timing_samples,
        },
        fps: FpsChartView {
            min_label: fps_label(fps_scale.min),
            max_label: fps_label(fps_scale.max),
            budget_lines: render_preview_perf_fps_budget_lines(fps_scale),
            fps_points,
            samples: fps_samples,
        },
    })
}

fn render_preview_perf_phase_stack_html(measurement: &PreviewPerfMeasurement) -> String {
    let phases = [
        ("input", measurement.phases.input_mean_us, "phase-input"),
        (
            "animation",
            measurement.phases.animation_mean_us,
            "phase-animation",
        ),
        (
            "build",
            measurement.phases.build_content_mean_us,
            "phase-rebuild",
        ),
        (
            "dispatch",
            measurement.phases.scene_dispatch_mean_us,
            "phase-rebuild",
        ),
        (
            "finish",
            measurement.phases.scene_finish_mean_us,
            "phase-rebuild",
        ),
        ("render", measurement.phases.render_mean_us, "phase-render"),
    ];
    let total = phases
        .iter()
        .map(|(_, value, _)| *value)
        .sum::<u64>()
        .max(1);
    let segments = phases
        .iter()
        .map(|(name, value, class)| PhaseSegmentView {
            class,
            width: format!("{:.2}", ratio_percent(*value, total).max(0.5)),
            name,
            label: micros_label(*value),
        })
        .collect();
    render_perf_template(&PhaseStackView { segments })
}

fn render_preview_perf_float_polyline_points(
    frames: &[PreviewPerfFrame],
    scale: PreviewPerfChartScale,
    value: impl Fn(&PreviewPerfFrame) -> f64,
) -> String {
    frames
        .iter()
        .map(|frame| {
            format!(
                "{:.2},{:.2}",
                frame_chart_x(frame.index, frames.len()),
                scale.y(value(frame))
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_preview_perf_metric_chart_html(
    title: &str,
    line_class: &str,
    frames: &[PreviewPerfFrame],
    value: impl Fn(&PreviewPerfFrame) -> f64,
    label: impl Fn(f64) -> String,
) -> String {
    if frames.len() < 2 {
        return String::new();
    }
    let values = frames.iter().map(&value).collect::<Vec<_>>();
    let actual_min = values
        .iter()
        .copied()
        .reduce(f64::min)
        .expect("preview perf metric chart has at least two frames");
    let actual_max = values
        .iter()
        .copied()
        .reduce(f64::max)
        .expect("preview perf metric chart has at least two frames");
    let scale = PreviewPerfChartScale::new(values.iter().copied());
    let points = render_preview_perf_float_polyline_points(frames, scale, &value);
    let samples = frames
        .iter()
        .map(|frame| {
            let current_value = value(frame);
            let cx = frame_chart_x(frame.index, frames.len());
            let cy = scale.y(current_value);
            PerfSampleView {
                value: Some(label(current_value)),
                ..perf_common_sample(frame, cx, cy)
            }
        })
        .collect();
    render_perf_template(&MetricChartView {
        title: title.to_owned(),
        min_label: label(actual_min),
        max_label: label(actual_max),
        line_class: line_class.to_owned(),
        points,
        samples,
    })
}

fn frame_chart_x(index: u64, frame_count: usize) -> f64 {
    if frame_count <= 1 {
        return 6.0;
    }
    (metric_to_f64(index) / sample_count_to_f64(frame_count - 1)).mul_add(88.0, 6.0)
}

#[derive(Clone, Copy)]
struct PreviewPerfChartScale {
    min: f64,
    max: f64,
}

impl PreviewPerfChartScale {
    fn new(values: impl IntoIterator<Item = f64>) -> Self {
        let mut values = values.into_iter();
        let first = values.next().unwrap_or(0.0);
        let (mut min, mut max) = (first, first);
        for value in values {
            min = min.min(value);
            max = max.max(value);
        }
        let span = max - min;
        let padding = if span <= f64::EPSILON {
            max.abs().mul_add(0.02, 1.0)
        } else {
            span * 0.12
        };
        Self {
            min: (min - padding).max(0.0),
            max: max + padding,
        }
    }

    fn contains(self, value: f64) -> bool {
        (self.min..=self.max).contains(&value)
    }

    fn y(self, value: f64) -> f64 {
        if self.max <= self.min {
            return 50.0;
        }
        let clamped = value.clamp(self.min, self.max);
        ((clamped - self.min) / (self.max - self.min)).mul_add(-84.0, 92.0)
    }
}

fn render_preview_perf_budget_lines(scale: PreviewPerfChartScale) -> Vec<BudgetLineView> {
    [(8_333.0, "budget-120"), (16_666.0, "budget-60")]
        .into_iter()
        .filter(|(value, _)| scale.contains(*value))
        .map(|(value, class)| BudgetLineView {
            class,
            y: format!("{:.2}", scale.y(value)),
        })
        .collect()
}

fn render_preview_perf_fps_budget_lines(scale: PreviewPerfChartScale) -> Vec<BudgetLineView> {
    [(120.0, "budget-120"), (60.0, "budget-60")]
        .into_iter()
        .filter(|(value, _)| scale.contains(*value))
        .map(|(value, class)| BudgetLineView {
            class,
            y: format!("{:.2}", scale.y(value)),
        })
        .collect()
}

pub(crate) fn preview_perf_throughput_fps(frame: &PreviewPerfFrame) -> f64 {
    1_000_000.0 / metric_to_f64(frame.total_us.max(1))
}

pub(crate) fn fps_label(value: f64) -> String {
    if value >= 1_000.0 {
        ">=1000fps".to_string()
    } else {
        format!("{value:.1}fps")
    }
}

pub(crate) fn ratio_percent(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    (metric_to_f64(numerator) / metric_to_f64(denominator)) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_circle_fixture() -> PerfSampleView {
        PerfSampleView {
            cx: "6.00".to_owned(),
            cy: "50.00".to_owned(),
            frame: 0,
            total: "10ms".to_owned(),
            gpu: "4ms".to_owned(),
            render: "3ms".to_owned(),
            rebuild: "2ms".to_owned(),
            cpu: "5.0%".to_owned(),
            memory: "1.0 MiB".to_owned(),
            fps: "100.0fps".to_owned(),
            layers: 3,
            clip_layers: 1,
            clip_depth: 2,
            ..PerfSampleView::default()
        }
    }

    #[test]
    fn perf_diagnosis_template_is_faithful() {
        let html = render_perf_template(&DiagnosisView {
            p95: "12ms".to_owned(),
            mean: "8ms".to_owned(),
            median: "7ms".to_owned(),
            rendered_p95: "11ms".to_owned(),
            rendered_frames: 90,
            idle_frames: 10,
            worst_frame: "frame 3 / 20ms".to_owned(),
            samples: 100,
            bottleneck_name: "render",
            bottleneck_mean: "5ms".to_owned(),
            rebuild_ratio: "12.5".to_owned(),
            rebuilt_frames: 12,
            cache_hit_ratio: "98.0".to_owned(),
            cache_hits: 980,
            cache_misses: 20,
        });
        assert!(html.contains("<section class=\"diagnosis\">"));
        assert!(html.contains(
            "<div><span>p95</span><strong>12ms</strong><small>mean 8ms / median 7ms</small></div>"
        ));
        assert!(html.contains(
            "<div><span>rebuild pressure</span><strong>12.5%</strong><small>12/100 frames</small></div>"
        ));
        assert!(html.contains(
            "<div><span>layout cache</span><strong>98.0% hit</strong><small>980 hits / 20 misses</small></div>"
        ));
    }

    #[test]
    fn perf_metric_chart_circle_carries_value_not_build() {
        let html = render_perf_template(&MetricChartView {
            title: "CPU usage".to_owned(),
            min_label: "1.0%".to_owned(),
            max_label: "9.0%".to_owned(),
            line_class: "line-cpu".to_owned(),
            points: "6.00,50.00 94.00,20.00".to_owned(),
            samples: vec![PerfSampleView {
                value: Some("5.0%".to_owned()),
                ..sample_circle_fixture()
            }],
        });
        assert!(html.contains(
            "<polyline class=\"line-cpu\" points=\"6.00,50.00 94.00,20.00\"></polyline>"
        ));
        assert!(html.contains("aria-label=\"CPU usage trend\""));
        assert!(html.contains("data-value=\"5.0%\""));
        assert!(html.contains("data-cpu=\"5.0%\""));
        // Metric-trend circles do not carry the frame-timeline-only phase attrs.
        assert!(!html.contains("data-build"));
    }

    #[test]
    fn perf_frame_timeline_circle_carries_build_not_value() {
        let timeline_sample = || PerfSampleView {
            build: Some("1ms".to_owned()),
            dispatch: Some("2ms".to_owned()),
            finish: Some("3ms".to_owned()),
            ..sample_circle_fixture()
        };
        let html = render_perf_template(&FrameTimelineView {
            timing: TimingChartView {
                min_label: "1ms".to_owned(),
                max_label: "20ms".to_owned(),
                budget_lines: vec![BudgetLineView {
                    class: "budget-120",
                    y: "30.00".to_owned(),
                }],
                total_points: "6.00,50.00".to_owned(),
                gpu_points: "6.00,60.00".to_owned(),
                render_points: "6.00,70.00".to_owned(),
                rebuild_points: "6.00,80.00".to_owned(),
                samples: vec![timeline_sample()],
            },
            fps: FpsChartView {
                min_label: "50.0fps".to_owned(),
                max_label: "120.0fps".to_owned(),
                budget_lines: Vec::new(),
                fps_points: "6.00,40.00".to_owned(),
                samples: vec![timeline_sample()],
            },
        });
        assert!(html.contains("<section class=\"timeline-grid\">"));
        assert!(html.contains(
            "<line class=\"budget budget-120\" x1=\"6\" y1=\"30.00\" x2=\"94\" y2=\"30.00\"></line>"
        ));
        assert!(html.contains("<polyline class=\"line-total\" points=\"6.00,50.00\"></polyline>"));
        assert!(html.contains("<polyline class=\"line-fps\" points=\"6.00,40.00\"></polyline>"));
        assert!(html.contains("data-build=\"1ms\" data-dispatch=\"2ms\" data-finish=\"3ms\""));
        // Frame-timeline circles do not carry the metric-trend-only value attr.
        assert!(!html.contains("data-value"));
    }

    #[test]
    fn perf_page_template_embeds_summary_and_reports() {
        let html = render_perf_template(&PerfPageView {
            worst_p95: "16ms".to_owned(),
            missed_120: "4".to_owned(),
            reports: "<section class=\"report\"><h2>demo</h2></section>".to_owned(),
        });
        assert!(html.contains("<title>WaterUI Preview Perf</title>"));
        assert!(html.contains("<strong>16ms</strong>"));
        assert!(html.contains("<strong>4</strong>"));
        assert!(html.contains("<section class=\"report\"><h2>demo</h2></section>"));
        assert!(html.contains("formatSample"));
    }
}
