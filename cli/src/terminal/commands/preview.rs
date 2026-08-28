//! `water preview` command implementation.
//!
//! Renders or semantically tests a `WaterUI` preview.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use color_eyre::eyre::{Result, bail};
use ignore::WalkBuilder;
use serde::Deserialize;
use syn::{Attribute, Item};

use crate::shell::Shell;
use crate::toolchain_checks;
use crate::{error, header, note, success};
use waterui_cli::preview::protocol::{AppError, DylibId, function_path_to_symbol};
use waterui_cli::preview::{
    HydrolysisPreviewEventKind, HydrolysisPreviewPointerButton, HydrolysisPreviewRequest,
    HydrolysisPreviewScenario, HydrolysisPreviewScenarioEvent, HydrolysisPreviewSource,
    HydrolysisPreviewTheme, PreviewPlatform, PreviewSession, launch_preview_session,
    render_preview_with_hydrolysis, test_preview_with_hydrolysis,
};

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
    let sccache_path = super::detect_sccache_path(shell).await;

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
        None => {}
    }

    let Some(target) = args.target.as_deref() else {
        bail!(
            "`water preview` requires a target. Use `water preview <target>` or `water preview test`."
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
    let sccache_path = super::detect_sccache_path(shell).await;

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
            bail!("preview test requires a target or `--all`.");
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

// Both lints are host-dependent, so neither `expect` can be fulfilled everywhere:
// on macOS the body is an infallible `const`-compatible `Ok`, while every other host
// bails at runtime with an unsupported-host error.
#[allow(
    clippy::unnecessary_wraps,
    reason = "non-macOS hosts return an explicit unsupported-host error"
)]
#[allow(
    clippy::missing_const_for_fn,
    reason = "non-macOS hosts call the non-const `bail!`"
)]
fn native_preview_platform() -> Result<CliPreviewPlatform> {
    #[cfg(target_os = "macos")]
    {
        Ok(CliPreviewPlatform::Macos)
    }

    #[cfg(not(target_os = "macos"))]
    {
        // `bail!` expands to a `return`, so the trailing semicolon keeps this a
        // statement rather than a macro invocation in expression position.
        bail!(
            "No native preview platform is configured for this host. Pass `--platform` explicitly."
        );
    }
}

fn ensure_hydrolysis_preview_platform(platform: CliPreviewPlatform) -> Result<()> {
    if platform != CliPreviewPlatform::Macos {
        bail!("`water preview test` supports Hydrolysis on macos only.");
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
