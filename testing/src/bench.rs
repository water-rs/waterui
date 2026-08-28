//! Execution engine behind `#[waterui::bench(...)]`.
//!
//! The attribute macro expands into a plain `#[test]` named
//! `waterui_bench_<name>` that calls [`run_bench`]. The run shape comes from
//! the environment: when none of `WATERUI_BENCH_SAMPLES` /
//! `WATERUI_BENCH_WARMUPS` / `WATERUI_BENCH_REPETITIONS` is set the bench runs
//! in *smoke mode* (0 warmups, 2 samples, 1 repetition) so a plain
//! `cargo nextest run` keeps every bench compiling and executing cheaply as a
//! correctness test. Setting any of them switches to a *full run*: budgets are
//! enforced in-process (a violation panics, so a blown budget is a failed
//! test), and when `WATERUI_BENCH_REPORT_DIR` is set the serialized
//! [`BenchReport`] is written there for `water bench` to aggregate.
//!
//! Budget evaluation lives here — and only here. `water bench` narrows budgets
//! by exporting the `WATERUI_BENCH_MAX_*` cap variables, which are merged on
//! top of the attribute budgets with the tighter limit winning.

use std::path::PathBuf;
use std::time::Duration;

use waterui_preview_protocol::bench::{
    BENCH_MAX_CLIP_LAYERS_ENV, BENCH_MAX_GPU_SURFACE_LAYERS_ENV, BENCH_MAX_MEAN_US_ENV,
    BENCH_MAX_P95_US_ENV, BENCH_MAX_REBUILD_RATIO_ENV, BENCH_MAX_SCENE_LAYERS_ENV,
    BENCH_REPETITIONS_ENV, BENCH_REPORT_DIR_ENV, BENCH_SAMPLES_ENV, BENCH_WARMUPS_ENV,
    bench_report_file_name,
};
pub use waterui_preview_protocol::bench::{BenchBudgets, BenchReport, BenchRunConfig};

use crate::perf::{PerfConfig, PerfReport, PerfStats};
use crate::protocol::perf_report_to_protocol;

/// Run shape used when no `WATERUI_BENCH_*` run variable is set: benches then
/// act as cheap correctness tests under a plain `cargo nextest run`.
const SMOKE_CONFIG: PerfConfig = PerfConfig {
    warmups: 0,
    samples: 2,
    repetitions: 1,
};

/// Executes one bench body and applies full-run reporting and budgets.
///
/// The macro passes the attribute budgets; `run` receives the resolved
/// [`PerfConfig`] and must return the recorded report.
///
/// # Panics
///
/// Panics when the bench records no measurements, when a `WATERUI_BENCH_*`
/// variable cannot be parsed, when the report file cannot be written, or —
/// in full-run mode — when any measurement exceeds its effective budget.
pub fn run_bench(
    crate_name: &str,
    bench_name: &str,
    budgets: BenchBudgets,
    run: impl FnOnce(PerfConfig) -> PerfReport,
) {
    let full_run = run_config_from_env();
    let config = full_run.map_or(SMOKE_CONFIG, |config| PerfConfig {
        warmups: config.warmups,
        samples: config.samples,
        repetitions: config.repetitions,
    });

    let report = run(config);
    assert!(
        !report.measurements().is_empty(),
        "bench `{crate_name}::{bench_name}` recorded no measurements; call `measure` at least once"
    );

    let Some(run_config) = full_run else {
        return;
    };

    let effective = merge_budgets(budgets, budget_caps_from_env());
    write_report(crate_name, bench_name, run_config, effective, &report);
    enforce_budgets(crate_name, bench_name, effective, &report);
}

/// Reads the full-run shape from the environment; `None` means smoke mode.
fn run_config_from_env() -> Option<BenchRunConfig> {
    let samples = env_parsed::<u32>(BENCH_SAMPLES_ENV);
    let warmups = env_parsed::<u32>(BENCH_WARMUPS_ENV);
    let repetitions = env_parsed::<u32>(BENCH_REPETITIONS_ENV);
    if samples.is_none() && warmups.is_none() && repetitions.is_none() {
        return None;
    }
    let defaults = PerfConfig::default();
    Some(BenchRunConfig {
        warmups: warmups.unwrap_or(defaults.warmups),
        samples: samples.unwrap_or(defaults.samples),
        repetitions: repetitions.unwrap_or(defaults.repetitions),
    })
}

/// Reads the CLI budget caps from the environment.
fn budget_caps_from_env() -> BenchBudgets {
    BenchBudgets {
        max_p95_us: env_parsed::<u64>(BENCH_MAX_P95_US_ENV),
        max_mean_us: env_parsed::<u64>(BENCH_MAX_MEAN_US_ENV),
        max_rebuild_ratio: env_parsed::<f64>(BENCH_MAX_REBUILD_RATIO_ENV),
        max_scene_layers: env_parsed::<u64>(BENCH_MAX_SCENE_LAYERS_ENV),
        max_gpu_surface_layers: env_parsed::<u64>(BENCH_MAX_GPU_SURFACE_LAYERS_ENV),
        max_clip_layers: env_parsed::<u64>(BENCH_MAX_CLIP_LAYERS_ENV),
    }
}

/// Merges attribute budgets with environment caps; the tighter limit wins.
fn merge_budgets(attribute: BenchBudgets, caps: BenchBudgets) -> BenchBudgets {
    fn tighter<T: PartialOrd>(a: Option<T>, b: Option<T>) -> Option<T> {
        match (a, b) {
            (Some(a), Some(b)) => Some(if b < a { b } else { a }),
            (value, None) | (None, value) => value,
        }
    }
    BenchBudgets {
        max_p95_us: tighter(attribute.max_p95_us, caps.max_p95_us),
        max_mean_us: tighter(attribute.max_mean_us, caps.max_mean_us),
        max_rebuild_ratio: tighter(attribute.max_rebuild_ratio, caps.max_rebuild_ratio),
        max_scene_layers: tighter(attribute.max_scene_layers, caps.max_scene_layers),
        max_gpu_surface_layers: tighter(
            attribute.max_gpu_surface_layers,
            caps.max_gpu_surface_layers,
        ),
        max_clip_layers: tighter(attribute.max_clip_layers, caps.max_clip_layers),
    }
}

/// Panics with a precise message when any measurement exceeds the budgets.
fn enforce_budgets(crate_name: &str, bench_name: &str, budgets: BenchBudgets, report: &PerfReport) {
    for measurement in report.measurements() {
        let stats = measurement.stats();
        let violation = |metric: &str, actual: String, limit: String| -> ! {
            panic!(
                "bench `{crate_name}::{bench_name}` measurement `{name}`: {metric} {actual} exceeded budget {limit}",
                name = measurement.name,
            )
        };
        if let Some(limit) = budgets.max_p95_us {
            let actual = micros(stats.p95);
            if actual > limit {
                violation("frame p95", format!("{actual}us"), format!("{limit}us"));
            }
        }
        if let Some(limit) = budgets.max_mean_us {
            let actual = micros(stats.mean);
            if actual > limit {
                violation("frame mean", format!("{actual}us"), format!("{limit}us"));
            }
        }
        if let Some(limit) = budgets.max_rebuild_ratio {
            let actual = rebuild_ratio(&stats);
            if actual > limit {
                violation("rebuild ratio", format!("{actual:.4}"), format!("{limit}"));
            }
        }
        if let Some(limit) = budgets.max_scene_layers
            && stats.scene_layers > limit
        {
            violation(
                "scene layers",
                stats.scene_layers.to_string(),
                limit.to_string(),
            );
        }
        if let Some(limit) = budgets.max_gpu_surface_layers
            && stats.gpu_surface_layers > limit
        {
            violation(
                "GPU surface layers",
                stats.gpu_surface_layers.to_string(),
                limit.to_string(),
            );
        }
        if let Some(limit) = budgets.max_clip_layers
            && stats.clip_layers > limit
        {
            violation(
                "clip layers",
                stats.clip_layers.to_string(),
                limit.to_string(),
            );
        }
    }
}

/// Share of sampled frames that performed a structural rebuild.
fn rebuild_ratio(stats: &PerfStats) -> f64 {
    if stats.samples == 0 {
        return 0.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "frame counts are far below f64's integer-exact range"
    )]
    {
        stats.rebuilt_frames as f64 / stats.samples as f64
    }
}

fn write_report(
    crate_name: &str,
    bench_name: &str,
    config: BenchRunConfig,
    budgets: BenchBudgets,
    report: &PerfReport,
) {
    let Some(dir) = std::env::var_os(BENCH_REPORT_DIR_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
        panic!(
            "bench `{crate_name}::{bench_name}`: failed to create report directory `{}`: {error}",
            dir.display()
        )
    });
    let wire = BenchReport {
        crate_name: crate_name.to_owned(),
        bench_name: bench_name.to_owned(),
        config,
        budgets,
        measurements: perf_report_to_protocol(report),
    };
    let path = dir.join(bench_report_file_name(crate_name, bench_name));
    let json = serde_json::to_vec(&wire).unwrap_or_else(|error| {
        panic!("bench `{crate_name}::{bench_name}`: failed to serialize report: {error}")
    });
    std::fs::write(&path, json).unwrap_or_else(|error| {
        panic!(
            "bench `{crate_name}::{bench_name}`: failed to write report `{}`: {error}",
            path.display()
        )
    });
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Reads and parses one `WATERUI_BENCH_*` variable; an unparsable value is a
/// configuration bug and fails fast.
fn env_parsed<T>(name: &str) -> Option<T>
where
    T: std::str::FromStr,
    T::Err: core::fmt::Display,
{
    let value = std::env::var(name).ok()?;
    match value.trim().parse::<T>() {
        Ok(parsed) => Some(parsed),
        Err(error) => panic!("failed to parse `{name}={value}`: {error}"),
    }
}
