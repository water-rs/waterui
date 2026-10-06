use std::path::{Path, PathBuf};

use askama::Template;
use eyre::{Context as _, Result, bail};

use crate::artifact_symbols::ArtifactSymbols;
use crate::backend::reinit_backend;
use crate::build::{BuildOptions, BuildProfile, BuildProgress, BuiltTarget};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::hydrolysis::platform::{
    build_hydrolysis_with_envs_and_features, stage_hydrolysis_shared_runtime,
};
use crate::platform::{TargetBackend, TargetPlatform};
use crate::project::{ManagedBackends, Project};
use crate::project_model::assets;
use crate::utils::command;

const HYDROLYSIS_PREVIEW_FEATURE: &str = "waterui-preview-mode";
const HYDROLYSIS_PREVIEW_TEST_FEATURE: &str = "waterui-preview-test-mode";

use waterui_preview_protocol::run::{PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode};
pub use waterui_preview_protocol::run::{
    ScenarioEvent as HydrolysisPreviewScenarioEvent,
    ScenarioEventKind as HydrolysisPreviewEventKind,
    ScenarioPointerButton as HydrolysisPreviewPointerButton,
};

/// Theme package selected for Hydrolysis preview rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisPreviewTheme {
    /// Material Design 3 package.
    Material3,
}

impl HydrolysisPreviewTheme {
    /// The Rust expression producing the `hydrolysis::Style` value the
    /// preview runtimes are constructed with.
    const fn style(self) -> &'static str {
        match self {
            Self::Material3 => "hydrolysis_m3::Material3::defaults()",
        }
    }

    fn font_declarations(self) -> Vec<assets::FontDeclaration> {
        match self {
            Self::Material3 => vec![assets::FontDeclaration {
                name: "Roboto".to_string(),
                source: assets::FontSource::BuiltIn,
                crate_name: "hydrolysis-m3".to_string(),
            }],
        }
    }
}

/// Source used to produce a Hydrolysis preview view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisPreviewSource<'a> {
    /// Existing `#[preview]` export symbol.
    Symbol(&'a str),
    /// Inline Rust expression returning `impl View`.
    Expression(&'a str),
}

/// Interactive capture scenario for Hydrolysis preview.
#[derive(Debug, Clone, PartialEq)]
pub struct HydrolysisPreviewScenario {
    /// Capture timestamps in milliseconds from scenario start.
    pub captures_ms: Vec<u64>,
    /// Input events sorted by timestamp.
    pub events: Vec<HydrolysisPreviewScenarioEvent>,
    /// Directory where captured frames are written.
    pub output_dir: PathBuf,
}

#[derive(Template)]
#[template(
    path = "src/preview/hydrolysis_preview_bindings.rs.tpl",
    escape = "none"
)]
struct HydrolysisPreviewBindingsTemplate<'a> {
    expression_mode: bool,
    preview_symbol: &'a str,
    preview_expression: &'a str,
    crate_name_ident: &'a str,
    preview_theme_style: &'a str,
    include_automation: bool,
    semantic_automation_body: &'a str,
}

/// Common inputs for driving the managed Hydrolysis preview backend.
#[derive(Debug, Clone)]
pub struct HydrolysisPreviewRequest<'a> {
    /// `WaterUI` project directory.
    pub project_path: &'a Path,
    /// Preview view source.
    pub source: HydrolysisPreviewSource<'a>,
    /// Theme package the preview runtimes are constructed with.
    pub theme: HydrolysisPreviewTheme,
    /// Desktop platform the preview binary builds and stages for — the same
    /// target `water run` compiles the managed backend for on this host.
    pub platform: TargetPlatform,
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// `sccache` binary used for compilation caching, when available.
    pub sccache_path: Option<PathBuf>,
    /// Sink compile progress is reported to while the preview build runs cargo.
    pub progress: Option<BuildProgress>,
}

/// Render a preview via the managed Hydrolysis backend binary.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
pub async fn render_preview_with_hydrolysis(
    request: HydrolysisPreviewRequest<'_>,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    let (width, height, theme) = (request.width, request.height, request.theme);
    let (project, built) = build_preview_session(&request, None).await?;
    stage_hydrolysis_resources(&project, theme, &built.app_symbols()?).await?;
    stage_hydrolysis_shared_runtime(&project, &built, request.platform).await?;
    run_preview_binary(
        &project,
        &built.artifact,
        width,
        height,
        output_path,
        scenario,
    )
    .await
}

/// Run a semantic preview test session via the managed Hydrolysis backend binary.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
pub async fn test_preview_with_hydrolysis(
    request: HydrolysisPreviewRequest<'_>,
    automation_body: &str,
) -> Result<String> {
    let (width, height, theme) = (request.width, request.height, request.theme);
    let (project, built) = build_preview_session(&request, Some(automation_body)).await?;
    stage_hydrolysis_resources(&project, theme, &built.app_symbols()?).await?;
    stage_hydrolysis_shared_runtime(&project, &built, request.platform).await?;
    run_preview_test_binary(&project, &built.artifact, width, height).await
}

/// Build the managed hydrolysis binary for a preview request: write the
/// generated bindings, compile in preview mode — or preview-test mode when
/// `automation_body` is `Some` — and return the build result. The app
/// library artifact on it carries the `waterui_meta_*`/`waterui_preview_*`
/// symbols any post-build step (resource staging, `--all` discovery) reads.
async fn build_preview_session(
    request: &HydrolysisPreviewRequest<'_>,
    automation_body: Option<&str>,
) -> Result<(Project, BuiltTarget)> {
    let project = ensure_hydrolysis_backend_ready(request.project_path).await?;
    write_preview_bindings(&project, request.source, request.theme, automation_body).await?;

    let mut build_options = BuildOptions::development(BuildProfile::Debug);
    if let Some(sccache_path) = request.sccache_path.clone() {
        build_options = build_options.with_sccache(sccache_path);
    }
    if let Some(progress) = request.progress.clone() {
        build_options = build_options.with_progress(progress);
    }
    let feature = if automation_body.is_some() {
        HYDROLYSIS_PREVIEW_TEST_FEATURE
    } else {
        HYDROLYSIS_PREVIEW_FEATURE
    };
    let built = build_hydrolysis_with_envs_and_features(
        &project,
        request.platform,
        build_options,
        &[],
        &[feature],
    )
    .await?;
    Ok((project, built))
}

/// Enumerate the `waterui_preview_*` exports the project crate carries.
///
/// The names are read from the library artifact of a preview-mode build of
/// the project's managed backend — `preview test --all` discovers its
/// targets this way, and the probe compiles the same crate graph the
/// per-target builds then reuse warm.
///
/// # Errors
/// Returns an error when the preview build fails or its artifact cannot be
/// parsed.
pub async fn discover_hydrolysis_preview_exports(
    project_path: &Path,
    theme: HydrolysisPreviewTheme,
    platform: TargetPlatform,
    sccache_path: Option<PathBuf>,
    progress: Option<BuildProgress>,
) -> Result<Vec<String>> {
    let request = HydrolysisPreviewRequest {
        project_path,
        source: HydrolysisPreviewSource::Expression("text(\"\")"),
        theme,
        platform,
        width: 0.0,
        height: 0.0,
        sccache_path,
        progress,
    };
    let (_project, built) = build_preview_session(&request, None).await?;
    Ok(built.app_symbols()?.leaves_with_prefix("waterui_preview_"))
}

/// Stages the project's assets and the selected theme's fonts into the
/// generated backend's `resources/` directory. Shared by the preview and MCP
/// runtime modes. `symbols` is the app library artifact the build just
/// produced — its `waterui_meta_bundle_*` statics declare the asset mounts.
pub async fn stage_hydrolysis_resources(
    project: &Project,
    theme: HydrolysisPreviewTheme,
    symbols: &ArtifactSymbols,
) -> Result<()> {
    let resources_dir = project
        .backend_path::<HydrolysisBackend>()
        .join("resources");
    let manifest =
        assets::stage_project_assets_for_gtk(project, &resources_dir, symbols, false).await?;

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let mut font_declarations =
        assets::scan_fonts(project, &backend_path.join("Cargo.toml")).await?;
    font_declarations.extend(theme.font_declarations());
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);
    if resolved_fonts.is_empty() {
        return Ok(());
    }

    let fonts_dest = resources_dir.join("fonts");
    assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
    Ok(())
}

/// Opens the project and makes sure its managed Hydrolysis backend exists and
/// matches the current templates. Shared by the preview and MCP flows.
pub async fn ensure_hydrolysis_backend_ready(project_path: &Path) -> Result<Project> {
    let project = Project::open(
        project_path,
        ManagedBackends::for_backend(TargetBackend::Hydrolysis),
    )
    .await?;
    if HydrolysisBackend::requires_regeneration(&project).await? {
        reinit_backend::<HydrolysisBackend>(&project).await?;
    }

    Ok(project)
}

async fn write_preview_bindings(
    project: &Project,
    source: HydrolysisPreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
    automation_body: Option<&str>,
) -> Result<()> {
    let file_name = if automation_body.is_some() {
        "preview_test.rs"
    } else {
        "preview_symbol.rs"
    };
    let module_path = project
        .backend_path::<HydrolysisBackend>()
        .join("src")
        .join(file_name);
    let crate_name_ident = project.crate_name().rust_ident();
    let (expression_mode, preview_symbol, preview_expression) = match source {
        HydrolysisPreviewSource::Symbol(symbol) => (false, symbol, ""),
        HydrolysisPreviewSource::Expression(expression) => (true, "", expression),
    };
    let rendered = HydrolysisPreviewBindingsTemplate {
        expression_mode,
        preview_symbol,
        preview_expression,
        crate_name_ident: crate_name_ident.as_str(),
        preview_theme_style: theme.style(),
        include_automation: automation_body.is_some(),
        semantic_automation_body: automation_body.unwrap_or(""),
    }
    .render()
    .wrap_err("Failed to render hydrolysis preview bindings template")?;
    smol::fs::write(&module_path, rendered)
        .await
        .wrap_err_with(|| format!("Failed to write {}", module_path.display()))?;
    Ok(())
}

/// Writes the run config JSON next to the backend sources and returns its
/// path; the file is overwritten per invocation.
async fn write_run_config(project: &Project, config: &PreviewRunConfig) -> Result<PathBuf> {
    let path = project
        .backend_path::<HydrolysisBackend>()
        .join("preview-run.json");
    let json = serde_json::to_vec_pretty(config)
        .wrap_err("Failed to serialize hydrolysis preview run config")?;
    smol::fs::write(&path, json)
        .await
        .wrap_err_with(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

async fn run_preview_binary(
    project: &Project,
    binary_path: &Path,
    width: f32,
    height: f32,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    let mode = match scenario {
        Some(scenario) => PreviewRunMode::Scenario {
            output_dir: absolute_output_path(&scenario.output_dir)?,
            captures_ms: scenario.captures_ms.clone(),
            events: scenario.events.clone(),
        },
        None => PreviewRunMode::Image {
            output: absolute_output_path(output_path)?,
        },
    };
    let config = PreviewRunConfig {
        width,
        height,
        mode,
    };
    let config_path = write_run_config(project, &config).await?;
    let backend_path = project.backend_path::<HydrolysisBackend>();

    let mut child = smol::process::Command::new(binary_path);
    let child = command(&mut child);
    child.current_dir(&backend_path);
    child.env(PREVIEW_RUN_CONFIG_ENV, &config_path);

    let output = child.output().await.wrap_err_with(|| {
        format!(
            "Failed to run hydrolysis preview binary {}",
            binary_path.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        bail!("Hydrolysis preview binary failed: {details}");
    }

    match config.mode {
        PreviewRunMode::Scenario {
            ref output_dir,
            ref captures_ms,
            ..
        } => {
            for capture_ms in captures_ms {
                let frame_path = scenario_frame_path(output_dir, *capture_ms);
                expect_nonempty_output(&frame_path, "scenario frame").await?;
            }
        }
        PreviewRunMode::Image { ref output } => {
            expect_nonempty_output(output, "output").await?;
        }
        PreviewRunMode::Semantic => {
            unreachable!("render runs only produce images or scenarios")
        }
    }

    Ok(())
}

async fn run_preview_test_binary(
    project: &Project,
    binary_path: &Path,
    width: f32,
    height: f32,
) -> Result<String> {
    let config = PreviewRunConfig {
        width,
        height,
        mode: PreviewRunMode::Semantic,
    };
    let config_path = write_run_config(project, &config).await?;
    let backend_path = project.backend_path::<HydrolysisBackend>();

    let mut child = smol::process::Command::new(binary_path);
    let child = command(&mut child);
    child.current_dir(&backend_path);
    child.env(PREVIEW_RUN_CONFIG_ENV, &config_path);

    let output = child.output().await.wrap_err_with(|| {
        format!(
            "Failed to run hydrolysis preview test binary {}",
            binary_path.display()
        )
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        bail!("Hydrolysis preview test binary failed: {details}");
    }

    Ok(stdout)
}

async fn expect_nonempty_output(path: &Path, what: &str) -> Result<()> {
    let metadata = smol::fs::metadata(path).await.wrap_err_with(|| {
        format!(
            "Hydrolysis preview did not produce {what} {}",
            path.display()
        )
    })?;
    if metadata.len() == 0 {
        bail!(
            "Hydrolysis preview wrote empty {what} to {}",
            path.display()
        );
    }
    Ok(())
}

fn scenario_frame_path(output_dir: &Path, capture_ms: u64) -> PathBuf {
    output_dir.join(format!("frame-{capture_ms:04}ms.png"))
}

fn absolute_output_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}
