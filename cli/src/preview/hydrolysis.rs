use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{Context as _, Result, bail};

use crate::backend::reinit_backend;
use crate::build::BuildOptions;
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::hydrolysis::platform::{
    build_hydrolysis_with_envs_and_args, built_hydrolysis_binary_path,
};
use crate::platform::TargetPlatform;
use crate::project::Project;
use crate::project_model::assets;
use crate::utils::command;

const HYDROLYSIS_PREVIEW_OUTPUT_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_OUTPUT";
const HYDROLYSIS_PREVIEW_OUTPUT_DIR_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_OUTPUT_DIR";
const HYDROLYSIS_PREVIEW_WIDTH_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_WIDTH";
const HYDROLYSIS_PREVIEW_HEIGHT_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_HEIGHT";
const HYDROLYSIS_PREVIEW_CAPTURE_MS_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_CAPTURE_MS";
const HYDROLYSIS_PREVIEW_EVENTS_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_EVENTS";
const HYDROLYSIS_PREVIEW_PERF_WARMUPS_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_PERF_WARMUPS";
const HYDROLYSIS_PREVIEW_PERF_SAMPLES_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_PERF_SAMPLES";
const HYDROLYSIS_PREVIEW_PERF_REPETITIONS_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_PERF_REPETITIONS";
const HYDROLYSIS_PREVIEW_FLAMEGRAPH_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_FLAMEGRAPH";
const HYDROLYSIS_PREVIEW_FLAMEGRAPH_FREQUENCY_ENV: &str =
    "WATERUI_HYDROLYSIS_PREVIEW_FLAMEGRAPH_FREQUENCY";
const HYDROLYSIS_PREVIEW_FEATURE: &str = "waterui-preview-mode";
const HYDROLYSIS_PREVIEW_TEST_FEATURE: &str = "waterui-preview-test-mode";

/// Theme package selected for Hydrolysis preview rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisPreviewTheme {
    /// Material Design 3 package.
    Material3,
}

impl HydrolysisPreviewTheme {
    const fn name(self) -> &'static str {
        match self {
            Self::Material3 => "material3",
        }
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisPreviewTestMode {
    /// Semantic accessibility-tree test without a render target.
    Semantic,
    /// Full offscreen GPU performance test.
    Perf,
}

impl HydrolysisPreviewTestMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Perf => "perf",
        }
    }
}

/// Repeated frame sampling configuration for Hydrolysis preview perf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HydrolysisPreviewPerfConfig {
    /// Warmup frame count before recording samples.
    pub warmups: u32,
    /// Recorded frame count.
    pub samples: u32,
    /// Independent repetitions.
    pub repetitions: u32,
}

/// Optional CPU call-stack flamegraph configuration for preview perf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrolysisPreviewFlamegraph {
    /// Output SVG path.
    pub output_path: PathBuf,
    /// Sampling frequency in Hertz.
    pub frequency: i32,
}

/// Pointer button used by a Hydrolysis preview scenario event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisPreviewPointerButton {
    /// Primary pointer button.
    Primary,
    /// Secondary pointer button.
    Secondary,
    /// Middle pointer button.
    Middle,
}

impl HydrolysisPreviewPointerButton {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Middle => "middle",
        }
    }
}

/// Event kind in a Hydrolysis preview scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrolysisPreviewEventKind {
    /// Move the pointer without pressing.
    PointerMove,
    /// Press a pointer button.
    PointerDown,
    /// Release a pointer button.
    PointerUp,
    /// Cancel the active pointer.
    PointerCancel,
    /// Dispatch a wheel or trackpad scroll event.
    Scroll,
}

impl HydrolysisPreviewEventKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PointerMove => "pointer_move",
            Self::PointerDown => "pointer_down",
            Self::PointerUp => "pointer_up",
            Self::PointerCancel => "pointer_cancel",
            Self::Scroll => "scroll",
        }
    }
}

/// One input event in a Hydrolysis preview scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct HydrolysisPreviewScenarioEvent {
    /// Event timestamp in milliseconds from scenario start.
    pub at_ms: u64,
    /// Event kind.
    pub kind: HydrolysisPreviewEventKind,
    /// Pointer x coordinate in WaterUI logical units.
    pub x: f32,
    /// Pointer y coordinate in WaterUI logical units.
    pub y: f32,
    /// Pointer button for down/up events.
    pub button: Option<HydrolysisPreviewPointerButton>,
    /// Scroll delta along the x axis for scroll events.
    pub dx: f32,
    /// Scroll delta along the y axis for scroll events.
    pub dy: f32,
    /// Whether scroll delta values are line units instead of logical units.
    pub is_line_delta: bool,
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
#[template(path = "src/preview/hydrolysis_preview_symbol.rs.tpl", escape = "none")]
struct HydrolysisPreviewSymbolTemplate<'a> {
    expression_mode: bool,
    preview_symbol: &'a str,
    preview_expression: &'a str,
    crate_name_ident: &'a str,
    preview_theme: &'a str,
    preview_theme_installer: &'a str,
    preview_output_env: &'a str,
    preview_output_dir_env: &'a str,
    preview_width_env: &'a str,
    preview_height_env: &'a str,
    preview_capture_ms_env: &'a str,
    preview_events_env: &'a str,
}

#[derive(Template)]
#[template(path = "src/preview/hydrolysis_preview_test.rs.tpl", escape = "none")]
struct HydrolysisPreviewTestTemplate<'a> {
    expression_mode: bool,
    preview_symbol: &'a str,
    preview_expression: &'a str,
    crate_name_ident: &'a str,
    preview_theme: &'a str,
    preview_theme_installer: &'a str,
    test_mode: &'a str,
    semantic_automation_body: &'a str,
    perf_automation_body: &'a str,
    preview_width_env: &'a str,
    preview_height_env: &'a str,
    perf_warmups_env: &'a str,
    perf_samples_env: &'a str,
    perf_repetitions_env: &'a str,
    flamegraph_env: &'a str,
    flamegraph_frequency_env: &'a str,
}

/// Render a preview via the managed Hydrolysis backend binary.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
pub async fn render_preview_with_hydrolysis(
    project_path: &Path,
    source: HydrolysisPreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
    width: f32,
    height: f32,
    sccache_path: Option<PathBuf>,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    let project = ensure_hydrolysis_backend_ready(project_path).await?;
    write_preview_symbol_bindings(&project, source, theme).await?;
    stage_preview_resources(&project, theme).await?;

    let mut build_options = BuildOptions::new(false);
    if let Some(sccache_path) = sccache_path {
        build_options = build_options.with_sccache(sccache_path);
    }
    build_hydrolysis_with_envs_and_args(
        &project,
        TargetPlatform::MacOS,
        build_options,
        &[],
        &["--features", HYDROLYSIS_PREVIEW_FEATURE],
    )
    .await?;

    let binary_path = built_hydrolysis_binary_path(&project, "debug").await?;
    run_preview_binary(&project, &binary_path, width, height, output_path, scenario).await
}

/// Run a preview test or perf session via the managed Hydrolysis backend binary.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
#[allow(clippy::too_many_arguments)]
pub async fn test_preview_with_hydrolysis(
    project_path: &Path,
    source: HydrolysisPreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
    mode: HydrolysisPreviewTestMode,
    width: f32,
    height: f32,
    sccache_path: Option<PathBuf>,
    automation_body: &str,
    perf_config: Option<HydrolysisPreviewPerfConfig>,
    flamegraph: Option<&HydrolysisPreviewFlamegraph>,
) -> Result<String> {
    let project = ensure_hydrolysis_backend_ready(project_path).await?;
    write_preview_test_bindings(&project, source, theme, mode, automation_body).await?;
    stage_preview_resources(&project, theme).await?;

    let mut build_options = BuildOptions::new(false);
    if let Some(sccache_path) = sccache_path {
        build_options = build_options.with_sccache(sccache_path);
    }
    build_hydrolysis_with_envs_and_args(
        &project,
        TargetPlatform::MacOS,
        build_options,
        &[],
        &["--features", HYDROLYSIS_PREVIEW_TEST_FEATURE],
    )
    .await?;

    let binary_path = built_hydrolysis_binary_path(&project, "debug").await?;
    run_preview_test_binary(
        &project,
        &binary_path,
        width,
        height,
        perf_config,
        flamegraph,
    )
    .await
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
    if project.hydrolysis_backend().is_none() {
        if !project.is_playground() {
            bail!("Hydrolysis backend is not configured. Run `water backend add hydrolysis`.");
        }

        reinit_backend::<HydrolysisBackend>(&project).await?;
        project = Project::open(project_path).await?;
        return Ok(project);
    }

    if HydrolysisBackend::requires_regeneration(&project)? {
        reinit_backend::<HydrolysisBackend>(&project).await?;
        project = Project::open(project_path).await?;
    }

    Ok(project)
}

async fn write_preview_symbol_bindings(
    project: &Project,
    source: HydrolysisPreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
) -> Result<()> {
    let module_path = project
        .backend_path::<HydrolysisBackend>()
        .join("src")
        .join("preview_symbol.rs");
    let crate_name_ident = project.crate_name().rust_ident();
    let (expression_mode, preview_symbol, preview_expression) = match source {
        HydrolysisPreviewSource::Symbol(symbol) => (false, symbol, ""),
        HydrolysisPreviewSource::Expression(expression) => (true, "", expression),
    };
    let rendered = HydrolysisPreviewSymbolTemplate {
        expression_mode,
        preview_symbol,
        preview_expression,
        crate_name_ident: crate_name_ident.as_str(),
        preview_theme: theme.name(),
        preview_theme_installer: theme.installer(),
        preview_output_env: HYDROLYSIS_PREVIEW_OUTPUT_ENV,
        preview_output_dir_env: HYDROLYSIS_PREVIEW_OUTPUT_DIR_ENV,
        preview_width_env: HYDROLYSIS_PREVIEW_WIDTH_ENV,
        preview_height_env: HYDROLYSIS_PREVIEW_HEIGHT_ENV,
        preview_capture_ms_env: HYDROLYSIS_PREVIEW_CAPTURE_MS_ENV,
        preview_events_env: HYDROLYSIS_PREVIEW_EVENTS_ENV,
    }
    .render()
    .wrap_err("Failed to render hydrolysis preview symbol template")?;
    smol::fs::write(&module_path, rendered)
        .await
        .wrap_err_with(|| format!("Failed to write {}", module_path.display()))?;
    Ok(())
}

async fn write_preview_test_bindings(
    project: &Project,
    source: HydrolysisPreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
    mode: HydrolysisPreviewTestMode,
    automation_body: &str,
) -> Result<()> {
    let module_path = project
        .backend_path::<HydrolysisBackend>()
        .join("src")
        .join("preview_test.rs");
    let crate_name_ident = project.crate_name().rust_ident();
    let (expression_mode, preview_symbol, preview_expression) = match source {
        HydrolysisPreviewSource::Symbol(symbol) => (false, symbol, ""),
        HydrolysisPreviewSource::Expression(expression) => (true, "", expression),
    };
    let rendered = HydrolysisPreviewTestTemplate {
        expression_mode,
        preview_symbol,
        preview_expression,
        crate_name_ident: crate_name_ident.as_str(),
        preview_theme: theme.name(),
        preview_theme_installer: theme.installer(),
        test_mode: mode.as_str(),
        semantic_automation_body: if mode == HydrolysisPreviewTestMode::Semantic {
            automation_body
        } else {
            ""
        },
        perf_automation_body: if mode == HydrolysisPreviewTestMode::Perf {
            automation_body
        } else {
            ""
        },
        preview_width_env: HYDROLYSIS_PREVIEW_WIDTH_ENV,
        preview_height_env: HYDROLYSIS_PREVIEW_HEIGHT_ENV,
        perf_warmups_env: HYDROLYSIS_PREVIEW_PERF_WARMUPS_ENV,
        perf_samples_env: HYDROLYSIS_PREVIEW_PERF_SAMPLES_ENV,
        perf_repetitions_env: HYDROLYSIS_PREVIEW_PERF_REPETITIONS_ENV,
        flamegraph_env: HYDROLYSIS_PREVIEW_FLAMEGRAPH_ENV,
        flamegraph_frequency_env: HYDROLYSIS_PREVIEW_FLAMEGRAPH_FREQUENCY_ENV,
    }
    .render()
    .wrap_err("Failed to render hydrolysis preview test template")?;
    smol::fs::write(&module_path, rendered)
        .await
        .wrap_err_with(|| format!("Failed to write {}", module_path.display()))?;
    Ok(())
}

async fn run_preview_binary(
    project: &Project,
    binary_path: &Path,
    width: f32,
    height: f32,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    let absolute_preview_output_path = absolute_output_path(output_path)?;
    let backend_path = project.backend_path::<HydrolysisBackend>();

    let mut child = smol::process::Command::new(binary_path);
    let child = command(&mut child);
    child.current_dir(&backend_path);
    child.env(HYDROLYSIS_PREVIEW_OUTPUT_ENV, &absolute_preview_output_path);
    child.env(HYDROLYSIS_PREVIEW_WIDTH_ENV, width.to_string());
    child.env(HYDROLYSIS_PREVIEW_HEIGHT_ENV, height.to_string());
    if let Some(scenario) = scenario {
        let output_dir = absolute_output_path(&scenario.output_dir)?;
        child.env(HYDROLYSIS_PREVIEW_OUTPUT_DIR_ENV, output_dir);
        child.env(
            HYDROLYSIS_PREVIEW_CAPTURE_MS_ENV,
            encode_captures(&scenario.captures_ms),
        );
        child.env(
            HYDROLYSIS_PREVIEW_EVENTS_ENV,
            encode_events(&scenario.events),
        );
    }

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

    if let Some(scenario) = scenario {
        let output_dir = absolute_output_path(&scenario.output_dir)?;
        for capture_ms in &scenario.captures_ms {
            let frame_path = scenario_frame_path(&output_dir, *capture_ms);
            let metadata = smol::fs::metadata(&frame_path).await.wrap_err_with(|| {
                format!(
                    "Hydrolysis preview did not produce scenario frame {}",
                    frame_path.display()
                )
            })?;
            if metadata.len() == 0 {
                bail!(
                    "Hydrolysis preview wrote empty scenario frame to {}",
                    frame_path.display()
                );
            }
        }
    } else {
        let metadata = smol::fs::metadata(&absolute_preview_output_path)
            .await
            .wrap_err_with(|| {
                format!(
                    "Hydrolysis preview did not produce {}",
                    absolute_preview_output_path.display()
                )
            })?;
        if metadata.len() == 0 {
            bail!(
                "Hydrolysis preview wrote empty output to {}",
                absolute_preview_output_path.display()
            );
        }
    }

    Ok(())
}

async fn run_preview_test_binary(
    project: &Project,
    binary_path: &Path,
    width: f32,
    height: f32,
    perf_config: Option<HydrolysisPreviewPerfConfig>,
    flamegraph: Option<&HydrolysisPreviewFlamegraph>,
) -> Result<String> {
    let backend_path = project.backend_path::<HydrolysisBackend>();

    let mut child = smol::process::Command::new(binary_path);
    let child = command(&mut child);
    child.current_dir(&backend_path);
    child.env(HYDROLYSIS_PREVIEW_WIDTH_ENV, width.to_string());
    child.env(HYDROLYSIS_PREVIEW_HEIGHT_ENV, height.to_string());
    if let Some(config) = perf_config {
        child.env(
            HYDROLYSIS_PREVIEW_PERF_WARMUPS_ENV,
            config.warmups.to_string(),
        );
        child.env(
            HYDROLYSIS_PREVIEW_PERF_SAMPLES_ENV,
            config.samples.to_string(),
        );
        child.env(
            HYDROLYSIS_PREVIEW_PERF_REPETITIONS_ENV,
            config.repetitions.to_string(),
        );
    }
    if let Some(flamegraph) = flamegraph {
        child.env(
            HYDROLYSIS_PREVIEW_FLAMEGRAPH_ENV,
            absolute_output_path(&flamegraph.output_path)?,
        );
        child.env(
            HYDROLYSIS_PREVIEW_FLAMEGRAPH_FREQUENCY_ENV,
            flamegraph.frequency.to_string(),
        );
    }

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

    if let Some(flamegraph) = flamegraph {
        let flamegraph_path = absolute_output_path(&flamegraph.output_path)?;
        let metadata = smol::fs::metadata(&flamegraph_path)
            .await
            .wrap_err_with(|| {
                format!(
                    "Hydrolysis preview perf did not produce flamegraph {}",
                    flamegraph_path.display()
                )
            })?;
        if metadata.len() == 0 {
            bail!(
                "Hydrolysis preview perf wrote empty flamegraph to {}",
                flamegraph_path.display()
            );
        }
    }

    Ok(stdout)
}

fn scenario_frame_path(output_dir: &Path, capture_ms: u64) -> PathBuf {
    output_dir.join(format!("frame-{capture_ms:04}ms.png"))
}

fn encode_captures(captures_ms: &[u64]) -> String {
    captures_ms
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn encode_events(events: &[HydrolysisPreviewScenarioEvent]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}",
                event.at_ms,
                event.kind.as_str(),
                event.x,
                event.y,
                event
                    .button
                    .map_or("", HydrolysisPreviewPointerButton::as_str),
                event.dx,
                event.dy,
                u8::from(event.is_line_delta)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn absolute_output_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}
