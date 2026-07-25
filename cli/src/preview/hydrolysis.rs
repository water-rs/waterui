use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{Context as _, Result, bail};

use crate::backend::reinit_backend;
use crate::build::{BuildOptions, RustLinkage};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::hydrolysis::platform::{
    build_hydrolysis_with_envs_and_features, built_hydrolysis_binary_path,
};
use crate::platform::TargetPlatform;
use crate::project::Project;
use crate::project_model::assets;
use crate::utils::command;

const HYDROLYSIS_PREVIEW_FEATURE: &str = "waterui-preview-mode";
const HYDROLYSIS_PREVIEW_TEST_FEATURE: &str = "waterui-preview-test-mode";

use waterui_preview_protocol::hydrolysis::{
    PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode,
};
pub use waterui_preview_protocol::hydrolysis::{
    PerfRunConfig as HydrolysisPreviewPerfRun, ScenarioEvent as HydrolysisPreviewScenarioEvent,
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
    const fn installer(self) -> &'static str {
        match self {
            Self::Material3 => "hydrolysis_m3::install",
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

/// Hydrolysis preview test mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HydrolysisPreviewTestMode {
    /// Semantic accessibility-tree test without a render target.
    Semantic,
    /// Full offscreen GPU performance test.
    Perf(HydrolysisPreviewPerfRun),
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
    preview_theme_installer: &'a str,
    include_automation: bool,
    semantic_automation_body: &'a str,
    perf_automation_body: &'a str,
}

/// Common inputs for driving the managed Hydrolysis preview backend.
#[derive(Debug, Clone)]
pub struct HydrolysisPreviewRequest<'a> {
    /// `WaterUI` project directory.
    pub project_path: &'a Path,
    /// Preview view source.
    pub source: HydrolysisPreviewSource<'a>,
    /// Theme package installed into the preview environment.
    pub theme: HydrolysisPreviewTheme,
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// `sccache` binary used for compilation caching, when available.
    pub sccache_path: Option<PathBuf>,
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
    let HydrolysisPreviewRequest {
        project_path,
        source,
        theme,
        width,
        height,
        sccache_path,
    } = request;
    let project = ensure_hydrolysis_backend_ready(project_path).await?;
    write_preview_bindings(&project, source, theme, None).await?;
    stage_preview_resources(&project, theme).await?;

    let mut build_options = BuildOptions::development(false);
    if let Some(sccache_path) = sccache_path {
        build_options = build_options.with_sccache(sccache_path);
    }
    build_hydrolysis_with_envs_and_features(
        &project,
        TargetPlatform::MacOS,
        build_options,
        &[],
        &[HYDROLYSIS_PREVIEW_FEATURE],
    )
    .await?;

    let binary_path = built_hydrolysis_binary_path(
        &project,
        TargetPlatform::MacOS,
        "debug",
        RustLinkage::SharedRuntime,
        &[HYDROLYSIS_PREVIEW_FEATURE],
    )
    .await?;
    run_preview_binary(&project, &binary_path, width, height, output_path, scenario).await
}

/// Run a preview test or perf session via the managed Hydrolysis backend binary.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
pub async fn test_preview_with_hydrolysis(
    request: HydrolysisPreviewRequest<'_>,
    mode: HydrolysisPreviewTestMode,
    automation_body: &str,
) -> Result<String> {
    let HydrolysisPreviewRequest {
        project_path,
        source,
        theme,
        width,
        height,
        sccache_path,
    } = request;
    let project = ensure_hydrolysis_backend_ready(project_path).await?;
    write_preview_bindings(
        &project,
        source,
        theme,
        Some(PreviewAutomation {
            mode: &mode,
            body: automation_body,
        }),
    )
    .await?;
    stage_preview_resources(&project, theme).await?;

    let mut build_options = BuildOptions::development(false);
    if let Some(sccache_path) = sccache_path {
        build_options = build_options.with_sccache(sccache_path);
    }
    build_hydrolysis_with_envs_and_features(
        &project,
        TargetPlatform::MacOS,
        build_options,
        &[],
        &[HYDROLYSIS_PREVIEW_TEST_FEATURE],
    )
    .await?;

    let binary_path = built_hydrolysis_binary_path(
        &project,
        TargetPlatform::MacOS,
        "debug",
        RustLinkage::SharedRuntime,
        &[HYDROLYSIS_PREVIEW_TEST_FEATURE],
    )
    .await?;
    run_preview_test_binary(&project, &binary_path, width, height, &mode).await
}

async fn stage_preview_resources(project: &Project, theme: HydrolysisPreviewTheme) -> Result<()> {
    let resources_dir = project
        .backend_path::<HydrolysisBackend>()
        .join("resources");
    assets::stage_project_assets_for_gtk(project, &resources_dir).await?;

    let mut font_declarations = assets::scan_fonts(project).await?;
    font_declarations.extend(theme.font_declarations());
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(project)?);
    if resolved_fonts.is_empty() {
        return Ok(());
    }

    let fonts_dest = resources_dir.join("fonts");
    assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
    Ok(())
}

async fn ensure_hydrolysis_backend_ready(project_path: &Path) -> Result<Project> {
    let mut project = Project::open(project_path).await?;
    if project.hydrolysis_backend().is_none() && !project.is_playground() {
        bail!("Hydrolysis backend is not configured. Run `water backend add hydrolysis`.");
    }

    if HydrolysisBackend::requires_regeneration(&project)? {
        reinit_backend::<HydrolysisBackend>(&project).await?;
        project = Project::open(project_path).await?;
    }

    Ok(project)
}

/// Automation bound into the preview test binding.
struct PreviewAutomation<'a> {
    mode: &'a HydrolysisPreviewTestMode,
    body: &'a str,
}

async fn write_preview_bindings(
    project: &Project,
    source: HydrolysisPreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
    automation: Option<PreviewAutomation<'_>>,
) -> Result<()> {
    let file_name = if automation.is_some() {
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
    let (semantic_automation_body, perf_automation_body) =
        automation
            .as_ref()
            .map_or(("", ""), |automation| match automation.mode {
                HydrolysisPreviewTestMode::Semantic => (automation.body, ""),
                HydrolysisPreviewTestMode::Perf(_) => ("", automation.body),
            });
    let rendered = HydrolysisPreviewBindingsTemplate {
        expression_mode,
        preview_symbol,
        preview_expression,
        crate_name_ident: crate_name_ident.as_str(),
        preview_theme_installer: theme.installer(),
        include_automation: automation.is_some(),
        semantic_automation_body,
        perf_automation_body,
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
        PreviewRunMode::Semantic | PreviewRunMode::Perf(_) => {
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
    mode: &HydrolysisPreviewTestMode,
) -> Result<String> {
    let run_mode = match mode {
        HydrolysisPreviewTestMode::Semantic => PreviewRunMode::Semantic,
        HydrolysisPreviewTestMode::Perf(perf) => {
            let mut perf = perf.clone();
            perf.flamegraph = perf
                .flamegraph
                .as_deref()
                .map(absolute_output_path)
                .transpose()?;
            PreviewRunMode::Perf(perf)
        }
    };
    let config = PreviewRunConfig {
        width,
        height,
        mode: run_mode,
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

    if let PreviewRunMode::Perf(perf) = &config.mode
        && let Some(flamegraph) = &perf.flamegraph
    {
        expect_nonempty_output(flamegraph, "flamegraph").await?;
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
