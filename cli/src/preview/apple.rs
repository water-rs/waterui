//! In-process Apple preview for `water preview --platform macos`.
//!
//! Builds the managed Apple preview package — the generated binary that
//! calls `waterui_apple::preview::run` — with one cargo invocation in the
//! shared runtime's target directory, then execs it with the run
//! configuration. The render happens inside the generated process: no
//! TCP, no support app.

use std::path::{Path, PathBuf};

use askama::Template;
use eyre::{Context as _, Result};

use crate::apple::dynamic_runtime;
use crate::apple::platform::apple_build_features;
use crate::artifact_symbols::ArtifactSymbols;
use crate::build::{BuildProgress, RustBuild, RustDynamicLibraries, RustLinkage};
use crate::platform::{TargetBackend, TargetPlatform};
use crate::preview::launcher::ensure_project_dev_feature_for_preview;
use crate::preview::run::{self, absolute_output_path, expect_nonempty_output, write_run_config};
use crate::preview::{PreviewSource, PreviewTargetTemplate};
use crate::project::Project;
use crate::project_model::assets;

use waterui_preview_protocol::run::{PreviewRunConfig, PreviewRunMode};

/// Request for Apple in-process preview rendering.
#[derive(Debug, Clone)]
pub struct ApplePreviewRequest<'a> {
    /// Path to the project to preview.
    pub project_path: &'a Path,
    /// Source used to produce the preview view.
    pub source: PreviewSource<'a>,
    /// Preview width in logical pixels.
    pub width: f32,
    /// Preview height in logical pixels.
    pub height: f32,
    /// Optional sccache binary path.
    pub sccache_path: Option<PathBuf>,
    /// Optional build progress sink.
    pub progress: Option<BuildProgress>,
}

/// Generate `preview_target.rs`, the module wiring the preview binary to
/// the requested target.
///
/// Written through `write_file_if_changed`: a run that repeats a target
/// leaves the binary's own unit alone in the shared target directory.
async fn write_apple_preview_target(project: &Project, source: &PreviewSource<'_>) -> Result<()> {
    let crate_name_ident = project.crate_name().rust_ident();
    let rendered = PreviewTargetTemplate::apple(*source, crate_name_ident.as_str())
        .render()
        .map_err(eyre::Report::new)?;
    crate::project_model::templates::write_file_if_changed(
        &project
            .apple_preview_crate_path()
            .join("src/preview_target.rs"),
        rendered.as_bytes(),
    )
    .await
    .wrap_err("Failed to write the preview target module")
}

/// Stage the project's assets and fonts where the preview binary's
/// `ResourceContext` looks for them, the way the Hydrolysis preview
/// locates them.
async fn stage_apple_preview_resources(project: &Project, symbols: &ArtifactSymbols) -> Result<()> {
    let resources_dir = project.apple_preview_crate_path().join("resources");
    let manifest =
        assets::stage_project_assets_for_apple(project, &resources_dir, symbols, false).await?;

    let mut resolved_fonts = {
        let font_manifest = project.apple_crate_path().join("Cargo.toml");
        let font_declarations = assets::scan_fonts(project, &font_manifest).await?;
        assets::resolve_fonts(font_declarations).await?
    };
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);
    if resolved_fonts.is_empty() {
        return Ok(());
    }
    assets::copy_fonts(&resolved_fonts, &resources_dir.join("fonts")).await
}

/// Execute the preview binary in place with the run configuration it
/// reads from `WATERUI_PREVIEW_RUN_CONFIG`.
async fn run_preview_binary(
    project: &Project,
    binary: &Path,
    width: f32,
    height: f32,
    output_path: &Path,
) -> Result<()> {
    let crate_dir = project.apple_preview_crate_path();
    let run_config = PreviewRunConfig {
        width,
        height,
        mode: PreviewRunMode::Image {
            output: output_path.to_path_buf(),
        },
    };
    let run_config_path = write_run_config(&crate_dir, &run_config).await?;
    run::run_preview_binary(&crate_dir, binary, &run_config_path, "Apple preview").await?;
    Ok(())
}

/// Render a preview with the Apple backend in-process.
///
/// # Errors
///
/// Returns an error when the project cannot be opened, the preview package
/// cannot be scaffolded or built, the binary fails, or the image was not
/// produced.
pub async fn render_preview_with_apple(
    request: ApplePreviewRequest<'_>,
    output_path: &Path,
) -> Result<()> {
    let output_path = absolute_output_path(output_path)?;
    let project = Project::open_for_preview_build(request.project_path).await?;
    // The Apple companion is scaffolded too: its manifest owns the shared
    // runtime's feature forwards and the font declarations the resources
    // scan reads, and its build already lives in the shared target
    // directory.
    project.scaffold_apple_companion(true).await?;
    ensure_project_dev_feature_for_preview(&project).await?;
    project.scaffold_apple_preview_companion().await?;
    write_apple_preview_target(&project, &request.source).await?;

    let browser_runtime = project
        .browser_runtime_plan(TargetPlatform::MacOS, TargetBackend::Apple)
        .await?;
    let features =
        apple_build_features(&project, browser_runtime, RustLinkage::SharedRuntime).await?;
    let triple = TargetPlatform::MacOS.triple();
    let target_dir = project.water_target_dir(RustLinkage::SharedRuntime).await?;
    let mut rust_build = RustBuild::new(project.apple_preview_crate_path(), triple.clone())
        .with_project(&project)
        .with_features(features)
        .with_preferred_dynamic_linking()
        .with_target_dir(target_dir);
    if let Some(sccache) = request.sccache_path {
        rust_build = rust_build.with_sccache(sccache);
    }
    if let Some(progress) = request.progress {
        rust_build = rust_build.with_progress(progress);
    }
    let built = rust_build
        .build_binary(project.apple_preview_crate_name().as_str(), false)
        .await
        .wrap_err("Failed to build the Apple preview binary")?;

    // The binary links `libwaterui_dylib` and `libstd` dynamically: stage
    // both beside it — the profile directory is on its rpath — then give
    // the staged runtime copy the canonical install name and retarget the
    // binary's hashed reference to it, the same fix-up the entry-owning
    // binary gets, so only one runtime image loads in the process.
    let libraries = RustDynamicLibraries::resolve(&built, &triple, &project).await?;
    let profile_dir = built.profile_dir.clone();
    libraries.stage(&profile_dir).await?;
    let staged_runtime = libraries.stage_apple_canonical(&profile_dir).await?;
    let executable = built.executable()?;
    dynamic_runtime::retarget_module(executable, &staged_runtime).await?;
    dynamic_runtime::prepare_host_runtime(&staged_runtime).await?;

    let symbols = built.app_symbols()?;
    stage_apple_preview_resources(&project, &symbols).await?;
    run_preview_binary(
        &project,
        executable,
        request.width,
        request.height,
        &output_path,
    )
    .await?;
    expect_nonempty_output(&output_path, "the Apple preview binary's PNG").await
}
