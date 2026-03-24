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
use crate::utils::command;

const HYDROLYSIS_PREVIEW_OUTPUT_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_OUTPUT";
const HYDROLYSIS_PREVIEW_WIDTH_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_WIDTH";
const HYDROLYSIS_PREVIEW_HEIGHT_ENV: &str = "WATERUI_HYDROLYSIS_PREVIEW_HEIGHT";
const HYDROLYSIS_PREVIEW_FEATURE: &str = "waterui-preview-mode";

#[derive(Template)]
#[template(path = "src/preview/hydrolysis_preview_symbol.rs.tpl", escape = "none")]
struct HydrolysisPreviewSymbolTemplate<'a> {
    preview_symbol: &'a str,
    crate_name_ident: &'a str,
    preview_output_env: &'a str,
    preview_width_env: &'a str,
    preview_height_env: &'a str,
}

/// Render a preview via the managed Hydrolysis backend binary.
pub async fn render_preview_with_hydrolysis(
    project_path: &Path,
    symbol: &str,
    width: f32,
    height: f32,
    sccache_path: Option<PathBuf>,
    output_path: &Path,
) -> Result<()> {
    let project = ensure_hydrolysis_backend_ready(project_path).await?;
    write_preview_symbol_bindings(&project, symbol).await?;

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

    let binary_path = built_hydrolysis_binary_path(&project, "debug")?;
    run_preview_binary(&project, &binary_path, width, height, output_path).await
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

async fn write_preview_symbol_bindings(project: &Project, symbol: &str) -> Result<()> {
    let module_path = project
        .backend_path::<HydrolysisBackend>()
        .join("src")
        .join("preview_symbol.rs");
    let crate_name_ident = project.crate_name().rust_ident();
    let rendered = HydrolysisPreviewSymbolTemplate {
        preview_symbol: symbol,
        crate_name_ident: crate_name_ident.as_str(),
        preview_output_env: HYDROLYSIS_PREVIEW_OUTPUT_ENV,
        preview_width_env: HYDROLYSIS_PREVIEW_WIDTH_ENV,
        preview_height_env: HYDROLYSIS_PREVIEW_HEIGHT_ENV,
    }
    .render()
    .wrap_err("Failed to render hydrolysis preview symbol template")?;
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
) -> Result<()> {
    let absolute_output_path = absolute_output_path(output_path)?;
    let backend_path = project.backend_path::<HydrolysisBackend>();

    let mut child = smol::process::Command::new(binary_path);
    let child = command(&mut child);
    child.current_dir(&backend_path);
    child.env(HYDROLYSIS_PREVIEW_OUTPUT_ENV, &absolute_output_path);
    child.env(HYDROLYSIS_PREVIEW_WIDTH_ENV, width.to_string());
    child.env(HYDROLYSIS_PREVIEW_HEIGHT_ENV, height.to_string());

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

    let metadata = smol::fs::metadata(&absolute_output_path)
        .await
        .wrap_err_with(|| {
            format!(
                "Hydrolysis preview did not produce {}",
                absolute_output_path.display()
            )
        })?;
    if metadata.len() == 0 {
        bail!(
            "Hydrolysis preview wrote empty output to {}",
            absolute_output_path.display()
        );
    }

    Ok(())
}

fn absolute_output_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}
