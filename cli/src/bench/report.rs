//! Rendering for `water bench` reports.
//!
//! Budget evaluation happens inside `waterui-testing` while the benches run;
//! this module only aggregates the collected [`BenchReport`]s and renders them
//! as JSON, GitHub-Action-benchmark JSON, or a standalone HTML page. The
//! terminal layer prints the human summary itself.

use std::io::Write as _;
use std::path::Path;

use askama::Template;
use color_eyre::eyre::Result;
use serde::Serialize;

use waterui_preview_protocol::bench::{BenchReport, PerfFrame, PerfMeasurement};

/// `crate/bench` label identifying one report in every output format.
#[must_use]
pub fn bench_target_label(report: &BenchReport) -> String {
    format!("{}/{}", report.crate_name, report.bench_name)
}

/// Render a microsecond count the way the bench report displays it.
#[must_use]
pub fn micros_label(value: u64) -> String {
    format!("{value}us")
}

#[expect(
    clippy::cast_precision_loss,
    reason = "bench charts intentionally project integer telemetry into floating-point display coordinates"
)]
pub(crate) const fn metric_to_f64(value: u64) -> f64 {
    value as f64
}

#[expect(
    clippy::cast_precision_loss,
    reason = "bench chart sample counts are converted only for display averages and coordinates"
)]
pub(crate) const fn sample_count_to_f64(value: usize) -> f64 {
    value as f64
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bench chart scales contain finite non-negative telemetry and labels use rounded integers"
)]
pub(crate) fn rounded_metric_to_u64(value: f64) -> u64 {
    assert!(
        value.is_finite() && value >= 0.0 && value <= metric_to_f64(u64::MAX),
        "bench chart metric must be finite, non-negative, and fit into u64"
    );
    value.round() as u64
}

/// Render a byte count the way the bench report displays it.
#[must_use]
pub fn bytes_label(value: u64) -> String {
    const MIB: f64 = 1_048_576.0;
    format!("{:.1}MiB", metric_to_f64(value) / MIB)
}

/// CPU, memory, GPU and scene-complexity aggregates over a measurement's frames.
#[derive(Debug, Clone, Copy)]
pub struct BenchResourceSummary {
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
pub fn resource_summary(measurement: &PerfMeasurement) -> Option<BenchResourceSummary> {
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
        / u64::try_from(measurement.frames.len()).expect("bench sample count should fit u64");
    Some(BenchResourceSummary {
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

#[derive(Debug, Serialize)]
struct BenchOutput<'a> {
    reports: &'a [BenchReport],
}

/// Write every report as JSON on stdout.
///
/// # Errors
/// Returns an error if stdout cannot be written.
pub fn write_bench_stdout_json(reports: &[BenchReport]) -> Result<()> {
    let json = serde_json::to_vec_pretty(&BenchOutput { reports })?;
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
pub async fn write_bench_output_json(path: &Path, reports: &[BenchReport]) -> Result<()> {
    create_parent_dir(path).await?;
    smol::fs::write(path, serde_json::to_vec_pretty(&BenchOutput { reports })?).await?;
    Ok(())
}

#[derive(Debug, Serialize)]
struct GhaBenchmarkEntry {
    name: String,
    unit: &'static str,
    value: u64,
}

/// Write the `github-action-benchmark` `customSmallerIsBetter` JSON.
///
/// Every measurement contributes its frame p95 and frame mean in
/// microseconds, named `<crate>/<bench>/<measurement> <metric>`.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub async fn write_bench_gha_json(path: &Path, reports: &[BenchReport]) -> Result<()> {
    let entries = reports
        .iter()
        .flat_map(|report| {
            let target = bench_target_label(report);
            report.measurements.iter().flat_map(move |measurement| {
                [
                    GhaBenchmarkEntry {
                        name: format!("{target}/{} frame p95", measurement.name),
                        unit: "us",
                        value: measurement.p95_us,
                    },
                    GhaBenchmarkEntry {
                        name: format!("{target}/{} frame mean", measurement.name),
                        unit: "us",
                        value: measurement.mean_us,
                    },
                ]
            })
        })
        .collect::<Vec<_>>();
    create_parent_dir(path).await?;
    smol::fs::write(path, serde_json::to_vec_pretty(&entries)?).await?;
    Ok(())
}

/// Render every report into a standalone HTML page.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub async fn write_bench_html(path: &Path, reports: &[BenchReport]) -> Result<()> {
    let report_cards = reports
        .iter()
        .map(render_bench_report_html)
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
    let html = render_bench_template(&BenchPageView {
        worst_p95: micros_label(worst_p95),
        missed_120: missed_120.to_string(),
        reports: report_cards,
    });
    create_parent_dir(path).await?;
    smol::fs::write(path, html).await?;
    Ok(())
}

async fn create_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        smol::fs::create_dir_all(parent).await?;
    }
    Ok(())
}

/// One SVG sample dot in a bench chart, with the data attributes the report's
/// hover inspector reads. `build`/`dispatch`/`finish` are present only for the
/// frame-timeline charts; `value` only for the single-metric trend charts.
#[derive(Default)]
struct BenchSampleView {
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
#[template(path = "src/templates/bench/diagnosis.html", escape = "html")]
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
#[template(path = "src/templates/bench/phase_stack.html", escape = "html")]
struct PhaseStackView {
    segments: Vec<PhaseSegmentView>,
}

#[derive(Template)]
#[template(path = "src/templates/bench/metric_chart.html", escape = "html")]
struct MetricChartView {
    title: String,
    min_label: String,
    max_label: String,
    line_class: String,
    points: String,
    samples: Vec<BenchSampleView>,
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
    samples: Vec<BenchSampleView>,
}

struct FpsChartView {
    min_label: String,
    max_label: String,
    budget_lines: Vec<BudgetLineView>,
    fps_points: String,
    samples: Vec<BenchSampleView>,
}

#[derive(Template)]
#[template(path = "src/templates/bench/frame_timeline.html", escape = "html")]
struct FrameTimelineView {
    timing: TimingChartView,
    fps: FpsChartView,
}

#[derive(Template)]
#[template(path = "src/templates/bench/resources.html", escape = "html")]
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
#[template(path = "src/templates/bench/measurement.html", escape = "html")]
struct MeasurementView {
    name: String,
    budget_label: &'static str,
    diagnosis: String,
    phase_stack: String,
    frame_timeline: String,
    resources: String,
}

#[derive(Template)]
#[template(path = "src/templates/bench/report_card.html", escape = "html")]
struct ReportCardView {
    target: String,
    measurements: String,
}

#[derive(Template)]
#[template(path = "src/templates/bench_report.html", escape = "html")]
struct BenchPageView {
    worst_p95: String,
    missed_120: String,
    reports: String,
}

/// Renders a template, treating any rendering error as a bug (the templates are
/// compile-checked and the data is plain owned values, so this is infallible).
fn render_bench_template<T: Template>(template: &T) -> String {
    template
        .render()
        .expect("bench report template rendering is infallible")
}

/// Builds the chart-agnostic part of a sample dot (position + the data
/// attributes every chart exposes). Callers fill in the chart-specific
/// `build`/`dispatch`/`finish` (frame timeline) or `value` (metric trend).
fn bench_common_sample(frame: &PerfFrame, cx: f64, cy: f64) -> BenchSampleView {
    BenchSampleView {
        cx: format!("{cx:.2}"),
        cy: format!("{cy:.2}"),
        frame: frame.index,
        total: micros_label(frame.total_us),
        gpu: micros_label(frame.gpu_frame_us),
        render: micros_label(frame.render_us),
        rebuild: micros_label(frame.rebuild_us),
        cpu: format!("{:.1}%", frame.cpu_percent),
        memory: bytes_label(frame.memory_bytes),
        fps: fps_label(bench_throughput_fps(frame)),
        layers: frame.scene_layers,
        clip_layers: frame.clip_layers,
        clip_depth: frame.max_clip_depth,
        ..BenchSampleView::default()
    }
}

fn render_bench_report_html(report: &BenchReport) -> String {
    let measurements = report
        .measurements
        .iter()
        .map(render_bench_measurement_html)
        .collect::<String>();
    render_bench_template(&ReportCardView {
        target: bench_target_label(report),
        measurements,
    })
}

fn render_bench_measurement_html(measurement: &PerfMeasurement) -> String {
    let diagnosis = render_bench_diagnosis_html(measurement);
    let resources = render_bench_resource_timeline_html(measurement);
    let frame_timeline = render_bench_frame_timeline_html(measurement);
    let phase_stack = render_bench_phase_stack_html(measurement);
    render_bench_template(&MeasurementView {
        name: measurement.name.clone(),
        budget_label: bench_budget_label(measurement),
        diagnosis,
        phase_stack,
        frame_timeline,
        resources,
    })
}

const fn bench_budget_label(measurement: &PerfMeasurement) -> &'static str {
    if measurement.p95_us > 16_666 {
        "misses 60fps"
    } else if measurement.p95_us > 8_333 {
        "misses 120fps"
    } else {
        "120fps ready"
    }
}

fn render_bench_diagnosis_html(measurement: &PerfMeasurement) -> String {
    let worst = measurement.frames.iter().max_by_key(|frame| frame.total_us);
    let bottleneck = bench_bottleneck(measurement);
    let rebuild_ratio = ratio_percent(measurement.rebuilt_frames, measurement.samples);
    let cache_total = measurement
        .measurement_cache_hits
        .saturating_add(measurement.measurement_cache_misses);
    let cache_hit_ratio = ratio_percent(measurement.measurement_cache_hits, cache_total);
    let worst_frame = worst.map_or_else(
        || "none".to_string(),
        |frame| format!("frame {} / {}", frame.index, micros_label(frame.total_us)),
    );
    render_bench_template(&DiagnosisView {
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

struct BenchBottleneck {
    name: &'static str,
    mean_us: u64,
}

fn bench_bottleneck(measurement: &PerfMeasurement) -> BenchBottleneck {
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
    .map(|(name, mean_us)| BenchBottleneck { name, mean_us })
    .expect("bench bottleneck phase list is non-empty")
}

fn render_bench_resource_timeline_html(measurement: &PerfMeasurement) -> String {
    let Some(summary) = resource_summary(measurement) else {
        return String::new();
    };
    let cpu_chart = render_bench_metric_chart_html(
        "CPU usage",
        "line-cpu",
        &measurement.frames,
        |frame| frame.cpu_percent,
        |value| format!("{value:.1}%"),
    );
    let memory_chart = render_bench_metric_chart_html(
        "Memory",
        "line-memory",
        &measurement.frames,
        |frame| metric_to_f64(frame.memory_bytes) / 1_048_576.0,
        |value| format!("{value:.1} MiB"),
    );
    let gpu_chart = render_bench_metric_chart_html(
        "GPU pipeline",
        "line-gpu",
        &measurement.frames,
        |frame| metric_to_f64(frame.gpu_frame_us),
        |value| micros_label(rounded_metric_to_u64(value)),
    );
    let layer_chart = render_bench_metric_chart_html(
        "Compositor layers",
        "line-layers",
        &measurement.frames,
        |frame| metric_to_f64(frame.scene_layers),
        |value| format!("{value:.0}"),
    );
    let clip_chart = render_bench_metric_chart_html(
        "Clip layers",
        "line-clip",
        &measurement.frames,
        |frame| metric_to_f64(frame.clip_layers),
        |value| format!("{value:.0}"),
    );
    render_bench_template(&ResourcesView {
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

fn render_bench_frame_timeline_html(measurement: &PerfMeasurement) -> String {
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
    let timing_scale = BenchChartScale::new(timing_values.iter().copied());
    let fps_values = measurement
        .frames
        .iter()
        .map(bench_throughput_fps)
        .map(|value| value.min(1_000.0))
        .collect::<Vec<_>>();
    let fps_scale = BenchChartScale::new(fps_values.iter().copied());
    let fps_points = render_bench_float_polyline_points(&measurement.frames, fps_scale, |frame| {
        bench_throughput_fps(frame).min(1_000.0)
    });
    let total_points =
        render_bench_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.total_us)
        });
    let render_points =
        render_bench_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.render_us)
        });
    let rebuild_points =
        render_bench_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.rebuild_us)
        });
    let gpu_points =
        render_bench_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            metric_to_f64(frame.gpu_frame_us)
        });
    let timing_samples = measurement
        .frames
        .iter()
        .map(|frame| {
            let cx = frame_chart_x(frame.index, measurement.frames.len());
            let cy = timing_scale.y(metric_to_f64(frame.total_us));
            BenchSampleView {
                build: Some(micros_label(frame.build_content_us)),
                dispatch: Some(micros_label(frame.scene_dispatch_us)),
                finish: Some(micros_label(frame.scene_finish_us)),
                ..bench_common_sample(frame, cx, cy)
            }
        })
        .collect();
    let fps_samples = measurement
        .frames
        .iter()
        .map(|frame| {
            let cx = frame_chart_x(frame.index, measurement.frames.len());
            let cy = fps_scale.y(bench_throughput_fps(frame).min(1_000.0));
            BenchSampleView {
                build: Some(micros_label(frame.build_content_us)),
                dispatch: Some(micros_label(frame.scene_dispatch_us)),
                finish: Some(micros_label(frame.scene_finish_us)),
                ..bench_common_sample(frame, cx, cy)
            }
        })
        .collect();
    render_bench_template(&FrameTimelineView {
        timing: TimingChartView {
            min_label: micros_label(rounded_metric_to_u64(timing_scale.min)),
            max_label: micros_label(rounded_metric_to_u64(timing_scale.max)),
            budget_lines: render_bench_budget_lines(timing_scale),
            total_points,
            gpu_points,
            render_points,
            rebuild_points,
            samples: timing_samples,
        },
        fps: FpsChartView {
            min_label: fps_label(fps_scale.min),
            max_label: fps_label(fps_scale.max),
            budget_lines: render_bench_fps_budget_lines(fps_scale),
            fps_points,
            samples: fps_samples,
        },
    })
}

fn render_bench_phase_stack_html(measurement: &PerfMeasurement) -> String {
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
    render_bench_template(&PhaseStackView { segments })
}

fn render_bench_float_polyline_points(
    frames: &[PerfFrame],
    scale: BenchChartScale,
    value: impl Fn(&PerfFrame) -> f64,
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

fn render_bench_metric_chart_html(
    title: &str,
    line_class: &str,
    frames: &[PerfFrame],
    value: impl Fn(&PerfFrame) -> f64,
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
        .expect("bench metric chart has at least two frames");
    let actual_max = values
        .iter()
        .copied()
        .reduce(f64::max)
        .expect("bench metric chart has at least two frames");
    let scale = BenchChartScale::new(values.iter().copied());
    let points = render_bench_float_polyline_points(frames, scale, &value);
    let samples = frames
        .iter()
        .map(|frame| {
            let current_value = value(frame);
            let cx = frame_chart_x(frame.index, frames.len());
            let cy = scale.y(current_value);
            BenchSampleView {
                value: Some(label(current_value)),
                ..bench_common_sample(frame, cx, cy)
            }
        })
        .collect();
    render_bench_template(&MetricChartView {
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
struct BenchChartScale {
    min: f64,
    max: f64,
}

impl BenchChartScale {
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

fn render_bench_budget_lines(scale: BenchChartScale) -> Vec<BudgetLineView> {
    [(8_333.0, "budget-120"), (16_666.0, "budget-60")]
        .into_iter()
        .filter(|(value, _)| scale.contains(*value))
        .map(|(value, class)| BudgetLineView {
            class,
            y: format!("{:.2}", scale.y(value)),
        })
        .collect()
}

fn render_bench_fps_budget_lines(scale: BenchChartScale) -> Vec<BudgetLineView> {
    [(120.0, "budget-120"), (60.0, "budget-60")]
        .into_iter()
        .filter(|(value, _)| scale.contains(*value))
        .map(|(value, class)| BudgetLineView {
            class,
            y: format!("{:.2}", scale.y(value)),
        })
        .collect()
}

pub(crate) fn bench_throughput_fps(frame: &PerfFrame) -> f64 {
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

    fn sample_circle_fixture() -> BenchSampleView {
        BenchSampleView {
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
            ..BenchSampleView::default()
        }
    }

    #[test]
    fn bench_diagnosis_template_is_faithful() {
        let html = render_bench_template(&DiagnosisView {
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
    fn bench_metric_chart_circle_carries_value_not_build() {
        let html = render_bench_template(&MetricChartView {
            title: "CPU usage".to_owned(),
            min_label: "1.0%".to_owned(),
            max_label: "9.0%".to_owned(),
            line_class: "line-cpu".to_owned(),
            points: "6.00,50.00 94.00,20.00".to_owned(),
            samples: vec![BenchSampleView {
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
    fn bench_frame_timeline_circle_carries_build_not_value() {
        let timeline_sample = || BenchSampleView {
            build: Some("1ms".to_owned()),
            dispatch: Some("2ms".to_owned()),
            finish: Some("3ms".to_owned()),
            ..sample_circle_fixture()
        };
        let html = render_bench_template(&FrameTimelineView {
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
    fn bench_page_template_embeds_summary_and_reports() {
        let html = render_bench_template(&BenchPageView {
            worst_p95: "16ms".to_owned(),
            missed_120: "4".to_owned(),
            reports: "<section class=\"report\"><h2>demo</h2></section>".to_owned(),
        });
        assert!(html.contains("<title>WaterUI Bench Report</title>"));
        assert!(html.contains("<strong>16ms</strong>"));
        assert!(html.contains("<strong>4</strong>"));
        assert!(html.contains("<section class=\"report\"><h2>demo</h2></section>"));
        assert!(html.contains("formatSample"));
    }
}
