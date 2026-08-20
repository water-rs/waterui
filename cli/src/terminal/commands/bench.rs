//! `water bench` command implementation.
//!
//! Thin terminal front end: argument parsing, format selection, and the human
//! summary. The bench engine itself lives in `waterui_cli::bench`.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell::Shell;
use crate::{header, note, success};
use waterui_cli::bench::report::{
    bench_target_label, micros_label, write_bench_gha_json, write_bench_html,
    write_bench_output_json, write_bench_stdout_json,
};
use waterui_cli::bench::{BenchRunOptions, BenchSuiteRun, run_bench_suite};
use waterui_preview_protocol::bench::{BenchBudgets, BenchReport, BenchRunConfig, PerfMeasurement};

/// Arguments for the bench command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Substring narrowing which benches run, matched against the bench name.
    filter: Option<String>,

    /// Project or crate directory containing the benches (defaults to the
    /// current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Recorded frame count per measurement.
    #[arg(long, default_value_t = 120)]
    samples: u32,

    /// Warmup frame count before sampling.
    #[arg(long, default_value_t = 10)]
    warmups: u32,

    /// Independent measurement repetitions.
    #[arg(long, default_value_t = 7)]
    repetitions: u32,

    /// Directory keeping the per-bench report JSON files (a temporary
    /// directory is used when unset).
    #[arg(long)]
    report_dir: Option<PathBuf>,

    /// Presentation mode for bench results.
    #[arg(long, value_enum, default_value_t = BenchOutputFormat::Human)]
    format: BenchOutputFormat,

    /// Output path for `--format json` or `--format html`.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Write `github-action-benchmark` `customSmallerIsBetter` JSON to PATH.
    #[arg(long)]
    gha: Option<PathBuf>,

    /// Cap every bench's p95 frame-time budget, in microseconds (tighter than
    /// the attribute budget wins).
    #[arg(long)]
    max_p95_us: Option<u64>,

    /// Cap every bench's mean frame-time budget, in microseconds.
    #[arg(long)]
    max_mean_us: Option<u64>,

    /// Cap every bench's rebuild-ratio budget (0.0..=1.0).
    #[arg(long)]
    max_rebuild_ratio: Option<f64>,

    /// Cap every bench's compositor scene-layer budget.
    #[arg(long)]
    max_scene_layers: Option<u64>,

    /// Cap every bench's embedded GPU-surface-layer budget.
    #[arg(long)]
    max_gpu_surface_layers: Option<u64>,

    /// Cap every bench's Vello clip-layer budget.
    #[arg(long)]
    max_clip_layers: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BenchOutputFormat {
    /// Human-friendly terminal summary.
    Human,
    /// Structured JSON written to stdout or `--output`.
    Json,
    /// Standalone visual HTML report written to `--output`.
    Html,
}

/// Run the bench command.
///
/// # Errors
/// Returns an error if the bench suite cannot run, a report cannot be
/// written, or the nextest run failed (which includes budget violations).
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    if args.samples == 0 {
        bail!("`water bench --samples` must be greater than zero.");
    }
    if args.repetitions == 0 {
        bail!("`water bench --repetitions` must be greater than zero.");
    }
    let path = crate::project_path::canonicalize(&args.path)?;
    header!(shell, "Bench: {}", path.display());
    note!(
        shell,
        "Running `cargo nextest` with {} warmups, {} samples, {} repetitions",
        args.warmups,
        args.samples,
        args.repetitions
    );

    let suite = run_bench_suite(BenchRunOptions {
        path,
        filter: args.filter,
        config: BenchRunConfig {
            warmups: args.warmups,
            samples: args.samples,
            repetitions: args.repetitions,
        },
        report_dir: args.report_dir,
        budget_caps: BenchBudgets {
            max_p95_us: args.max_p95_us,
            max_mean_us: args.max_mean_us,
            max_rebuild_ratio: args.max_rebuild_ratio,
            max_scene_layers: args.max_scene_layers,
            max_gpu_surface_layers: args.max_gpu_surface_layers,
            max_clip_layers: args.max_clip_layers,
        },
    })
    .await?;

    render_reports(shell, &suite, args.format, args.output.as_deref()).await?;
    if let Some(gha_path) = args.gha.as_deref() {
        write_bench_gha_json(gha_path, &suite.reports).await?;
        success!(
            shell,
            "Benchmark-action JSON written: {}",
            gha_path.display()
        );
    }

    finish(shell, &suite)
}

async fn render_reports(
    shell: &Shell,
    suite: &BenchSuiteRun,
    format: BenchOutputFormat,
    output: Option<&std::path::Path>,
) -> Result<()> {
    match format {
        BenchOutputFormat::Human => {
            for report in &suite.reports {
                emit_bench_human(shell, report);
            }
        }
        BenchOutputFormat::Json => {
            if let Some(path) = output {
                write_bench_output_json(path, &suite.reports).await?;
                success!(shell, "Bench JSON written: {}", path.display());
            } else {
                write_bench_stdout_json(&suite.reports)?;
            }
        }
        BenchOutputFormat::Html => {
            let path = output.map_or_else(
                || PathBuf::from("waterui-bench-report.html"),
                std::path::Path::to_path_buf,
            );
            write_bench_html(&path, &suite.reports).await?;
            success!(shell, "Bench HTML report written: {}", path.display());
        }
    }
    Ok(())
}

fn finish(shell: &Shell, suite: &BenchSuiteRun) -> Result<()> {
    if !suite.nextest_succeeded {
        bail!(
            "bench run failed: a bench panicked or exceeded its budget; see the nextest output above"
        );
    }
    success!(
        shell,
        "{} bench report{} collected",
        suite.reports.len(),
        if suite.reports.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn emit_bench_human(shell: &Shell, report: &BenchReport) {
    // This deliberately stays compact and regular: the default terminal format
    // is optimized for humans and LLM agents to scan, while stable machine
    // consumption belongs to JSON mode.
    note!(
        shell,
        "Bench report: {} (warmups={} samples={} repetitions={})",
        bench_target_label(report),
        report.config.warmups,
        report.config.samples,
        report.config.repetitions
    );
    for measurement in &report.measurements {
        note!(
            shell,
            "  {}: samples={} rendered={} idle={} mean={} median={} p95={} min={} max={} rebuilt={}/{} missed120={}/{} missed60={}/{}",
            measurement.name,
            measurement.samples,
            measurement.rendered_frames,
            measurement.idle_frames,
            micros_label(measurement.mean_us),
            micros_label(measurement.median_us),
            micros_label(measurement.p95_us),
            micros_label(measurement.min_us),
            micros_label(measurement.max_us),
            measurement.rebuilt_frames,
            measurement.samples,
            measurement.missed_120fps_frames,
            measurement.samples,
            measurement.missed_60fps_frames,
            measurement.samples
        );
        note!(
            shell,
            "    phases: rebuild mean={} p95={} | build={} p95={} | dispatch={} p95={} | finish={} p95={} | render mean={} p95={} | animation mean={} | input mean={}",
            micros_label(measurement.phases.rebuild_mean_us),
            micros_label(measurement.phases.rebuild_p95_us),
            micros_label(measurement.phases.build_content_mean_us),
            micros_label(measurement.phases.build_content_p95_us),
            micros_label(measurement.phases.scene_dispatch_mean_us),
            micros_label(measurement.phases.scene_dispatch_p95_us),
            micros_label(measurement.phases.scene_finish_mean_us),
            micros_label(measurement.phases.scene_finish_p95_us),
            micros_label(measurement.phases.render_mean_us),
            micros_label(measurement.phases.render_p95_us),
            micros_label(measurement.phases.animation_mean_us),
            micros_label(measurement.phases.input_mean_us)
        );
        note!(
            shell,
            "    layers: compositor={} vello={} gpu-surface={} clip-pushes={} max-clip-depth={} | cache hits={} misses={}",
            measurement.scene_layers,
            measurement.vello_scene_layers,
            measurement.gpu_surface_layers,
            measurement.clip_layers,
            measurement.max_clip_depth,
            measurement.measurement_cache_hits,
            measurement.measurement_cache_misses
        );
        for line in budget_headroom_lines(&report.budgets, measurement) {
            note!(shell, "    budget: {line}");
        }
    }
}

/// Renders one `metric actual/limit (headroom)` line per configured budget.
fn budget_headroom_lines(budgets: &BenchBudgets, measurement: &PerfMeasurement) -> Vec<String> {
    fn headroom(actual: f64, limit: f64) -> String {
        if limit <= 0.0 {
            return "no headroom".to_string();
        }
        format!("{:.0}% headroom", ((limit - actual) / limit) * 100.0)
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "frame counts and microsecond totals are far below f64's integer-exact range"
    )]
    fn count_line(metric: &str, actual: u64, limit: u64) -> String {
        format!(
            "{metric} {actual}/{limit} ({})",
            headroom(actual as f64, limit as f64)
        )
    }
    let mut lines = Vec::new();
    if let Some(limit) = budgets.max_p95_us {
        lines.push(count_line("frame p95 us", measurement.p95_us, limit));
    }
    if let Some(limit) = budgets.max_mean_us {
        lines.push(count_line("frame mean us", measurement.mean_us, limit));
    }
    if let Some(limit) = budgets.max_rebuild_ratio {
        let actual = rebuild_ratio(measurement);
        lines.push(format!(
            "rebuild ratio {actual:.3}/{limit} ({})",
            headroom(actual, limit)
        ));
    }
    if let Some(limit) = budgets.max_scene_layers {
        lines.push(count_line("scene layers", measurement.scene_layers, limit));
    }
    if let Some(limit) = budgets.max_gpu_surface_layers {
        lines.push(count_line(
            "gpu-surface layers",
            measurement.gpu_surface_layers,
            limit,
        ));
    }
    if let Some(limit) = budgets.max_clip_layers {
        lines.push(count_line("clip layers", measurement.clip_layers, limit));
    }
    lines
}

#[expect(
    clippy::cast_precision_loss,
    reason = "frame counts are far below f64's integer-exact range"
)]
fn rebuild_ratio(measurement: &PerfMeasurement) -> f64 {
    if measurement.samples == 0 {
        return 0.0;
    }
    measurement.rebuilt_frames as f64 / measurement.samples as f64
}
