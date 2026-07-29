//! `water preview` command implementation.
//!
//! Renders, tests, or profiles a `WaterUI` preview.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use askama::Template;
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use color_eyre::eyre::{Result, bail};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use syn::{Attribute, Item};

use crate::shell::Shell;
use crate::toolchain_checks;
use crate::{error, header, note, success, warn};
use waterui_cli::preview::protocol::{AppError, DylibId, function_path_to_symbol};
use waterui_cli::preview::{
    HydrolysisPreviewEventKind, HydrolysisPreviewPerfRun, HydrolysisPreviewPointerButton,
    HydrolysisPreviewRequest, HydrolysisPreviewScenario, HydrolysisPreviewScenarioEvent,
    HydrolysisPreviewSource, HydrolysisPreviewTestMode, HydrolysisPreviewTheme, PreviewPlatform,
    PreviewSession, launch_preview_session, render_preview_with_hydrolysis,
    test_preview_with_hydrolysis,
};
use waterui_cli::toolchain::sccache::Sccache;
use waterui_cli::utils::sccache_install_hint;

/// Target platform for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliPreviewPlatform {
    /// iOS Simulator.
    Ios,
    /// macOS.
    Macos,
    /// Android Emulator.
    Android,
}

impl From<CliPreviewPlatform> for PreviewPlatform {
    fn from(p: CliPreviewPlatform) -> Self {
        match p {
            CliPreviewPlatform::Ios => Self::IosSimulator,
            CliPreviewPlatform::Macos => Self::Macos,
            CliPreviewPlatform::Android => Self::Android,
        }
    }
}

async fn run_preview_test(shell: &Shell, args: PreviewTestArgs) -> Result<()> {
    let platform = resolve_preview_platform(args.platform)?;
    ensure_hydrolysis_preview_platform(platform)?;
    let (width, height) = parse_frame(&args.frame)?;
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let crate_name = read_project_crate_name(&project_path).await?;
    let targets = resolve_test_targets(
        &project_path,
        &crate_name,
        args.target.as_deref(),
        args.expr,
        args.all,
    )
    .await?;
    let automation_body = load_automation_body(
        args.code.as_deref(),
        args.code_file.as_deref(),
        "",
        "`water preview test`",
    )
    .await?;
    let sccache_path = resolve_sccache_path(shell).await;

    for target in targets {
        header!(shell, "Preview test: {}", target.display_name());
        let spinner = shell.spinner("Building and testing with hydrolysis...");
        let output = test_preview_with_hydrolysis(
            HydrolysisPreviewRequest {
                project_path: &project_path,
                source: target.hydrolysis_source(),
                theme: args.theme.into(),
                width,
                height,
                sccache_path: sccache_path.clone(),
            },
            HydrolysisPreviewTestMode::Semantic,
            &automation_body,
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        emit_child_output(shell, &output);
        success!(
            shell,
            "Preview semantic test passed: {}",
            target.display_name()
        );
    }

    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "keeps one ordered perf run lifecycle from artifact planning through report emission"
)]
async fn run_preview_perf(shell: &Shell, args: PreviewPerfArgs) -> Result<()> {
    let platform = resolve_preview_platform(args.platform)?;
    ensure_hydrolysis_preview_platform(platform)?;
    let (width, height) = parse_frame(&args.frame)?;
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let crate_name = read_project_crate_name(&project_path).await?;
    let targets = resolve_test_targets(
        &project_path,
        &crate_name,
        args.target.as_deref(),
        args.expr,
        args.all,
    )
    .await?;
    if args.samples == 0 {
        bail!("`water preview perf --samples` must be greater than zero.");
    }
    if args.flamegraph_frequency <= 0 {
        bail!("`water preview perf --flamegraph-frequency` must be greater than zero.");
    }
    let automation_body = load_automation_body(
        args.code.as_deref(),
        args.code_file.as_deref(),
        "",
        "`water preview perf`",
    )
    .await?;
    let sccache_path = resolve_sccache_path(shell).await;
    let format_output = args.output.clone();
    let flamegraph_path =
        resolve_preview_perf_flamegraph_path(args.all, format_output.as_deref(), args.flamegraph);
    let flamegraphs = resolve_flamegraphs(Some(flamegraph_path.as_path()), args.all, &targets)?;
    let json_reports =
        resolve_perf_artifacts(args.report_json.as_deref(), args.all, &targets, "json")?;
    let traces = resolve_perf_artifacts(args.trace.as_deref(), args.all, &targets, "json")?;
    let html_reports =
        resolve_perf_mode_artifacts(args.format, format_output.as_deref(), args.all, &targets)?;
    let mut reports = Vec::new();

    for ((((target, flamegraph), json_report), trace), html_report) in targets
        .into_iter()
        .zip(flamegraphs)
        .zip(json_reports)
        .zip(traces)
        .zip(html_reports)
    {
        if args.format != PreviewPerfOutputFormat::Json {
            header!(shell, "Preview perf: {}", target.display_name());
        }
        let spinner = (args.format != PreviewPerfOutputFormat::Json)
            .then(|| shell.spinner("Building and profiling with hydrolysis..."))
            .flatten();
        let output = test_preview_with_hydrolysis(
            HydrolysisPreviewRequest {
                project_path: &project_path,
                source: target.hydrolysis_source(),
                theme: args.theme.into(),
                width,
                height,
                sccache_path: sccache_path.clone(),
            },
            HydrolysisPreviewTestMode::Perf(HydrolysisPreviewPerfRun {
                warmups: args.warmups,
                samples: args.samples,
                repetitions: args.repetitions,
                flamegraph: flamegraph.clone(),
                flamegraph_frequency: args.flamegraph_frequency,
            }),
            &automation_body,
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        let mut perf_report =
            parse_preview_perf_output(target.display_name().to_string(), &output)?;
        if let Some(flamegraph) = flamegraph.as_ref() {
            perf_report.flamegraph = Some(flamegraph.clone());
        }
        enforce_perf_budget(
            &perf_report,
            PreviewPerfBudget {
                p95_us: args.max_p95_us,
                rebuild_ratio: args.max_rebuild_ratio,
                scene_layers: args.max_scene_layers,
                gpu_surface_layers: args.max_gpu_surface_layers,
                clip_layers: args.max_clip_layers,
            },
        )?;
        if args.format == PreviewPerfOutputFormat::Human {
            emit_preview_perf_human(shell, &perf_report);
        }
        if let Some(path) = json_report {
            write_preview_perf_json(&path, &perf_report).await?;
        }
        if let Some(path) = trace {
            write_preview_perf_trace(&path, &perf_report).await?;
        }
        if let Some(path) = html_report {
            write_preview_perf_html(&path, std::slice::from_ref(&perf_report)).await?;
            open_preview_perf_html(&path).await?;
            success!(shell, "Preview perf report opened: {}", path.display());
        }
        if args.format != PreviewPerfOutputFormat::Json {
            if let Some(flamegraph) = flamegraph {
                success!(
                    shell,
                    "Preview perf passed: {} (flamegraph: {})",
                    target.display_name(),
                    flamegraph.display()
                );
            } else {
                success!(shell, "Preview perf passed: {}", target.display_name());
            }
        }
        reports.push(perf_report);
    }

    match args.format {
        PreviewPerfOutputFormat::Human => {}
        PreviewPerfOutputFormat::Json => {
            if let Some(path) = format_output.as_deref() {
                write_preview_perf_output_json(path, &reports).await?;
            } else {
                write_preview_perf_stdout_json(&reports)?;
            }
        }
        PreviewPerfOutputFormat::Html => {
            if args.all {
                let path =
                    format_output.unwrap_or_else(|| PathBuf::from("preview-perf-report.html"));
                write_preview_perf_html(&path, &reports).await?;
                open_preview_perf_html(&path).await?;
                success!(shell, "Preview perf report opened: {}", path.display());
            }
        }
    }

    Ok(())
}

/// Rendering backend for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliPreviewBackend {
    /// Apple preview support app.
    Apple,
    /// Android preview support app.
    Android,
    /// Hydrolysis direct renderer.
    Hydrolysis,
}

/// Theme package for Hydrolysis preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliHydrolysisPreviewTheme {
    /// Material Design 3 theme package.
    Material3,
}

impl From<CliHydrolysisPreviewTheme> for HydrolysisPreviewTheme {
    fn from(value: CliHydrolysisPreviewTheme) -> Self {
        match value {
            CliHydrolysisPreviewTheme::Material3 => Self::Material3,
        }
    }
}

/// Arguments for the preview command.
#[derive(ClapArgs, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    /// Preview operation. Omit this to render a preview image.
    #[command(subcommand)]
    command: Option<PreviewCommand>,

    /// Preview target: a `#[preview]` function path or a `WaterUI` expression.
    target: Option<String>,

    /// Treat the target as a `WaterUI` expression returning `impl View`.
    #[arg(long)]
    expr: bool,

    /// Target platform (defaults to the native preview platform).
    #[arg(short, long, value_enum)]
    platform: Option<CliPreviewPlatform>,

    /// Rendering backend.
    #[arg(long, value_enum)]
    backend: Option<CliPreviewBackend>,

    /// Theme package for Hydrolysis preview.
    #[arg(long, value_enum)]
    theme: Option<CliHydrolysisPreviewTheme>,

    /// Frame size `WIDTHxHEIGHT` (default: `375x667`).
    #[arg(short, long, default_value = "375x667")]
    frame: String,

    /// Output file (default: preview.png).
    #[arg(short, long, default_value = "preview.png")]
    output: PathBuf,

    /// Hydrolysis scenario TOML for interaction/timeline capture.
    #[arg(long)]
    scenario: Option<PathBuf>,

    /// Output directory for Hydrolysis scenario frames.
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Subcommand, Debug)]
enum PreviewCommand {
    /// Run semantic assertions against a preview.
    Test(PreviewTestArgs),
    /// Profile a preview through the full Hydrolysis offscreen GPU pipeline.
    Perf(PreviewPerfArgs),
}

#[derive(ClapArgs, Debug)]
struct PreviewTestArgs {
    /// Preview target: a `#[preview]` function path or a `WaterUI` expression.
    target: Option<String>,

    /// Discover and test every `#[preview]` function in the crate.
    #[arg(long)]
    all: bool,

    /// Treat the target as a `WaterUI` expression returning `impl View`.
    #[arg(long)]
    expr: bool,

    /// Target platform (defaults to the native preview platform).
    #[arg(short, long, value_enum)]
    platform: Option<CliPreviewPlatform>,

    /// Theme package for Hydrolysis preview testing.
    #[arg(long, value_enum)]
    theme: CliHydrolysisPreviewTheme,

    /// Frame size `WIDTHxHEIGHT` (default: `375x667`).
    #[arg(short, long, default_value = "375x667")]
    frame: String,

    /// Rust automation body. Receives `app: &mut waterui_testing::SemanticApp`.
    #[arg(long)]
    code: Option<String>,

    /// File containing a Rust automation body.
    #[arg(long)]
    code_file: Option<PathBuf>,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(ClapArgs, Debug)]
struct PreviewPerfArgs {
    /// Preview target: a `#[preview]` function path or a `WaterUI` expression.
    target: Option<String>,

    /// Discover and profile every `#[preview]` function in the crate.
    #[arg(long)]
    all: bool,

    /// Treat the target as a `WaterUI` expression returning `impl View`.
    #[arg(long)]
    expr: bool,

    /// Target platform (defaults to the native preview platform).
    #[arg(short, long, value_enum)]
    platform: Option<CliPreviewPlatform>,

    /// Theme package for Hydrolysis preview perf.
    #[arg(long, value_enum)]
    theme: CliHydrolysisPreviewTheme,

    /// Frame size `WIDTHxHEIGHT` (default: `375x667`).
    #[arg(short, long, default_value = "375x667")]
    frame: String,

    /// Warmup frame count before sampling.
    #[arg(long, default_value_t = 10)]
    warmups: u32,

    /// Recorded frame count per measurement.
    #[arg(long, default_value_t = 120)]
    samples: u32,

    /// Independent measurement repetitions.
    #[arg(long, default_value_t = 7)]
    repetitions: u32,

    /// Rust automation body. Receives `perf: &mut waterui_testing::PerfApp<_, _, _>`.
    #[arg(long)]
    code: Option<String>,

    /// File containing a Rust automation body.
    #[arg(long)]
    code_file: Option<PathBuf>,

    /// Write a CPU call-stack flamegraph SVG. With `--all`, PATH is a directory.
    #[arg(long, num_args = 0..=1, default_missing_value = "__waterui_default_flamegraph__")]
    flamegraph: Option<PathBuf>,

    /// Flamegraph sampling frequency in Hertz.
    #[arg(long, default_value_t = 100)]
    flamegraph_frequency: i32,

    /// Write a machine-readable perf report JSON. With `--all`, PATH is a directory.
    #[arg(long)]
    report_json: Option<PathBuf>,

    /// Write a Chrome/Perfetto-compatible aggregate trace JSON. With `--all`, PATH is a directory.
    #[arg(long)]
    trace: Option<PathBuf>,

    /// Fail when any measurement p95 exceeds this duration in microseconds.
    #[arg(long)]
    max_p95_us: Option<u64>,

    /// Fail when any measurement rebuild ratio exceeds this value.
    #[arg(long)]
    max_rebuild_ratio: Option<f64>,

    /// Fail when any measurement submits more compositor scene layers than this value.
    #[arg(long)]
    max_scene_layers: Option<u64>,

    /// Fail when any measurement submits more embedded GPU surface layers than this value.
    #[arg(long)]
    max_gpu_surface_layers: Option<u64>,

    /// Fail when any measurement pushes more Vello clip layers than this value.
    #[arg(long)]
    max_clip_layers: Option<u64>,

    /// Presentation mode for perf results.
    #[arg(long, value_enum, default_value_t = PreviewPerfOutputFormat::Human)]
    format: PreviewPerfOutputFormat,

    /// Output path for `--format json` or `--format html`. HTML opens automatically.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PreviewPerfOutputFormat {
    /// Human-friendly terminal summary.
    Human,
    /// Structured JSON written to stdout or `--output`.
    Json,
    /// Minimal visual HTML report written to `--output` and opened in the browser.
    Html,
}

impl core::fmt::Display for PreviewPerfOutputFormat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Human => f.write_str("human"),
            Self::Json => f.write_str("json"),
            Self::Html => f.write_str("html"),
        }
    }
}

#[derive(Debug, Serialize)]
struct PreviewPerfOutput<'a> {
    reports: &'a [PreviewPerfReport],
}

#[derive(Debug, Serialize)]
struct PreviewPerfReport {
    target: String,
    measurements: Vec<PreviewPerfMeasurement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flamegraph: Option<PathBuf>,
}

use waterui_preview_protocol::hydrolysis::{
    PerfFrame as PreviewPerfFrame, PerfMeasurement as PreviewPerfMeasurement,
};

/// Run the preview command.
///
/// # Errors
/// Returns an error if preview fails.
#[expect(
    clippy::too_many_lines,
    reason = "keeps preview command dispatch and support-app cleanup in one linear lifecycle"
)]
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    match args.command {
        Some(PreviewCommand::Test(args)) => return run_preview_test(shell, args).await,
        Some(PreviewCommand::Perf(args)) => return run_preview_perf(shell, args).await,
        None => {}
    }

    let Some(target) = args.target.as_deref() else {
        bail!(
            "`water preview` requires a target. Use `water preview <target>`, `water preview test`, or `water preview perf`."
        );
    };

    // Parse frame size
    let (width, height) = parse_frame(&args.frame)?;
    let platform = resolve_preview_platform(args.platform)?;

    // Canonicalize project path
    let project_path = crate::project_path::canonicalize(&args.path)?;

    let crate_name = read_project_crate_name(&project_path).await?;

    let backend = resolve_preview_backend(platform, args.backend)?;
    let hydrolysis_theme = resolve_hydrolysis_preview_theme(backend, args.theme)?;
    let preview_target = resolve_preview_target(&crate_name, target, args.expr);
    header!(shell, "Preview: {}", preview_target.display_name());

    check_toolchain_for_backend(platform, backend).await?;

    // Detect sccache for compilation caching
    let sccache = Sccache;
    let sccache_path = sccache.path().await.map_or_else(
        |_| {
            warn!(
                shell,
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    );

    if backend == CliPreviewBackend::Hydrolysis {
        let scenario = load_hydrolysis_scenario(args.scenario.as_deref(), args.output_dir).await?;
        let spinner = shell.spinner("Building and rendering with hydrolysis...");
        render_preview_with_hydrolysis(
            HydrolysisPreviewRequest {
                project_path: &project_path,
                source: preview_target.hydrolysis_source(),
                theme: hydrolysis_theme.expect("hydrolysis preview theme must be resolved"),
                width,
                height,
                sccache_path,
            },
            &args.output,
            scenario.as_ref(),
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        if let Some(scenario) = scenario {
            success!(
                shell,
                "Preview frames saved to {}",
                scenario.output_dir.display()
            );
        } else {
            success!(shell, "Preview saved to {}", args.output.display());
        }
        return Ok(());
    }

    if args.scenario.is_some() || args.output_dir.is_some() {
        bail!("`--scenario` and `--output-dir` are supported only with `--backend hydrolysis`.");
    }

    let PreviewTarget::Function {
        function_path,
        symbol,
    } = &preview_target
    else {
        bail!("Expression preview is currently supported only with `--backend hydrolysis`.");
    };

    // Launch preview session (connects to existing app or launches new one)
    let spinner = shell.spinner("Connecting to preview app...");
    let preview_platform: PreviewPlatform = platform.into();
    let mut session =
        launch_preview_session(&project_path, preview_platform, sccache_path.clone()).await?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    let result = async {
        // Build dylib
        let spinner = shell.spinner("Building project...");
        let dylib = session.build_dylib(&project_path).await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }

        let spinner = shell.spinner("Rendering view...");
        let png_data = render_with_symbol(
            &mut session,
            function_path,
            symbol,
            dylib.id,
            &dylib.path,
            width,
            height,
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }

        // Save output
        if png_data.is_empty() {
            error!(shell, "Preview returned empty PNG data");
            bail!("Preview returned empty PNG data");
        }

        smol::fs::write(&args.output, &png_data).await?;
        success!(shell, "Preview saved to {}", args.output.display());
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            // Keep preview app running for reuse by future preview commands.
            session.detach();
            Ok(())
        }
        Err(err) => {
            // On failure, terminate the preview app to avoid reusing a broken process.
            match session.shutdown().await {
                Ok(()) => Err(err),
                Err(shutdown_error) => Err(err.wrap_err(format!(
                    "preview support app shutdown also failed: {shutdown_error}"
                ))),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    captures_ms: Vec<u64>,
    #[serde(default)]
    events: Vec<ScenarioEventFile>,
}

#[derive(Debug, Deserialize)]
struct ScenarioEventFile {
    at_ms: u64,
    kind: String,
    x: Option<f32>,
    y: Option<f32>,
    button: Option<String>,
    dx: Option<f32>,
    dy: Option<f32>,
    is_line_delta: Option<bool>,
}

async fn load_hydrolysis_scenario(
    scenario_path: Option<&std::path::Path>,
    output_dir: Option<PathBuf>,
) -> Result<Option<HydrolysisPreviewScenario>> {
    let Some(scenario_path) = scenario_path else {
        if output_dir.is_some() {
            bail!("`--output-dir` requires `--scenario`.");
        }
        return Ok(None);
    };
    let Some(output_dir) = output_dir else {
        bail!("`--scenario` requires `--output-dir`.");
    };
    let source = smol::fs::read_to_string(scenario_path).await?;
    let mut scenario: ScenarioFile = toml::from_str(&source)?;
    if scenario.captures_ms.is_empty() {
        bail!("Hydrolysis preview scenario must contain at least one capture timestamp.");
    }
    scenario.captures_ms.sort_unstable();
    let capture_count = scenario.captures_ms.len();
    scenario.captures_ms.dedup();
    if scenario.captures_ms.len() != capture_count {
        bail!("Hydrolysis preview scenario capture timestamps must be unique.");
    }
    let mut events = scenario
        .events
        .iter()
        .map(parse_scenario_event)
        .collect::<Result<Vec<_>>>()?;
    events.sort_by_key(|event| event.at_ms);
    Ok(Some(HydrolysisPreviewScenario {
        captures_ms: scenario.captures_ms,
        events,
        output_dir,
    }))
}

fn parse_scenario_event(event: &ScenarioEventFile) -> Result<HydrolysisPreviewScenarioEvent> {
    let kind = match event.kind.as_str() {
        "pointer_move" | "hover" => HydrolysisPreviewEventKind::PointerMove,
        "pointer_down" => HydrolysisPreviewEventKind::PointerDown,
        "pointer_up" => HydrolysisPreviewEventKind::PointerUp,
        "pointer_cancel" => HydrolysisPreviewEventKind::PointerCancel,
        "scroll" | "wheel" => HydrolysisPreviewEventKind::Scroll,
        other => {
            bail!("unsupported Hydrolysis preview scenario event kind `{other}`");
        }
    };
    let button = event
        .button
        .as_deref()
        .map(|button| match button {
            "primary" => Ok(HydrolysisPreviewPointerButton::Primary),
            "secondary" => Ok(HydrolysisPreviewPointerButton::Secondary),
            "middle" => Ok(HydrolysisPreviewPointerButton::Middle),
            other => {
                bail!("unsupported Hydrolysis preview pointer button `{other}`");
            }
        })
        .transpose()?
        .unwrap_or_default();
    let needs_point = !matches!(kind, HydrolysisPreviewEventKind::PointerCancel);
    let x = match event.x {
        Some(x) => x,
        None if needs_point => {
            bail!("Hydrolysis preview scenario event requires x coordinate");
        }
        None => 0.0,
    };
    let y = match event.y {
        Some(y) => y,
        None if needs_point => {
            bail!("Hydrolysis preview scenario event requires y coordinate");
        }
        None => 0.0,
    };
    let dx = event.dx.unwrap_or(0.0);
    let dy = event.dy.unwrap_or(0.0);
    if matches!(kind, HydrolysisPreviewEventKind::Scroll)
        && dx.abs() <= f32::EPSILON
        && dy.abs() <= f32::EPSILON
    {
        bail!("Hydrolysis preview scroll event requires non-zero dx or dy");
    }
    Ok(HydrolysisPreviewScenarioEvent {
        at_ms: event.at_ms,
        kind,
        x,
        y,
        button,
        dx,
        dy,
        is_line_delta: event.is_line_delta.unwrap_or(false),
    })
}

async fn read_project_crate_name(project_path: &Path) -> Result<String> {
    let cargo_toml = project_path.join("Cargo.toml");
    let cargo_content = smol::fs::read_to_string(&cargo_toml).await?;
    let cargo: toml::Table = cargo_content.parse()?;
    cargo
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| color_eyre::eyre::eyre!("Could not find package name in Cargo.toml"))
}

async fn resolve_test_targets(
    project_path: &Path,
    crate_name: &str,
    target: Option<&str>,
    force_expression: bool,
    all: bool,
) -> Result<Vec<PreviewTarget>> {
    match (all, target) {
        (true, Some(_)) => {
            bail!("`--all` cannot be combined with an explicit preview target.");
        }
        (true, None) if force_expression => {
            bail!("`--all` cannot be combined with `--expr`.");
        }
        (true, None) => discover_preview_targets(project_path, crate_name).await,
        (false, Some(target)) => {
            if force_expression {
                Ok(vec![PreviewTarget::Expression {
                    expression: target.to_string(),
                }])
            } else {
                Ok(vec![resolve_preview_target(crate_name, target, false)])
            }
        }
        (false, None) => {
            bail!("preview test/perf requires a target or `--all`.");
        }
    }
}

async fn discover_preview_targets(
    project_path: &Path,
    crate_name: &str,
) -> Result<Vec<PreviewTarget>> {
    let src_dir = project_path.join("src");
    let mut previews = BTreeMap::<String, PathBuf>::new();
    let mut duplicates = Vec::<(String, PathBuf, PathBuf)>::new();

    for entry in WalkBuilder::new(&src_dir).standard_filters(true).build() {
        let entry = entry?;
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let source = smol::fs::read_to_string(path).await?;
        let file = syn::parse_file(&source)?;
        collect_preview_functions(path, &file.items, &mut previews, &mut duplicates);
    }

    if !duplicates.is_empty() {
        let mut message = String::from(
            "duplicate `#[preview]` function names are not supported because WaterUI preview exports use function names only:",
        );
        for (name, first, second) in duplicates {
            let _ = write!(
                message,
                "\n  `{name}` in {} and {}",
                first.display(),
                second.display()
            );
        }
        bail!("{message}");
    }
    if previews.is_empty() {
        bail!(
            "no `#[preview]` functions found under {}",
            src_dir.display()
        );
    }

    Ok(previews
        .into_keys()
        .map(|function_name| PreviewTarget::Function {
            symbol: function_path_to_symbol(crate_name, &function_name),
            function_path: function_name,
        })
        .collect())
}

fn collect_preview_functions(
    path: &Path,
    items: &[Item],
    previews: &mut BTreeMap<String, PathBuf>,
    duplicates: &mut Vec<(String, PathBuf, PathBuf)>,
) {
    for item in items {
        match item {
            Item::Fn(function) if has_preview_attr(&function.attrs) => {
                let name = function.sig.ident.to_string();
                if let Some(first) = previews.get(&name) {
                    duplicates.push((name, first.clone(), path.to_path_buf()));
                } else {
                    previews.insert(name, path.to_path_buf());
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    collect_preview_functions(path, items, previews, duplicates);
                }
            }
            _ => {}
        }
    }
}

fn has_preview_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        path.is_ident("preview") || segments == ["waterui".to_string(), "preview".to_string()]
    })
}

async fn load_automation_body(
    code: Option<&str>,
    code_file: Option<&Path>,
    default_body: &str,
    command_name: &str,
) -> Result<String> {
    match (code, code_file) {
        (Some(_), Some(_)) => {
            bail!("{command_name} accepts either `--code` or `--code-file`, not both.");
        }
        (Some(code), None) => Ok(code.to_string()),
        (None, Some(path)) => smol::fs::read_to_string(path).await.map_err(Into::into),
        (None, None) => Ok(default_body.to_string()),
    }
}

fn resolve_flamegraphs(
    flamegraph: Option<&Path>,
    all: bool,
    targets: &[PreviewTarget],
) -> Result<Vec<Option<PathBuf>>> {
    let Some(path) = flamegraph else {
        return Ok(std::iter::repeat_with(|| None)
            .take(targets.len())
            .collect());
    };
    let default_marker = Path::new("__waterui_default_flamegraph__");
    if all {
        let dir = if path == default_marker {
            std::env::temp_dir().join("waterui-preview-flamegraphs")
        } else {
            path.to_path_buf()
        };
        if dir.exists() && !dir.is_dir() {
            bail!(
                "`water preview perf --all --flamegraph` expects a directory, got {}",
                dir.display()
            );
        }
        std::fs::create_dir_all(&dir)?;
        return Ok(targets
            .iter()
            .map(|target| Some(dir.join(format!("{}.svg", target.file_stem()))))
            .collect());
    }
    if targets.len() != 1 {
        bail!("internal error: single flamegraph path received for multiple preview targets");
    }
    let output_path = if path == default_marker {
        std::env::temp_dir().join("waterui-preview-flamegraph.svg")
    } else {
        path.to_path_buf()
    };
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(vec![Some(output_path)])
}

fn resolve_preview_perf_flamegraph_path(
    all: bool,
    output: Option<&Path>,
    flamegraph: Option<PathBuf>,
) -> PathBuf {
    flamegraph.unwrap_or_else(|| {
        if all {
            PathBuf::from("__waterui_default_flamegraph__")
        } else {
            output.map_or_else(
                || PathBuf::from("__waterui_default_flamegraph__"),
                |path| path.with_extension("flamegraph.svg"),
            )
        }
    })
}

fn resolve_perf_artifacts(
    path: Option<&Path>,
    all: bool,
    targets: &[PreviewTarget],
    extension: &str,
) -> Result<Vec<Option<PathBuf>>> {
    let Some(path) = path else {
        return Ok(std::iter::repeat_with(|| None)
            .take(targets.len())
            .collect());
    };
    if all {
        if path.exists() && !path.is_dir() {
            bail!(
                "`water preview perf --all` expects artifact paths to be directories, got {}",
                path.display()
            );
        }
        std::fs::create_dir_all(path)?;
        return Ok(targets
            .iter()
            .map(|target| Some(path.join(format!("{}.{}", target.file_stem(), extension))))
            .collect());
    }
    if targets.len() != 1 {
        bail!("internal error: single perf artifact path received for multiple preview targets");
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(vec![Some(path.to_path_buf())])
}

fn resolve_perf_mode_artifacts(
    format: PreviewPerfOutputFormat,
    output: Option<&Path>,
    all: bool,
    targets: &[PreviewTarget],
) -> Result<Vec<Option<PathBuf>>> {
    match format {
        PreviewPerfOutputFormat::Human | PreviewPerfOutputFormat::Json => {
            Ok(std::iter::repeat_with(|| None)
                .take(targets.len())
                .collect())
        }
        PreviewPerfOutputFormat::Html if all => Ok(std::iter::repeat_with(|| None)
            .take(targets.len())
            .collect()),
        PreviewPerfOutputFormat::Html => {
            if targets.len() != 1 {
                bail!(
                    "internal error: single HTML report path received for multiple preview targets"
                );
            }
            let path = output.map_or_else(
                || std::env::temp_dir().join("waterui-preview-perf.html"),
                Path::to_path_buf,
            );
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            Ok(vec![Some(path)])
        }
    }
}

fn parse_preview_perf_output(target: String, output: &str) -> Result<PreviewPerfReport> {
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

fn emit_preview_perf_human(shell: &Shell, report: &PreviewPerfReport) {
    // This deliberately stays compact and regular: the default terminal format is optimized for
    // humans and LLM agents to scan, while stable machine consumption belongs to JSON mode.
    note!(shell, "Perf report: {}", report.target);
    for measurement in &report.measurements {
        note!(
            shell,
            "  {}: samples={} rendered={} idle={} mean={} median={} p95={} min={} max={} rendered-mean={} rendered-p95={} rendered-max={} rebuilt={}/{} missed120={}/{} missed60={}/{}",
            measurement.name,
            measurement.samples,
            measurement.rendered_frames,
            measurement.idle_frames,
            micros_label(measurement.mean_us),
            micros_label(measurement.median_us),
            micros_label(measurement.p95_us),
            micros_label(measurement.min_us),
            micros_label(measurement.max_us),
            micros_label(measurement.rendered_mean_us),
            micros_label(measurement.rendered_p95_us),
            micros_label(measurement.rendered_max_us),
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
            "    caches: measurement hits={} misses={}",
            measurement.measurement_cache_hits,
            measurement.measurement_cache_misses
        );
        note!(
            shell,
            "    layers: compositor={} vello={} gpu-surface={} clip-pushes={} max-clip-depth={}",
            measurement.scene_layers,
            measurement.vello_scene_layers,
            measurement.gpu_surface_layers,
            measurement.clip_layers,
            measurement.max_clip_depth
        );
        note!(
            shell,
            "    filters: applied={} capture={} effect={}",
            measurement.applied_filter_count,
            micros_label(measurement.applied_filter_capture_us),
            micros_label(measurement.applied_filter_effect_us)
        );
        if let Some(resources) = resource_summary(measurement) {
            note!(
                shell,
                "    resources: cpu avg={:.1}% max={:.1}% | memory max={} | gpu-frame avg={} max={} | layers avg={:.1} max={} | clip avg={:.1} max-depth={} | raw_samples={}",
                resources.avg_cpu_percent,
                resources.max_cpu_percent,
                bytes_label(resources.max_memory_bytes),
                micros_label(resources.avg_gpu_frame_us),
                micros_label(resources.max_gpu_frame_us),
                resources.avg_scene_layers,
                resources.max_scene_layers,
                resources.avg_clip_layers,
                resources.max_clip_depth,
                measurement.frames.len()
            );
        }
    }
    if let Some(flamegraph) = &report.flamegraph {
        note!(shell, "  flamegraph: {}", flamegraph.display());
    }
}

fn micros_label(value: u64) -> String {
    format!("{value}us")
}

#[expect(
    clippy::cast_precision_loss,
    reason = "preview charts intentionally project integer telemetry into floating-point display coordinates"
)]
const fn metric_to_f64(value: u64) -> f64 {
    value as f64
}

#[expect(
    clippy::cast_precision_loss,
    reason = "preview chart sample counts are converted only for display averages and coordinates"
)]
const fn sample_count_to_f64(value: usize) -> f64 {
    value as f64
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "preview chart scales contain finite non-negative telemetry and labels use rounded integers"
)]
fn rounded_metric_to_u64(value: f64) -> u64 {
    assert!(
        value.is_finite() && value >= 0.0 && value <= metric_to_f64(u64::MAX),
        "preview chart metric must be finite, non-negative, and fit into u64"
    );
    value.round() as u64
}

fn bytes_label(value: u64) -> String {
    const MIB: f64 = 1_048_576.0;
    format!("{:.1}MiB", metric_to_f64(value) / MIB)
}

struct PreviewPerfResourceSummary {
    avg_cpu_percent: f64,
    max_cpu_percent: f64,
    max_memory_bytes: u64,
    avg_gpu_frame_us: u64,
    max_gpu_frame_us: u64,
    avg_scene_layers: f64,
    max_scene_layers: u64,
    avg_clip_layers: f64,
    max_clip_depth: u64,
}

fn resource_summary(measurement: &PreviewPerfMeasurement) -> Option<PreviewPerfResourceSummary> {
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

#[derive(Clone, Copy, Debug)]
struct PreviewPerfBudget {
    p95_us: Option<u64>,
    rebuild_ratio: Option<f64>,
    scene_layers: Option<u64>,
    gpu_surface_layers: Option<u64>,
    clip_layers: Option<u64>,
}

fn enforce_perf_budget(report: &PreviewPerfReport, budget: PreviewPerfBudget) -> Result<()> {
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

async fn write_preview_perf_json(path: &Path, report: &PreviewPerfReport) -> Result<()> {
    let json = serde_json::to_vec_pretty(report)?;
    smol::fs::write(path, json).await?;
    Ok(())
}

fn write_preview_perf_stdout_json(reports: &[PreviewPerfReport]) -> Result<()> {
    let json = serde_json::to_vec_pretty(&PreviewPerfOutput { reports })?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&json)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

async fn write_preview_perf_output_json(path: &Path, reports: &[PreviewPerfReport]) -> Result<()> {
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

async fn write_preview_perf_trace(path: &Path, report: &PreviewPerfReport) -> Result<()> {
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

async fn write_preview_perf_html(path: &Path, reports: &[PreviewPerfReport]) -> Result<()> {
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

fn preview_perf_throughput_fps(frame: &PreviewPerfFrame) -> f64 {
    1_000_000.0 / metric_to_f64(frame.total_us.max(1))
}

fn fps_label(value: f64) -> String {
    if value >= 1_000.0 {
        ">=1000fps".to_string()
    } else {
        format!("{value:.1}fps")
    }
}

fn ratio_percent(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    (metric_to_f64(numerator) / metric_to_f64(denominator)) * 100.0
}

async fn open_preview_perf_html(path: &Path) -> Result<()> {
    let path = crate::project_path::canonicalize(path)?;
    let mut command = if cfg!(target_os = "macos") {
        let mut command = smol::process::Command::new("open");
        command.arg(&path);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = smol::process::Command::new("cmd");
        command.arg("/C").arg("start").arg("").arg(&path);
        command
    } else {
        let mut command = smol::process::Command::new("xdg-open");
        command.arg(&path);
        command
    };
    let status = command
        .status()
        .await
        .map_err(|error| color_eyre::eyre::eyre!("failed to open HTML report: {error}"))?;
    if !status.success() {
        bail!("failed to open HTML report {}: {status}", path.display());
    }
    Ok(())
}

async fn resolve_sccache_path(shell: &Shell) -> Option<PathBuf> {
    let sccache = Sccache;
    sccache.path().await.map_or_else(
        |_| {
            warn!(
                shell,
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    )
}

fn emit_child_output(shell: &Shell, output: &str) {
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        note!(shell, "{line}");
    }
}

#[derive(Debug)]
enum PreviewTarget {
    Function {
        function_path: String,
        symbol: String,
    },
    Expression {
        expression: String,
    },
}

impl PreviewTarget {
    fn display_name(&self) -> &str {
        match self {
            Self::Function { symbol, .. } => symbol,
            Self::Expression { expression } => expression,
        }
    }

    fn hydrolysis_source(&self) -> HydrolysisPreviewSource<'_> {
        match self {
            Self::Function { symbol, .. } => HydrolysisPreviewSource::Symbol(symbol),
            Self::Expression { expression } => HydrolysisPreviewSource::Expression(expression),
        }
    }

    fn file_stem(&self) -> String {
        match self {
            Self::Function { function_path, .. } => function_path.replace("::", "_"),
            Self::Expression { .. } => "expression".to_string(),
        }
    }
}

fn resolve_preview_target(crate_name: &str, target: &str, force_expression: bool) -> PreviewTarget {
    if force_expression || !is_function_path(target) {
        return PreviewTarget::Expression {
            expression: target.to_string(),
        };
    }

    PreviewTarget::Function {
        function_path: target.to_string(),
        symbol: function_path_to_symbol(crate_name, target),
    }
}

fn is_function_path(target: &str) -> bool {
    let mut segments = target.split("::").peekable();
    if segments.peek().is_none() {
        return false;
    }

    segments.all(is_rust_ident)
}

fn is_rust_ident(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn resolve_preview_backend(
    platform: CliPreviewPlatform,
    backend_override: Option<CliPreviewBackend>,
) -> Result<CliPreviewBackend> {
    let default_backend = match platform {
        CliPreviewPlatform::Ios | CliPreviewPlatform::Macos => CliPreviewBackend::Apple,
        CliPreviewPlatform::Android => CliPreviewBackend::Android,
    };

    let backend = backend_override.unwrap_or(default_backend);
    let supported = matches!(
        (platform, backend),
        (
            CliPreviewPlatform::Ios | CliPreviewPlatform::Macos,
            CliPreviewBackend::Apple
        ) | (CliPreviewPlatform::Macos, CliPreviewBackend::Hydrolysis)
            | (CliPreviewPlatform::Android, CliPreviewBackend::Android)
    );
    if !supported {
        bail!(
            "Preview backend {:?} does not support platform {:?}. Valid combinations: ios/apple, macos/apple, macos/hydrolysis, android/android",
            backend,
            platform
        );
    }
    Ok(backend)
}

const fn resolve_preview_platform(
    platform_override: Option<CliPreviewPlatform>,
) -> Result<CliPreviewPlatform> {
    if let Some(platform) = platform_override {
        return Ok(platform);
    }
    native_preview_platform()
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "non-macOS hosts return an explicit unsupported-host error"
)]
const fn native_preview_platform() -> Result<CliPreviewPlatform> {
    #[cfg(target_os = "macos")]
    {
        Ok(CliPreviewPlatform::Macos)
    }

    #[cfg(not(target_os = "macos"))]
    {
        bail!(
            "No native preview platform is configured for this host. Pass `--platform` explicitly."
        )
    }
}

fn ensure_hydrolysis_preview_platform(platform: CliPreviewPlatform) -> Result<()> {
    if platform != CliPreviewPlatform::Macos {
        bail!("`water preview test` and `water preview perf` support Hydrolysis on macos only.");
    }
    Ok(())
}

fn resolve_hydrolysis_preview_theme(
    backend: CliPreviewBackend,
    theme: Option<CliHydrolysisPreviewTheme>,
) -> Result<Option<HydrolysisPreviewTheme>> {
    match (backend, theme) {
        (CliPreviewBackend::Hydrolysis, Some(theme)) => Ok(Some(theme.into())),
        (CliPreviewBackend::Hydrolysis, None) => {
            bail!(
                "Hydrolysis preview requires an explicit theme package. Pass `--theme material3`."
            );
        }
        (_, Some(_)) => {
            bail!("`--theme` is only supported with `--backend hydrolysis`.");
        }
        (_, None) => Ok(None),
    }
}

async fn check_toolchain_for_backend(
    platform: CliPreviewPlatform,
    backend: CliPreviewBackend,
) -> Result<()> {
    match backend {
        CliPreviewBackend::Apple => {
            let sdk = match platform {
                CliPreviewPlatform::Ios => waterui_cli::apple::toolchain::AppleSdk::IosSimulator,
                CliPreviewPlatform::Macos => waterui_cli::apple::toolchain::AppleSdk::Macos,
                CliPreviewPlatform::Android => {
                    bail!("Internal error: Apple preview backend is not supported on android");
                }
            };
            toolchain_checks::check_apple(sdk).await?;
        }
        CliPreviewBackend::Android => {
            if platform != CliPreviewPlatform::Android {
                bail!("Internal error: Android preview backend is not supported on {platform:?}");
            }
            toolchain_checks::check_android_run().await?;
        }
        CliPreviewBackend::Hydrolysis => {
            if platform != CliPreviewPlatform::Macos {
                bail!(
                    "Internal error: Hydrolysis preview backend is not supported on {platform:?}"
                );
            }
        }
    }
    Ok(())
}

async fn render_with_symbol(
    session: &mut PreviewSession,
    function_path: &str,
    symbol: &str,
    dylib_id: DylibId,
    dylib_path: &std::path::Path,
    width: f32,
    height: f32,
) -> Result<Vec<u8>> {
    let prefer_local_path = session.platform == PreviewPlatform::Macos;
    match session
        .client
        .render_with_dylib_file(
            dylib_id,
            dylib_path,
            symbol,
            width,
            height,
            prefer_local_path,
        )
        .await
    {
        Ok(data) => Ok(data),
        Err(AppError::SymbolNotFound(_)) => {
            bail!("{}", missing_preview_symbol_message(function_path, symbol));
        }
        Err(err) => {
            bail!("Preview app error: {err}");
        }
    }
}

/// Parse frame size from `WIDTHxHEIGHT` string.
fn parse_frame(s: &str) -> Result<(f32, f32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        bail!("Invalid frame format: expected WIDTHxHEIGHT (e.g., 375x667)");
    }

    let width: f32 = parts[0]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid frame width"))?;
    let height: f32 = parts[1]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid frame height"))?;

    if !width.is_finite() || width <= 0.0 {
        bail!("Invalid frame width: must be a positive finite number");
    }
    if !height.is_finite() || height <= 0.0 {
        bail!("Invalid frame height: must be a positive finite number");
    }

    Ok((width, height))
}

fn missing_preview_symbol_message(function_path: &str, symbol: &str) -> String {
    format!(
        "Preview component not found: `{function_path}`\nExpected export symbol: `{symbol}`\n\
The preview function is likely missing `#[preview]` (or the name is wrong).\n\
Example:\n  #[preview]\n  fn {}() -> impl View {{ ... }}",
        function_path.rsplit("::").next().unwrap_or(function_path)
    )
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

    #[test]
    fn formats_missing_preview_symbol_message() {
        let symbol = "waterui_preview_app_card_preview";
        let message = missing_preview_symbol_message("dashboard::admin::card_preview", symbol);
        assert!(message.contains("dashboard::admin::card_preview"));
        assert!(message.contains("waterui_preview_app_card_preview"));
        assert!(message.contains("#[preview]"));
        assert!(message.contains("fn card_preview()"));
    }

    #[test]
    fn rejects_non_positive_frame_values() {
        assert!(parse_frame("0x100").is_err());
        assert!(parse_frame("-1x100").is_err());
        assert!(parse_frame("100x0").is_err());
        assert!(parse_frame("100x-1").is_err());
    }

    #[test]
    fn rejects_non_finite_frame_values() {
        assert!(parse_frame("NaNx100").is_err());
        assert!(parse_frame("100xinf").is_err());
    }

    #[test]
    fn resolves_plain_path_as_preview_function() {
        let target = resolve_preview_target("my-crate", "dashboard::card", false);
        let PreviewTarget::Function {
            function_path,
            symbol,
        } = target
        else {
            panic!("expected function target");
        };
        assert_eq!(function_path, "dashboard::card");
        assert_eq!(symbol, "waterui_preview_my_crate_card");
    }

    #[test]
    fn resolves_expression_syntax_as_expression_preview() {
        let target = resolve_preview_target("my-crate", "button(\"Save\")", false);
        let PreviewTarget::Expression { expression } = target else {
            panic!("expected expression target");
        };
        assert_eq!(expression, "button(\"Save\")");
    }

    #[test]
    fn expr_flag_forces_identifier_as_expression_preview() {
        let target = resolve_preview_target("my-crate", "main_view", true);
        let PreviewTarget::Expression { expression } = target else {
            panic!("expected expression target");
        };
        assert_eq!(expression, "main_view");
    }

    #[test]
    fn hydrolysis_preview_requires_explicit_theme() {
        let result = resolve_hydrolysis_preview_theme(CliPreviewBackend::Hydrolysis, None);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_theme_for_non_hydrolysis_preview() {
        let result = resolve_hydrolysis_preview_theme(
            CliPreviewBackend::Apple,
            Some(CliHydrolysisPreviewTheme::Material3),
        );
        assert!(result.is_err());
    }
}
