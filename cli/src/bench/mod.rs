//! `water bench` engine.
//!
//! Runs every `#[waterui::bench]` in a crate under `cargo nextest` in
//! full-measurement mode (the `WATERUI_BENCH_*` environment selects the run
//! shape), collects the JSON reports the benches write, and hands them to
//! [`report`] for rendering. Budget evaluation happens inside
//! `waterui-testing` while the benches run — a blown budget is a failed test,
//! which surfaces here as a failed nextest run.

pub mod report;

use std::path::{Path, PathBuf};
use std::process::Stdio;

use color_eyre::eyre::{Result, WrapErr as _, bail};

use waterui_preview_protocol::bench::{
    BENCH_MAX_CLIP_LAYERS_ENV, BENCH_MAX_GPU_SURFACE_LAYERS_ENV, BENCH_MAX_MEAN_US_ENV,
    BENCH_MAX_P95_US_ENV, BENCH_MAX_REBUILD_RATIO_ENV, BENCH_MAX_SCENE_LAYERS_ENV,
    BENCH_REPETITIONS_ENV, BENCH_REPORT_DIR_ENV, BENCH_SAMPLES_ENV, BENCH_WARMUPS_ENV,
    BenchBudgets, BenchReport, BenchRunConfig,
};

/// Test-name prefix `#[waterui::bench]` expands to; the discovery contract
/// between the macro and this runner.
const BENCH_TEST_PREFIX: &str = "waterui_bench_";

/// Resolves a path against the current directory without requiring it to exist.
///
/// `canonicalize` is unusable here: the directory is created after this point.
fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().wrap_err("failed to resolve the current directory")?;
    Ok(cwd.join(path))
}

/// One `water bench` invocation.
#[derive(Debug, Clone)]
pub struct BenchRunOptions {
    /// Directory of the project or crate whose benches run.
    pub path: PathBuf,
    /// Substring narrowing which benches run, matched against the test name.
    pub filter: Option<String>,
    /// Frame-run shape exported to the benches.
    pub config: BenchRunConfig,
    /// Directory receiving the per-bench report JSON files; a temporary
    /// directory is used when unset.
    pub report_dir: Option<PathBuf>,
    /// Budget caps exported to the benches; each is merged against the
    /// attribute budgets with the tighter limit winning.
    pub budget_caps: BenchBudgets,
}

/// Outcome of one bench suite run.
#[derive(Debug)]
pub struct BenchSuiteRun {
    /// Collected reports, sorted by crate then bench name.
    pub reports: Vec<BenchReport>,
    /// Whether the underlying nextest run succeeded (budget violations and
    /// panicking benches fail it).
    pub nextest_succeeded: bool,
}

/// Runs the crate's benches under `cargo nextest` and collects their reports.
///
/// nextest inherits the terminal, so build and per-test progress stream
/// directly to the user.
///
/// # Errors
/// Returns an error when `cargo-nextest` is missing, the run cannot be
/// spawned, or the collected reports cannot be read.
pub async fn run_bench_suite(options: BenchRunOptions) -> Result<BenchSuiteRun> {
    ensure_nextest_installed().await?;

    // Held so a temporary report directory outlives collection.
    let _temp_dir;
    let report_dir = if let Some(dir) = &options.report_dir {
        // Absolute, and deliberately so: this path is both created and read
        // here, in the CLI's working directory, but it is handed to a nextest
        // process whose working directory is the bench crate. A relative path
        // therefore names two different directories on the two sides — the
        // benches write their reports under the crate, and collection then
        // finds nothing and reports the crate as having no benches at all.
        let dir = absolute_path(dir)?;
        smol::fs::create_dir_all(&dir)
            .await
            .wrap_err_with(|| format!("failed to create report directory {}", dir.display()))?;
        clear_stale_reports(&dir).await?;
        dir
    } else {
        let temp_dir = tempfile::Builder::new()
            .prefix("waterui-bench-")
            .tempdir()
            .wrap_err("failed to create temporary bench report directory")?;
        let path = temp_dir.path().to_path_buf();
        _temp_dir = temp_dir;
        path
    };

    let mut command = smol::process::Command::new("cargo");
    command
        .arg("nextest")
        .arg("run")
        .arg("--no-fail-fast")
        .arg("-E")
        .arg(nextest_filter_expression(options.filter.as_deref()))
        .current_dir(&options.path)
        .env(BENCH_WARMUPS_ENV, options.config.warmups.to_string())
        .env(BENCH_SAMPLES_ENV, options.config.samples.to_string())
        .env(
            BENCH_REPETITIONS_ENV,
            options.config.repetitions.to_string(),
        )
        .env(BENCH_REPORT_DIR_ENV, &report_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    apply_budget_cap_envs(&mut command, options.budget_caps);

    let status = command
        .status()
        .await
        .wrap_err("failed to run `cargo nextest`")?;

    let reports = collect_reports(&report_dir).await?;
    if reports.is_empty() && status.success() {
        bail!(
            "no `#[waterui::bench]` reports were produced under {}; \
             the crate has no benches{}",
            options.path.display(),
            options
                .filter
                .as_deref()
                .map(|filter| format!(" matching `{filter}`"))
                .unwrap_or_default()
        );
    }

    Ok(BenchSuiteRun {
        reports,
        nextest_succeeded: status.success(),
    })
}

/// Fails fast with an install hint when `cargo-nextest` is unavailable.
async fn ensure_nextest_installed() -> Result<()> {
    let probe = smol::process::Command::new("cargo")
        .arg("nextest")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await;
    match probe {
        Ok(status) if status.success() => Ok(()),
        _ => bail!(
            "`water bench` requires cargo-nextest. Install it with: cargo install cargo-nextest --locked"
        ),
    }
}

/// Builds the nextest filter expression selecting bench tests, optionally
/// narrowed by a user substring.
fn nextest_filter_expression(filter: Option<&str>) -> String {
    filter.map_or_else(
        || format!("test({BENCH_TEST_PREFIX})"),
        |filter| format!("test({BENCH_TEST_PREFIX}) & test({filter})"),
    )
}

fn apply_budget_cap_envs(command: &mut smol::process::Command, caps: BenchBudgets) {
    let mut set = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            command.env(name, value);
        }
    };
    set(BENCH_MAX_P95_US_ENV, caps.max_p95_us.map(|v| v.to_string()));
    set(
        BENCH_MAX_MEAN_US_ENV,
        caps.max_mean_us.map(|v| v.to_string()),
    );
    set(
        BENCH_MAX_REBUILD_RATIO_ENV,
        caps.max_rebuild_ratio.map(|v| v.to_string()),
    );
    set(
        BENCH_MAX_SCENE_LAYERS_ENV,
        caps.max_scene_layers.map(|v| v.to_string()),
    );
    set(
        BENCH_MAX_GPU_SURFACE_LAYERS_ENV,
        caps.max_gpu_surface_layers.map(|v| v.to_string()),
    );
    set(
        BENCH_MAX_CLIP_LAYERS_ENV,
        caps.max_clip_layers.map(|v| v.to_string()),
    );
}

/// Removes report files left by a previous run so stale benches are never
/// aggregated into this run's output.
async fn clear_stale_reports(dir: &Path) -> Result<()> {
    for path in report_files(dir)? {
        smol::fs::remove_file(&path)
            .await
            .wrap_err_with(|| format!("failed to remove stale bench report {}", path.display()))?;
    }
    Ok(())
}

async fn collect_reports(dir: &Path) -> Result<Vec<BenchReport>> {
    let mut reports = Vec::new();
    for path in report_files(dir)? {
        let raw = smol::fs::read(&path)
            .await
            .wrap_err_with(|| format!("failed to read bench report {}", path.display()))?;
        let report: BenchReport = serde_json::from_slice(&raw)
            .wrap_err_with(|| format!("failed to parse bench report {}", path.display()))?;
        reports.push(report);
    }
    reports.sort_by(|a, b| {
        (a.crate_name.as_str(), a.bench_name.as_str())
            .cmp(&(b.crate_name.as_str(), b.bench_name.as_str()))
    });
    Ok(reports)
}

fn report_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir)
        .wrap_err_with(|| format!("failed to list bench report directory {}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::nextest_filter_expression;

    #[test]
    fn bench_filter_selects_prefix_only_by_default() {
        assert_eq!(nextest_filter_expression(None), "test(waterui_bench_)");
    }

    #[test]
    fn bench_filter_intersects_user_substring() {
        assert_eq!(
            nextest_filter_expression(Some("scroll")),
            "test(waterui_bench_) & test(scroll)"
        );
    }
}
