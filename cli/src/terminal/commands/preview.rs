//! `water preview` command implementation.
//!
//! Renders, tests, or profiles a WaterUI preview.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use color_eyre::eyre::{Result, bail};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use syn::{Attribute, Item};

use crate::shell;
use crate::toolchain_checks;
use crate::{error, header, note, success, warn};
use waterui_cli::preview::protocol::{AppError, DylibId, function_path_to_symbol};
use waterui_cli::preview::{
    HydrolysisPreviewEventKind, HydrolysisPreviewPointerButton, HydrolysisPreviewScenario,
    HydrolysisPreviewScenarioEvent, HydrolysisPreviewSource, HydrolysisPreviewTestMode,
    HydrolysisPreviewTheme, PreviewPlatform, PreviewSession, launch_preview_session,
    render_preview_with_hydrolysis, test_preview_with_hydrolysis,
};
use waterui_cli::preview::{HydrolysisPreviewFlamegraph, HydrolysisPreviewPerfConfig};
use waterui_cli::toolchain::sccache::Sccache;
use waterui_cli::utils::sccache_install_hint;

const PREVIEW_PERF_HTML_TEMPLATE: &str = include_str!("../../templates/preview_perf_report.html");

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

async fn run_preview_test(args: PreviewTestArgs) -> Result<()> {
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
    let sccache_path = resolve_sccache_path().await;

    for target in targets {
        header!("Preview test: {}", target.display_name());
        let spinner = shell::spinner("Building and testing with hydrolysis...");
        let output = test_preview_with_hydrolysis(
            &project_path,
            target.hydrolysis_source(),
            args.theme.into(),
            HydrolysisPreviewTestMode::Semantic,
            width,
            height,
            sccache_path.clone(),
            &automation_body,
            None,
            None,
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        emit_child_output(&output);
        success!("Preview semantic test passed: {}", target.display_name());
    }

    Ok(())
}

async fn run_preview_perf(args: PreviewPerfArgs) -> Result<()> {
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
    let sccache_path = resolve_sccache_path().await;
    let format_output = args.output.clone();
    let flamegraph_path =
        resolve_preview_perf_flamegraph_path(args.all, format_output.as_deref(), args.flamegraph);
    let flamegraphs = resolve_flamegraphs(
        Some(flamegraph_path.as_path()),
        args.all,
        &targets,
        args.flamegraph_frequency,
    )?;
    let json_reports =
        resolve_perf_artifacts(args.report_json.as_deref(), args.all, &targets, "json")?;
    let traces = resolve_perf_artifacts(args.trace.as_deref(), args.all, &targets, "json")?;
    let html_reports =
        resolve_perf_mode_artifacts(args.format, format_output.as_deref(), args.all, &targets)?;
    let mut reports = Vec::new();

    for ((((target, flamegraph), json_report), trace), html_report) in targets
        .into_iter()
        .zip(flamegraphs.into_iter())
        .zip(json_reports.into_iter())
        .zip(traces.into_iter())
        .zip(html_reports.into_iter())
    {
        if args.format != PreviewPerfOutputFormat::Json {
            header!("Preview perf: {}", target.display_name());
        }
        let spinner = (args.format != PreviewPerfOutputFormat::Json)
            .then(|| shell::spinner("Building and profiling with hydrolysis..."))
            .flatten();
        let output = test_preview_with_hydrolysis(
            &project_path,
            target.hydrolysis_source(),
            args.theme.into(),
            HydrolysisPreviewTestMode::Perf,
            width,
            height,
            sccache_path.clone(),
            &automation_body,
            Some(HydrolysisPreviewPerfConfig {
                warmups: args.warmups,
                samples: args.samples,
                repetitions: args.repetitions,
            }),
            flamegraph.as_ref(),
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        let mut perf_report =
            parse_preview_perf_output(target.display_name().to_string(), &output)?;
        if let Some(flamegraph) = flamegraph.as_ref() {
            perf_report.flamegraph = Some(flamegraph.output_path.clone());
        }
        enforce_perf_budget(&perf_report, args.max_p95_us, args.max_rebuild_ratio)?;
        if args.format == PreviewPerfOutputFormat::Human {
            emit_preview_perf_human(&perf_report);
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
            success!("Preview perf report opened: {}", path.display());
        }
        if args.format != PreviewPerfOutputFormat::Json {
            if let Some(flamegraph) = flamegraph {
                success!(
                    "Preview perf passed: {} (flamegraph: {})",
                    target.display_name(),
                    flamegraph.output_path.display()
                );
            } else {
                success!("Preview perf passed: {}", target.display_name());
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
                success!("Preview perf report opened: {}", path.display());
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

    /// Preview target: a `#[preview]` function path or a WaterUI expression.
    target: Option<String>,

    /// Treat the target as a WaterUI expression returning `impl View`.
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
    /// Preview target: a `#[preview]` function path or a WaterUI expression.
    target: Option<String>,

    /// Discover and test every `#[preview]` function in the crate.
    #[arg(long)]
    all: bool,

    /// Treat the target as a WaterUI expression returning `impl View`.
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
    /// Preview target: a `#[preview]` function path or a WaterUI expression.
    target: Option<String>,

    /// Discover and profile every `#[preview]` function in the crate.
    #[arg(long)]
    all: bool,

    /// Treat the target as a WaterUI expression returning `impl View`.
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

#[derive(Debug, Serialize)]
struct PreviewPerfMeasurement {
    name: String,
    samples: u64,
    mean_us: u64,
    median_us: u64,
    p95_us: u64,
    min_us: u64,
    max_us: u64,
    rebuilt_frames: u64,
    missed_120fps_frames: u64,
    missed_60fps_frames: u64,
    measurement_cache_hits: u64,
    measurement_cache_misses: u64,
    scene_layers: u64,
    vello_scene_layers: u64,
    gpu_surface_layers: u64,
    clip_layers: u64,
    max_clip_depth: u64,
    phases: PreviewPerfPhases,
    frames: Vec<PreviewPerfFrame>,
}

#[derive(Debug, Default, Serialize)]
struct PreviewPerfPhases {
    rebuild_mean_us: u64,
    rebuild_p95_us: u64,
    render_mean_us: u64,
    render_p95_us: u64,
    animation_mean_us: u64,
    input_mean_us: u64,
}

#[derive(Debug, Serialize)]
struct PreviewPerfFrame {
    index: u64,
    total_us: u64,
    rebuild_us: u64,
    render_us: u64,
    acquire_us: u64,
    present_us: u64,
    animation_us: u64,
    input_us: u64,
    executor_before_us: u64,
    executor_after_us: u64,
    rebuilt: bool,
    rendered: bool,
    captured_snapshot: bool,
    cpu_percent: f64,
    memory_bytes: u64,
    gpu_frame_us: u64,
    measurement_cache_hits: u64,
    measurement_cache_misses: u64,
    scene_layers: u64,
    vello_scene_layers: u64,
    gpu_surface_layers: u64,
    clip_layers: u64,
    max_clip_depth: u64,
}

/// Run the preview command.
///
/// # Errors
/// Returns an error if preview fails.
pub async fn run(args: Args) -> Result<()> {
    match args.command {
        Some(PreviewCommand::Test(args)) => return run_preview_test(args).await,
        Some(PreviewCommand::Perf(args)) => return run_preview_perf(args).await,
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
    header!("Preview: {}", preview_target.display_name());

    check_toolchain_for_backend(platform, backend).await?;

    // Detect sccache for compilation caching
    let sccache = Sccache;
    let sccache_path = sccache.path().await.map_or_else(
        |_| {
            warn!(
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    );

    if backend == CliPreviewBackend::Hydrolysis {
        let scenario = load_hydrolysis_scenario(args.scenario.as_deref(), args.output_dir).await?;
        let spinner = shell::spinner("Building and rendering with hydrolysis...");
        render_preview_with_hydrolysis(
            &project_path,
            preview_target.hydrolysis_source(),
            hydrolysis_theme.expect("hydrolysis preview theme must be resolved"),
            width,
            height,
            sccache_path,
            &args.output,
            scenario.as_ref(),
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        if let Some(scenario) = scenario {
            success!("Preview frames saved to {}", scenario.output_dir.display());
        } else {
            success!("Preview saved to {}", args.output.display());
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
    let spinner = shell::spinner("Connecting to preview app...");
    let preview_platform: PreviewPlatform = platform.into();
    let mut session =
        launch_preview_session(&project_path, preview_platform, sccache_path.clone()).await?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    let result = async {
        // Build dylib
        let spinner = shell::spinner("Building project...");
        let dylib = session.build_dylib(&project_path).await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }

        let spinner = shell::spinner("Rendering view...");
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
            error!("Preview returned empty PNG data");
            bail!("Preview returned empty PNG data");
        }

        smol::fs::write(&args.output, &png_data).await?;
        success!("Preview saved to {}", args.output.display());
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
            session.shutdown().await?;
            Err(err)
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
        .into_iter()
        .map(parse_scenario_event)
        .collect::<Result<Vec<_>>>()?;
    events.sort_by_key(|event| event.at_ms);
    Ok(Some(HydrolysisPreviewScenario {
        captures_ms: scenario.captures_ms,
        events,
        output_dir,
    }))
}

fn parse_scenario_event(event: ScenarioEventFile) -> Result<HydrolysisPreviewScenarioEvent> {
    let kind = match event.kind.as_str() {
        "pointer_move" | "hover" => HydrolysisPreviewEventKind::PointerMove,
        "pointer_down" => HydrolysisPreviewEventKind::PointerDown,
        "pointer_up" => HydrolysisPreviewEventKind::PointerUp,
        "pointer_cancel" => HydrolysisPreviewEventKind::PointerCancel,
        other => bail!("unsupported Hydrolysis preview scenario event kind `{other}`"),
    };
    let button = event
        .button
        .as_deref()
        .map(|button| match button {
            "primary" => Ok(HydrolysisPreviewPointerButton::Primary),
            "secondary" => Ok(HydrolysisPreviewPointerButton::Secondary),
            "middle" => Ok(HydrolysisPreviewPointerButton::Middle),
            other => bail!("unsupported Hydrolysis preview pointer button `{other}`"),
        })
        .transpose()?;
    let needs_point = !matches!(kind, HydrolysisPreviewEventKind::PointerCancel);
    let x = match event.x {
        Some(x) => x,
        None if needs_point => bail!("Hydrolysis preview pointer event requires x coordinate"),
        None => 0.0,
    };
    let y = match event.y {
        Some(y) => y,
        None if needs_point => bail!("Hydrolysis preview pointer event requires y coordinate"),
        None => 0.0,
    };
    Ok(HydrolysisPreviewScenarioEvent {
        at_ms: event.at_ms,
        kind,
        x,
        y,
        button,
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
        (true, Some(_)) => bail!("`--all` cannot be combined with an explicit preview target."),
        (true, None) if force_expression => bail!("`--all` cannot be combined with `--expr`."),
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
        (false, None) => bail!("preview test/perf requires a target or `--all`."),
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
            message.push_str(&format!(
                "\n  `{name}` in {} and {}",
                first.display(),
                second.display()
            ));
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
            bail!("{command_name} accepts either `--code` or `--code-file`, not both.")
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
    frequency: i32,
) -> Result<Vec<Option<HydrolysisPreviewFlamegraph>>> {
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
            .map(|target| {
                Some(HydrolysisPreviewFlamegraph {
                    output_path: dir.join(format!("{}.svg", target.file_stem())),
                    frequency,
                })
            })
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
    Ok(vec![Some(HydrolysisPreviewFlamegraph {
        output_path,
        frequency,
    })])
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
            output
                .map(|path| path.with_extension("flamegraph.svg"))
                .unwrap_or_else(|| PathBuf::from("__waterui_default_flamegraph__"))
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
            let path = output
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::temp_dir().join("waterui-preview-perf.html"));
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
    let mut measurements = BTreeMap::<String, PreviewPerfMeasurement>::new();
    for line in output.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("perf ") {
            let (name, fields) = parse_named_perf_fields(rest)?;
            measurements.insert(
                name.clone(),
                PreviewPerfMeasurement {
                    name,
                    samples: parse_required_field(&fields, "samples")?,
                    mean_us: parse_required_field(&fields, "mean")?,
                    median_us: parse_required_field(&fields, "median")?,
                    p95_us: parse_required_field(&fields, "p95")?,
                    min_us: parse_required_field(&fields, "min")?,
                    max_us: parse_required_field(&fields, "max")?,
                    rebuilt_frames: parse_required_field(&fields, "rebuilt")?,
                    missed_120fps_frames: 0,
                    missed_60fps_frames: 0,
                    measurement_cache_hits: 0,
                    measurement_cache_misses: 0,
                    scene_layers: 0,
                    vello_scene_layers: 0,
                    gpu_surface_layers: 0,
                    clip_layers: 0,
                    max_clip_depth: 0,
                    phases: PreviewPerfPhases::default(),
                    frames: Vec::new(),
                },
            );
        } else if let Some(rest) = line.strip_prefix("perf-phases ") {
            let (name, fields) = parse_named_perf_fields(rest)?;
            let measurement = measurements.get_mut(&name).ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "Hydrolysis preview perf emitted phases before measurement `{name}`"
                )
            })?;
            measurement.phases = PreviewPerfPhases {
                rebuild_mean_us: parse_required_field(&fields, "rebuild_mean")?,
                rebuild_p95_us: parse_required_field(&fields, "rebuild_p95")?,
                render_mean_us: parse_required_field(&fields, "render_mean")?,
                render_p95_us: parse_required_field(&fields, "render_p95")?,
                animation_mean_us: parse_required_field(&fields, "animation_mean")?,
                input_mean_us: parse_required_field(&fields, "input_mean")?,
            };
            measurement.missed_120fps_frames = parse_required_field(&fields, "missed_120fps")?;
            measurement.missed_60fps_frames = parse_required_field(&fields, "missed_60fps")?;
            measurement.measurement_cache_hits =
                parse_required_field(&fields, "measurement_cache_hits")?;
            measurement.measurement_cache_misses =
                parse_required_field(&fields, "measurement_cache_misses")?;
            measurement.scene_layers = parse_required_field(&fields, "scene_layers")?;
            measurement.vello_scene_layers = parse_required_field(&fields, "vello_scene_layers")?;
            measurement.gpu_surface_layers = parse_required_field(&fields, "gpu_surface_layers")?;
            measurement.clip_layers = parse_required_field(&fields, "clip_layers")?;
            measurement.max_clip_depth = parse_required_field(&fields, "max_clip_depth")?;
        } else if let Some(rest) = line.strip_prefix("perf-sample ") {
            let (name, fields) = parse_named_perf_fields(rest)?;
            let measurement = measurements.get_mut(&name).ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "Hydrolysis preview perf emitted sample before measurement `{name}`"
                )
            })?;
            measurement.frames.push(PreviewPerfFrame {
                index: parse_required_field(&fields, "index")?,
                total_us: parse_required_field(&fields, "total")?,
                rebuild_us: parse_required_field(&fields, "rebuild")?,
                render_us: parse_required_field(&fields, "render")?,
                acquire_us: parse_required_field(&fields, "acquire")?,
                present_us: parse_required_field(&fields, "present")?,
                animation_us: parse_required_field(&fields, "animation")?,
                input_us: parse_required_field(&fields, "input")?,
                executor_before_us: parse_required_field(&fields, "executor_before")?,
                executor_after_us: parse_required_field(&fields, "executor_after")?,
                rebuilt: parse_required_field(&fields, "rebuilt")? != 0,
                rendered: parse_required_field(&fields, "rendered")? != 0,
                captured_snapshot: parse_required_field(&fields, "captured_snapshot")? != 0,
                cpu_percent: parse_required_field(&fields, "cpu_percent_milli")? as f64 / 1000.0,
                memory_bytes: parse_required_field(&fields, "memory_bytes")?,
                gpu_frame_us: parse_required_field(&fields, "gpu_frame")?,
                measurement_cache_hits: parse_required_field(&fields, "measurement_cache_hits")?,
                measurement_cache_misses: parse_required_field(
                    &fields,
                    "measurement_cache_misses",
                )?,
                scene_layers: parse_required_field(&fields, "scene_layers")?,
                vello_scene_layers: parse_required_field(&fields, "vello_scene_layers")?,
                gpu_surface_layers: parse_required_field(&fields, "gpu_surface_layers")?,
                clip_layers: parse_required_field(&fields, "clip_layers")?,
                max_clip_depth: parse_required_field(&fields, "max_clip_depth")?,
            });
        }
    }
    if measurements.is_empty() {
        bail!("Hydrolysis preview perf emitted no perf measurements");
    }
    Ok(PreviewPerfReport {
        target,
        measurements: measurements.into_values().collect(),
        flamegraph: None,
    })
}

fn emit_preview_perf_human(report: &PreviewPerfReport) {
    // This deliberately stays compact and regular: the default terminal format is optimized for
    // humans and LLM agents to scan, while stable machine consumption belongs to JSON mode.
    note!("Perf report: {}", report.target);
    for measurement in &report.measurements {
        note!(
            "  {}: samples={} mean={} median={} p95={} min={} max={} rebuilt={}/{} missed120={}/{} missed60={}/{}",
            measurement.name,
            measurement.samples,
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
            "    phases: rebuild mean={} p95={} | render mean={} p95={} | animation mean={} | input mean={}",
            micros_label(measurement.phases.rebuild_mean_us),
            micros_label(measurement.phases.rebuild_p95_us),
            micros_label(measurement.phases.render_mean_us),
            micros_label(measurement.phases.render_p95_us),
            micros_label(measurement.phases.animation_mean_us),
            micros_label(measurement.phases.input_mean_us)
        );
        note!(
            "    caches: measurement hits={} misses={}",
            measurement.measurement_cache_hits,
            measurement.measurement_cache_misses
        );
        note!(
            "    layers: compositor={} vello={} gpu-surface={} clip-pushes={} max-clip-depth={}",
            measurement.scene_layers,
            measurement.vello_scene_layers,
            measurement.gpu_surface_layers,
            measurement.clip_layers,
            measurement.max_clip_depth
        );
        if let Some(resources) = resource_summary(measurement) {
            note!(
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
        note!("  flamegraph: {}", flamegraph.display());
    }
}

fn micros_label(value: u64) -> String {
    format!("{value}us")
}

fn bytes_label(value: u64) -> String {
    const MIB: f64 = 1_048_576.0;
    format!("{:.1}MiB", value as f64 / MIB)
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
    let sample_count = measurement.frames.len() as f64;
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
            .map(|frame| frame.scene_layers as f64)
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
            .map(|frame| frame.clip_layers as f64)
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

fn parse_named_perf_fields(rest: &str) -> Result<(String, BTreeMap<String, u64>)> {
    let mut parts = rest.split_whitespace();
    let name = parts
        .next()
        .ok_or_else(|| color_eyre::eyre::eyre!("missing perf measurement name"))?
        .to_string();
    let mut fields = BTreeMap::new();
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            bail!("invalid perf field `{part}`");
        };
        let value = value
            .strip_suffix("us")
            .unwrap_or(value)
            .parse::<u64>()
            .map_err(|error| color_eyre::eyre::eyre!("invalid perf field `{part}`: {error}"))?;
        fields.insert(key.to_string(), value);
    }
    Ok((name, fields))
}

fn parse_required_field(fields: &BTreeMap<String, u64>, name: &str) -> Result<u64> {
    fields
        .get(name)
        .copied()
        .ok_or_else(|| color_eyre::eyre::eyre!("missing perf field `{name}`"))
}

fn enforce_perf_budget(
    report: &PreviewPerfReport,
    max_p95_us: Option<u64>,
    max_rebuild_ratio: Option<f64>,
) -> Result<()> {
    for measurement in &report.measurements {
        if let Some(max_p95_us) = max_p95_us
            && measurement.p95_us > max_p95_us
        {
            bail!(
                "Preview perf `{}` p95 {}us exceeded budget {}us",
                measurement.name,
                measurement.p95_us,
                max_p95_us
            );
        }
        if let Some(max_rebuild_ratio) = max_rebuild_ratio {
            let rebuild_ratio = measurement.rebuilt_frames as f64 / measurement.samples as f64;
            if rebuild_ratio > max_rebuild_ratio {
                bail!(
                    "Preview perf `{}` rebuild ratio {} exceeded budget {}",
                    measurement.name,
                    rebuild_ratio,
                    max_rebuild_ratio
                );
            }
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
    let total_measurements = reports
        .iter()
        .map(|report| report.measurements.len())
        .sum::<usize>();
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
    let html = PREVIEW_PERF_HTML_TEMPLATE
        .replace("{{REPORT_COUNT}}", &reports.len().to_string())
        .replace("{{MEASUREMENT_COUNT}}", &total_measurements.to_string())
        .replace("{{WORST_P95}}", &micros_label(worst_p95))
        .replace("{{MISSED_120}}", &missed_120.to_string())
        .replace("{{REPORTS}}", &report_cards);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        smol::fs::create_dir_all(parent).await?;
    }
    smol::fs::write(path, html).await?;
    Ok(())
}

fn render_preview_perf_report_html(report: &PreviewPerfReport) -> String {
    let measurements = report
        .measurements
        .iter()
        .map(|measurement| {
            render_preview_perf_measurement_html(measurement, report.flamegraph.as_deref())
        })
        .collect::<String>();
    format!(
        "<section class=\"report\"><h2>{}</h2><div class=\"measurements\">{}</div></section>",
        html_escape(report.target.as_str()),
        measurements
    )
}

fn render_preview_perf_flamegraph_html(flamegraph: Option<&Path>) -> String {
    let Some(flamegraph) = flamegraph else {
        return String::new();
    };
    let path = html_escape(&flamegraph.to_string_lossy());
    format!(
        concat!(
            "<section class=\"flamegraph\">",
            "<div><h3>Flamegraph</h3><a href=\"{}\">Open SVG</a></div>",
            "<div class=\"flamegraph-viewport\"><object data=\"{}\" type=\"image/svg+xml\"></object></div>",
            "</section>"
        ),
        path, path
    )
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
    format!(
        concat!(
            "<article class=\"measurement\">",
            "<div class=\"measurement-head\"><h3>{}</h3><span>{}</span></div>",
            "<nav class=\"tabs\" role=\"tablist\" aria-label=\"Performance views\">",
            "<button class=\"active\" type=\"button\" role=\"tab\" aria-selected=\"true\" data-tab-target=\"overview\">Overview</button>",
            "<button type=\"button\" role=\"tab\" aria-selected=\"false\" data-tab-target=\"frames\">Frames</button>",
            "<button type=\"button\" role=\"tab\" aria-selected=\"false\" data-tab-target=\"resources\">Resources</button>",
            "<button type=\"button\" role=\"tab\" aria-selected=\"false\" data-tab-target=\"flamegraph\">Flamegraph</button>",
            "</nav>",
            "<section class=\"tab-panel active\" data-tab-panel=\"overview\">{}{}",
            "</section>",
            "<section class=\"tab-panel\" data-tab-panel=\"frames\">{}",
            "</section>",
            "<section class=\"tab-panel\" data-tab-panel=\"resources\">{}",
            "</section>",
            "<section class=\"tab-panel\" data-tab-panel=\"flamegraph\">{}",
            "</section>",
            "</article>"
        ),
        html_escape(measurement.name.as_str()),
        preview_perf_budget_label(measurement),
        diagnosis,
        phase_stack,
        frame_timeline,
        resources,
        flamegraph
    )
}

fn preview_perf_budget_label(measurement: &PreviewPerfMeasurement) -> &'static str {
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
    let worst_frame = worst
        .map(|frame| format!("frame {} / {}", frame.index, micros_label(frame.total_us)))
        .unwrap_or_else(|| "none".to_string());
    format!(
        concat!(
            "<section class=\"diagnosis\">",
            "<div><span>p95</span><strong>{}</strong><small>mean {} / median {}</small></div>",
            "<div><span>worst sample</span><strong>{}</strong><small>{} samples</small></div>",
            "<div><span>bottleneck</span><strong>{}</strong><small>{}</small></div>",
            "<div><span>rebuild pressure</span><strong>{:.1}%</strong><small>{}/{} frames</small></div>",
            "<div><span>layout cache</span><strong>{:.1}% hit</strong><small>{} hits / {} misses</small></div>",
            "</section>"
        ),
        micros_label(measurement.p95_us),
        micros_label(measurement.mean_us),
        micros_label(measurement.median_us),
        worst_frame,
        measurement.samples,
        bottleneck.name,
        micros_label(bottleneck.mean_us),
        rebuild_ratio,
        measurement.rebuilt_frames,
        measurement.samples,
        cache_hit_ratio,
        measurement.measurement_cache_hits,
        measurement.measurement_cache_misses
    )
}

struct PreviewPerfBottleneck {
    name: &'static str,
    mean_us: u64,
}

fn preview_perf_bottleneck(measurement: &PreviewPerfMeasurement) -> PreviewPerfBottleneck {
    [
        ("render", measurement.phases.render_mean_us),
        ("rebuild", measurement.phases.rebuild_mean_us),
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
        |frame| f64::from(frame.cpu_percent),
        |value| format!("{value:.1}%"),
    );
    let memory_chart = render_preview_perf_metric_chart_html(
        "Memory",
        "line-memory",
        &measurement.frames,
        |frame| frame.memory_bytes as f64 / 1_048_576.0,
        |value| format!("{value:.1} MiB"),
    );
    let gpu_chart = render_preview_perf_metric_chart_html(
        "GPU pipeline",
        "line-gpu",
        &measurement.frames,
        |frame| frame.gpu_frame_us as f64,
        |value| micros_label(value.round() as u64),
    );
    let layer_chart = render_preview_perf_metric_chart_html(
        "Compositor layers",
        "line-layers",
        &measurement.frames,
        |frame| frame.scene_layers as f64,
        |value| format!("{value:.0}"),
    );
    let clip_chart = render_preview_perf_metric_chart_html(
        "Clip layers",
        "line-clip",
        &measurement.frames,
        |frame| frame.clip_layers as f64,
        |value| format!("{value:.0}"),
    );
    format!(
        concat!(
            "<section class=\"resources\">",
            "<div><span>CPU avg</span><strong>{:.1}%</strong></div>",
            "<div><span>CPU max</span><strong>{:.1}%</strong></div>",
            "<div><span>memory max</span><strong>{}</strong></div>",
            "<div><span>GPU pipeline avg</span><strong>{}</strong></div>",
            "<div><span>GPU pipeline max</span><strong>{}</strong></div>",
            "<div><span>layer avg</span><strong>{:.1}</strong></div>",
            "<div><span>layer max</span><strong>{}</strong></div>",
            "<div><span>clip avg</span><strong>{:.1}</strong></div>",
            "<div><span>clip depth max</span><strong>{}</strong></div>",
            "</section>",
            "<section class=\"trend-grid\">{}{}{}{}{}</section>"
        ),
        summary.avg_cpu_percent,
        summary.max_cpu_percent,
        bytes_label(summary.max_memory_bytes),
        micros_label(summary.avg_gpu_frame_us),
        micros_label(summary.max_gpu_frame_us),
        summary.avg_scene_layers,
        summary.max_scene_layers,
        summary.avg_clip_layers,
        summary.max_clip_depth,
        cpu_chart,
        memory_chart,
        gpu_chart,
        layer_chart,
        clip_chart
    )
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
                frame.total_us as f64,
                frame.render_us as f64,
                frame.rebuild_us as f64,
                frame.gpu_frame_us as f64,
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
            frame.total_us as f64
        });
    let render_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            frame.render_us as f64
        });
    let rebuild_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            frame.rebuild_us as f64
        });
    let gpu_points =
        render_preview_perf_float_polyline_points(&measurement.frames, timing_scale, |frame| {
            frame.gpu_frame_us as f64
        });
    let points = measurement
        .frames
        .iter()
        .map(|frame| {
            let x = frame_chart_x(frame.index, measurement.frames.len());
            let y = timing_scale.y(frame.total_us as f64);
            format!(
                concat!(
                    "<circle class=\"sample-point\" cx=\"{:.2}\" cy=\"{:.2}\" r=\"2.4\" tabindex=\"0\" ",
                    "data-frame=\"{}\" data-total=\"{}\" data-gpu=\"{}\" data-render=\"{}\" data-rebuild=\"{}\" data-cpu=\"{:.1}%\" data-memory=\"{}\" data-fps=\"{}\" data-layers=\"{}\" data-clip-layers=\"{}\" data-clip-depth=\"{}\"></circle>"
                ),
                x,
                y,
                frame.index,
                micros_label(frame.total_us),
                micros_label(frame.gpu_frame_us),
                micros_label(frame.render_us),
                micros_label(frame.rebuild_us),
                frame.cpu_percent,
                bytes_label(frame.memory_bytes),
                fps_label(preview_perf_throughput_fps(frame)),
                frame.scene_layers,
                frame.clip_layers,
                frame.max_clip_depth
            )
        })
        .collect::<String>();
    let fps_sample_points = measurement
        .frames
        .iter()
        .map(|frame| {
            let x = frame_chart_x(frame.index, measurement.frames.len());
            let y = fps_scale.y(preview_perf_throughput_fps(frame).min(1_000.0));
            format!(
                concat!(
                    "<circle class=\"sample-point\" cx=\"{:.2}\" cy=\"{:.2}\" r=\"2.4\" tabindex=\"0\" ",
                    "data-frame=\"{}\" data-total=\"{}\" data-gpu=\"{}\" data-render=\"{}\" data-rebuild=\"{}\" data-cpu=\"{:.1}%\" data-memory=\"{}\" data-fps=\"{}\" data-layers=\"{}\" data-clip-layers=\"{}\" data-clip-depth=\"{}\"></circle>"
                ),
                x,
                y,
                frame.index,
                micros_label(frame.total_us),
                micros_label(frame.gpu_frame_us),
                micros_label(frame.render_us),
                micros_label(frame.rebuild_us),
                frame.cpu_percent,
                bytes_label(frame.memory_bytes),
                fps_label(preview_perf_throughput_fps(frame)),
                frame.scene_layers,
                frame.clip_layers,
                frame.max_clip_depth
            )
        })
        .collect::<String>();
    let timing_chart = format!(
        concat!(
            "<section class=\"line-chart\">",
            "<div><h4>Frame time</h4><span>{} - {}</span></div>",
            "<svg viewBox=\"0 0 100 100\" role=\"img\" aria-label=\"Raw frame timing trend\">",
            "{}",
            "<polyline class=\"line-total\" points=\"{}\"></polyline>",
            "<polyline class=\"line-gpu\" points=\"{}\"></polyline>",
            "<polyline class=\"line-render\" points=\"{}\"></polyline>",
            "<polyline class=\"line-rebuild\" points=\"{}\"></polyline>",
            "<g class=\"sample-points\">{}</g>",
            "</svg>",
            "<div class=\"hover-inspector\" aria-live=\"polite\">Hover a sample</div>",
            "<div class=\"legend\"><span class=\"total\">total</span><span class=\"gpu\">GPU pipeline</span><span class=\"render\">render</span><span class=\"rebuild\">rebuild</span><span class=\"budget-label\">budget markers appear when inside range</span></div>",
            "</section>"
        ),
        micros_label(timing_scale.min.round() as u64),
        micros_label(timing_scale.max.round() as u64),
        render_preview_perf_budget_lines(timing_scale),
        total_points,
        gpu_points,
        render_points,
        rebuild_points,
        points
    );
    let fps_chart = format!(
        concat!(
            "<section class=\"line-chart\">",
            "<div><h4>Throughput FPS</h4><span>{} - {}</span></div>",
            "<svg viewBox=\"0 0 100 100\" role=\"img\" aria-label=\"Throughput FPS trend\">",
            "{}",
            "<polyline class=\"line-fps\" points=\"{}\"></polyline>",
            "<g class=\"sample-points\">{}</g>",
            "</svg>",
            "<div class=\"hover-inspector\" aria-live=\"polite\">Hover a frame</div>",
            "<div class=\"legend\"><span class=\"fps\">offscreen throughput FPS</span><span class=\"budget-label\">display budgets appear when inside range</span></div>",
            "</section>"
        ),
        fps_label(fps_scale.min),
        fps_label(fps_scale.max),
        render_preview_perf_fps_budget_lines(fps_scale),
        fps_points,
        fps_sample_points
    );
    format!(
        "<section class=\"timeline-grid\">{}{}</section>",
        timing_chart, fps_chart
    )
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
            "rebuild",
            measurement.phases.rebuild_mean_us,
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
        .map(|(name, value, class)| {
            let width = ratio_percent(*value, total).max(0.5);
            format!(
                "<i class=\"{}\" style=\"width:{:.2}%\"><span>{} {}</span></i>",
                class,
                width,
                name,
                micros_label(*value)
            )
        })
        .collect::<String>();
    format!(
        concat!(
            "<section class=\"phase-stack\">",
            "<div><h4>Phase breakdown</h4><span>mean frame work</span></div>",
            "<div class=\"phase-bar\">{}</div>",
            "</section>"
        ),
        segments
    )
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
            format!(
                concat!(
                    "<circle class=\"sample-point\" cx=\"{:.2}\" cy=\"{:.2}\" r=\"2.4\" tabindex=\"0\" ",
                    "data-frame=\"{}\" data-total=\"{}\" data-gpu=\"{}\" data-render=\"{}\" data-rebuild=\"{}\" data-cpu=\"{:.1}%\" data-memory=\"{}\" data-fps=\"{}\" data-layers=\"{}\" data-clip-layers=\"{}\" data-clip-depth=\"{}\" data-value=\"{}\"></circle>"
                ),
                frame_chart_x(frame.index, frames.len()),
                scale.y(current_value),
                frame.index,
                micros_label(frame.total_us),
                micros_label(frame.gpu_frame_us),
                micros_label(frame.render_us),
                micros_label(frame.rebuild_us),
                frame.cpu_percent,
                bytes_label(frame.memory_bytes),
                fps_label(preview_perf_throughput_fps(frame)),
                frame.scene_layers,
                frame.clip_layers,
                frame.max_clip_depth,
                html_escape(&label(current_value))
            )
        })
        .collect::<String>();
    format!(
        concat!(
            "<section class=\"line-chart metric-chart\">",
            "<div><h4>{}</h4><span>{} - {}</span></div>",
            "<svg viewBox=\"0 0 100 100\" role=\"img\" aria-label=\"{} trend\">",
            "<polyline class=\"{}\" points=\"{}\"></polyline>",
            "<g class=\"sample-points\">{}</g>",
            "</svg>",
            "<div class=\"hover-inspector\" aria-live=\"polite\">Hover a frame</div>",
            "</section>"
        ),
        html_escape(title),
        html_escape(&label(actual_min)),
        html_escape(&label(actual_max)),
        html_escape(title),
        line_class,
        points,
        samples
    )
}

fn frame_chart_x(index: u64, frame_count: usize) -> f64 {
    if frame_count <= 1 {
        return 6.0;
    }
    6.0 + ((index as f64 / (frame_count - 1) as f64) * 88.0)
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
        92.0 - (((clamped - self.min) / (self.max - self.min)) * 84.0)
    }
}

fn render_preview_perf_budget_lines(scale: PreviewPerfChartScale) -> String {
    [(8_333.0, "budget-120"), (16_666.0, "budget-60")]
        .into_iter()
        .filter(|(value, _)| scale.contains(*value))
        .map(|(value, class)| {
            let y = scale.y(value);
            format!(
                "<line class=\"budget {class}\" x1=\"6\" y1=\"{y:.2}\" x2=\"94\" y2=\"{y:.2}\"></line>"
            )
        })
        .collect()
}

fn render_preview_perf_fps_budget_lines(scale: PreviewPerfChartScale) -> String {
    [(120.0, "budget-120"), (60.0, "budget-60")]
        .into_iter()
        .filter(|(value, _)| scale.contains(*value))
        .map(|(value, class)| {
            let y = scale.y(value);
            format!(
                "<line class=\"budget {class}\" x1=\"6\" y1=\"{y:.2}\" x2=\"94\" y2=\"{y:.2}\"></line>"
            )
        })
        .collect()
}

fn preview_perf_throughput_fps(frame: &PreviewPerfFrame) -> f64 {
    1_000_000.0 / frame.total_us.max(1) as f64
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
    (numerator as f64 / denominator as f64) * 100.0
}

fn html_escape(value: &str) -> String {
    askama::filters::escape(value, askama::filters::Html)
        .expect("HTML escaping should be infallible")
        .to_string()
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

async fn resolve_sccache_path() -> Option<PathBuf> {
    let sccache = Sccache;
    sccache.path().await.map_or_else(
        |_| {
            warn!(
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    )
}

fn emit_child_output(output: &str) {
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        note!("{line}");
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

fn resolve_preview_platform(
    platform_override: Option<CliPreviewPlatform>,
) -> Result<CliPreviewPlatform> {
    if let Some(platform) = platform_override {
        return Ok(platform);
    }
    native_preview_platform()
}

fn native_preview_platform() -> Result<CliPreviewPlatform> {
    #[cfg(target_os = "macos")]
    {
        return Ok(CliPreviewPlatform::Macos);
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
            )
        }
        (_, Some(_)) => bail!("`--theme` is only supported with `--backend hydrolysis`."),
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
                    bail!("Internal error: Apple preview backend is not supported on android")
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

#[allow(clippy::too_many_arguments)]
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
            bail!("{}", missing_preview_symbol_message(function_path, symbol))
        }
        Err(err) => bail!("Preview app error: {err}"),
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

    #[test]
    fn formats_missing_preview_symbol_message() {
        let symbol = "waterui_preview_app_dashboard_admin_card_preview";
        let message = missing_preview_symbol_message("dashboard::admin::card_preview", symbol);
        assert!(message.contains("dashboard::admin::card_preview"));
        assert!(message.contains("waterui_preview_app_dashboard_admin_card_preview"));
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
        assert_eq!(symbol, "waterui_preview_my_crate_dashboard_card");
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
