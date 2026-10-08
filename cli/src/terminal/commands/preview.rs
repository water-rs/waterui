//! `water preview` command implementation.
//!
//! Renders or semantically tests a `WaterUI` preview.

use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, Subcommand};
use eyre::{Result, bail};
use serde::Deserialize;

use crate::shell::Shell;
use crate::{error, header, note, success};
use waterui_cli::build::BuildProgress;
use waterui_cli::mcp::preview::PreviewArgs;
use waterui_cli::preview::request::{
    self, CliHydrolysisPreviewTheme, CliPreviewBackend, CliPreviewPlatform, PreviewTarget,
    ResolvedPreviewBackend,
};
use waterui_cli::preview::{
    ApplePreviewRequest, HydrolysisAndroidPreviewRequest, HydrolysisPreviewEventKind,
    HydrolysisPreviewPointerButton, HydrolysisPreviewRequest, HydrolysisPreviewScenario,
    HydrolysisPreviewScenarioEvent, HydrolysisPreviewTheme, discover_hydrolysis_preview_exports,
    launch_preview_session, render_preview_with_apple, render_preview_with_hydrolysis,
    render_preview_with_hydrolysis_android, test_preview_with_hydrolysis,
};
use waterui_cli::project::read_project_crate_name;

async fn run_preview_test(shell: &Shell, args: PreviewTestArgs) -> Result<()> {
    let platform = request::resolve_preview_platform(args.platform)?;
    let target_platform = request::resolve_hydrolysis_test_platform(platform)?;
    request::check_toolchain_for_backend(ResolvedPreviewBackend::Hydrolysis(target_platform))
        .await?;
    let (width, height) = request::parse_frame(&args.frame)?;
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let crate_name = read_project_crate_name(&project_path).await?;
    let sccache_path =
        super::detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await;
    let targets = resolve_test_targets(
        &project_path,
        &crate_name,
        &args,
        target_platform,
        sccache_path.as_deref(),
        Some(&shell.build_progress()),
    )
    .await?;
    let automation_body = load_automation_body(
        args.code.as_deref(),
        args.code_file.as_deref(),
        "",
        "`water preview test`",
    )
    .await?;
    for target in targets {
        header!(shell, "Preview test: {}", target.display_name());
        let spinner = shell.spinner("Building and testing with hydrolysis...");
        let output = test_preview_with_hydrolysis(
            HydrolysisPreviewRequest {
                project_path: &project_path,
                source: target.source(),
                theme: args.theme.into(),
                platform: target_platform,
                width,
                height,
                sccache_path: sccache_path.clone(),
                progress: Some(shell.build_progress()),
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
    #[arg(short, long, default_value = request::DEFAULT_FRAME)]
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

impl Args {
    /// The shared preview arguments — the same shape the MCP `preview` tool
    /// accepts, so `water preview` and `tools/call preview` resolve identically.
    fn preview_args(&self, target: &str) -> PreviewArgs {
        PreviewArgs {
            target: target.to_string(),
            expr: self.expr,
            frame: Some(self.frame.clone()),
            backend: self.backend,
            theme: self.theme,
            platform: self.platform,
        }
    }
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
    #[arg(long, value_enum, default_value = "material3")]
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

    // Canonicalize project path
    let project_path = crate::project_path::canonicalize(&args.path)?;

    let crate_name = read_project_crate_name(&project_path).await?;

    // Resolve through the shared `PreviewArgs` contract — the same arguments
    // the MCP `preview` tool takes.
    let request = args.preview_args(target).resolve(&crate_name)?;
    header!(shell, "Preview: {}", request.target.display_name());

    request::check_toolchain_for_backend(request.backend).await?;

    // Detect sccache for compilation caching
    let sccache_path =
        super::detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await;

    if let ResolvedPreviewBackend::Hydrolysis(platform) = request.backend {
        let scenario = load_hydrolysis_scenario(args.scenario.as_deref(), args.output_dir).await?;
        let spinner = shell.spinner("Building and rendering with hydrolysis...");
        render_preview_with_hydrolysis(
            HydrolysisPreviewRequest {
                project_path: &project_path,
                source: request.target.source(),
                theme: request
                    .hydrolysis_theme
                    .expect("hydrolysis preview theme must be resolved"),
                platform,
                width: request.width,
                height: request.height,
                sccache_path,
                progress: Some(shell.build_progress()),
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

    if request.backend == ResolvedPreviewBackend::HydrolysisAndroid {
        let scenario = load_hydrolysis_scenario(args.scenario.as_deref(), args.output_dir).await?;
        let spinner = shell.spinner("Building and rendering with hydrolysis...");
        render_preview_with_hydrolysis_android(
            HydrolysisAndroidPreviewRequest {
                project_path: &project_path,
                source: request.target.source(),
                theme: request
                    .hydrolysis_theme
                    .expect("hydrolysis preview theme must be resolved"),
                width: request.width,
                height: request.height,
                sccache_path,
                progress: Some(shell.build_progress()),
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

    if request.backend == ResolvedPreviewBackend::Apple {
        let spinner = shell.spinner("Building and rendering with the Apple backend...");
        render_preview_with_apple(
            ApplePreviewRequest {
                project_path: &project_path,
                source: request.target.source(),
                width: request.width,
                height: request.height,
                sccache_path,
                progress: Some(shell.build_progress()),
            },
            &args.output,
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        success!(shell, "Preview saved to {}", args.output.display());
        return Ok(());
    }

    let PreviewTarget::Function {
        function_path,
        symbol,
    } = &request.target
    else {
        bail!("Expression preview is currently supported only with `--backend hydrolysis`.");
    };

    let ResolvedPreviewBackend::SupportApp(preview_platform) = request.backend else {
        unreachable!("the Hydrolysis arm returned early");
    };

    // Launch preview session (connects to existing app or launches new one)
    let spinner = shell.spinner("Connecting to preview app...");
    let mut session = Box::pin(launch_preview_session(
        &project_path,
        preview_platform,
        sccache_path.clone(),
        Some(shell.build_progress()),
    ))
    .await?;
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
        let png_data = request::render_with_symbol(
            &mut session,
            function_path,
            symbol,
            dylib.id,
            &dylib.path,
            request.width,
            request.height,
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

async fn resolve_test_targets(
    project_path: &Path,
    crate_name: &str,
    args: &PreviewTestArgs,
    target_platform: waterui_cli::platform::TargetPlatform,
    sccache_path: Option<&Path>,
    progress: Option<&BuildProgress>,
) -> Result<Vec<PreviewTarget>> {
    match (args.all, args.target.as_deref()) {
        (true, Some(_)) => {
            bail!("`--all` cannot be combined with an explicit preview target.");
        }
        (true, None) if args.expr => {
            bail!("`--all` cannot be combined with `--expr`.");
        }
        (true, None) => {
            discover_preview_targets(
                project_path,
                crate_name,
                args.theme.into(),
                target_platform,
                sccache_path,
                progress,
            )
            .await
        }
        (false, Some(target)) => {
            if args.expr {
                Ok(vec![PreviewTarget::Expression {
                    expression: target.to_string(),
                }])
            } else {
                Ok(vec![request::resolve_preview_target(
                    crate_name, target, false,
                )])
            }
        }
        (false, None) => {
            bail!("preview test requires a target or `--all`.");
        }
    }
}

/// Probe-build the project through its hydrolysis backend and read the
/// `waterui_preview_*` exports off the app library the build produced. The
/// probe shares its Cargo profile with the per-target builds `run_preview_test`
/// then performs, so discovery costs one warm build.
async fn discover_preview_targets(
    project_path: &Path,
    crate_name: &str,
    theme: HydrolysisPreviewTheme,
    target_platform: waterui_cli::platform::TargetPlatform,
    sccache_path: Option<&Path>,
    progress: Option<&BuildProgress>,
) -> Result<Vec<PreviewTarget>> {
    let exports = discover_hydrolysis_preview_exports(
        project_path,
        theme,
        target_platform,
        sccache_path.map(Path::to_path_buf),
        progress.cloned(),
    )
    .await?;
    // `#[preview]` exports `waterui_preview_<crate>_<fn>`; crate names are
    // normalized like `function_path_to_symbol` does (dashes become
    // underscores).
    let prefix = format!("waterui_preview_{}_", crate_name.replace('-', "_"));
    let found: Vec<String> = exports
        .into_iter()
        .filter(|symbol| symbol.starts_with(&prefix))
        .collect();
    if found.is_empty() {
        bail!("no `waterui_preview_*` exports found in {project_path:?}");
    }
    Ok(found
        .into_iter()
        .map(|symbol| PreviewTarget::Function {
            function_path: symbol[prefix.len()..].to_string(),
            symbol,
        })
        .collect())
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct PreviewCommandLine {
        #[command(flatten)]
        args: Args,
    }

    fn parse(args: &[&str]) -> Args {
        PreviewCommandLine::try_parse_from(args)
            .expect("args parse")
            .args
    }

    /// This host's desktop preview platform label — a resolved `--platform`
    /// must name the host OS, so tests substitute it rather than hardcoding
    /// one.
    fn host_platform_name() -> &'static str {
        match std::env::consts::OS {
            "macos" | "linux" | "windows" => std::env::consts::OS,
            other => panic!("test host {other} has no native preview platform"),
        }
    }

    #[test]
    fn cli_and_mcp_args_resolve_to_the_same_request() {
        // `water preview --expr --frame 800x600 --backend hydrolysis --theme
        // material3 --platform <host> 'text("hi")'` and the equivalent MCP
        // `preview` call must produce the identical render request.
        let platform = host_platform_name();
        let cli_args = parse(&[
            "preview",
            "text(\"hi\")",
            "--expr",
            "--frame",
            "800x600",
            "--backend",
            "hydrolysis",
            "--theme",
            "material3",
            "--platform",
            platform,
        ]);
        let cli_request = cli_args
            .preview_args(cli_args.target.as_deref().expect("target"))
            .resolve("demo_app")
            .expect("cli resolve");

        let mcp_args: PreviewArgs = serde_json::from_value(serde_json::json!({
            "target": "text(\"hi\")",
            "expr": true,
            "frame": "800x600",
            "backend": "hydrolysis",
            "theme": "material3",
            "platform": platform,
        }))
        .expect("mcp args parse");
        let mcp_request = mcp_args.resolve("demo_app").expect("mcp resolve");

        assert_eq!(cli_request, mcp_request);
    }

    #[test]
    fn cli_and_mcp_defaults_resolve_to_the_same_request() {
        let platform = host_platform_name();
        let cli_args = parse(&["preview", "views::home", "--platform", platform]);
        let cli_request = cli_args
            .preview_args(cli_args.target.as_deref().expect("target"))
            .resolve("demo_app")
            .expect("cli resolve");

        let mcp_args: PreviewArgs = serde_json::from_value(serde_json::json!({
            "target": "views::home",
            "platform": platform,
        }))
        .expect("mcp args parse");
        let mcp_request = mcp_args.resolve("demo_app").expect("mcp resolve");

        assert_eq!(cli_request, mcp_request);
    }

    #[test]
    fn rejects_non_positive_frame_values() {
        assert!(request::parse_frame("0x100").is_err());
        assert!(request::parse_frame("-1x100").is_err());
        assert!(request::parse_frame("100x0").is_err());
        assert!(request::parse_frame("100x-1").is_err());
    }

    #[test]
    fn rejects_non_finite_frame_values() {
        assert!(request::parse_frame("NaNx100").is_err());
        assert!(request::parse_frame("100xinf").is_err());
    }

    #[test]
    fn resolves_plain_path_as_preview_function() {
        let target = request::resolve_preview_target("my-crate", "dashboard::card", false);
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
        let target = request::resolve_preview_target("my-crate", "button(\"Save\")", false);
        let PreviewTarget::Expression { expression } = target else {
            panic!("expected expression target");
        };
        assert_eq!(expression, "button(\"Save\")");
    }

    #[test]
    fn expr_flag_forces_identifier_as_expression_preview() {
        let target = request::resolve_preview_target("my-crate", "main_view", true);
        let PreviewTarget::Expression { expression } = target else {
            panic!("expected expression target");
        };
        assert_eq!(expression, "main_view");
    }

    #[test]
    fn hydrolysis_preview_theme_defaults_to_material3() {
        let result = request::resolve_hydrolysis_preview_theme(
            ResolvedPreviewBackend::Hydrolysis(waterui_cli::platform::TargetPlatform::Linux),
            None,
        );
        assert_eq!(result.unwrap(), Some(HydrolysisPreviewTheme::Material3));
    }

    #[test]
    fn rejects_theme_for_non_hydrolysis_preview() {
        let result = request::resolve_hydrolysis_preview_theme(
            ResolvedPreviewBackend::SupportApp(waterui_cli::preview::PreviewPlatform::Macos),
            Some(CliHydrolysisPreviewTheme::Material3),
        );
        assert!(result.is_err());
    }
}
