//! Experimental terminal (TUI) backend support.
//!
//! The renderer lives in the standalone `water-rs/tui` repository — the CLI
//! only generates a thin launcher crate under the project's managed build cache
//! and hands the invoking terminal to the built binary. The backend is opt-in
//! through `water run --tui` alone: it is not a managed [`crate::backend::Backend`],
//! does not appear in backend selection, and cannot be configured in
//! `Water.toml` while experimental.
//!
//! Dependency resolution for the generated launcher follows the project's own
//! framework mode: a `waterui_path` checkout supplies path dependencies and the
//! checkout's `[patch]` table, a channel supplies the resolved framework's
//! `[patch]` table, and a stable project resolves everything from the registry.
//! The `waterui-tui` crate itself comes from `WATERUI_TUI_PATH`, a `water-rs/tui`
//! checkout beside a local `waterui_path`, or the pinned
//! [`crate::build_info::TUI_BACKEND`] revision, in that order.

use std::path::{Path, PathBuf};

use color_eyre::eyre;

use crate::{
    build::{RustBuild, RustLinkage},
    project::Project,
    templates::{self, TemplateContext},
    water_dir,
};

/// Directory the launcher's sources are generated into.
///
/// The TUI launcher always lives in the project's managed build cache —
/// including for application projects — because an experimental backend never
/// writes into the project's own tree.
async fn launcher_dir(project: &Project) -> eyre::Result<PathBuf> {
    Ok(water_dir::project_build_cache_dir(project.root())
        .await?
        .join("tui"))
}

fn template_context(project: &Project, dir: &Path) -> TemplateContext {
    let manifest = project.manifest();
    let app_name = manifest
        .package
        .name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();
    TemplateContext::for_project_manifest(manifest, project.crate_name().clone(), app_name)
        .with_backend_project_path(dir.to_path_buf())
        .with_project_root_path(project.root().to_path_buf())
}

/// Whether the generated launcher's sources differ from what the current
/// templates would produce for this project.
async fn requires_regeneration(project: &Project, dir: &Path) -> eyre::Result<bool> {
    let ctx = template_context(project, dir);
    for (relative, expected) in
        templates::tui::rendered_outputs(&ctx, project.tui_backend_crate_name().as_str())?
    {
        match std::fs::read(dir.join(&relative)) {
            Ok(existing) if existing == expected => {}
            Ok(_) | Err(_) => return Ok(true),
        }
    }
    Ok(false)
}

/// Regenerate the launcher crate when it is missing or stale and return its
/// directory.
///
/// # Errors
///
/// Returns an error if the build cache cannot be resolved, template rendering
/// fails, or the launcher's sources cannot be written.
pub async fn ensure_launcher(project: &Project) -> eyre::Result<PathBuf> {
    let dir = launcher_dir(project).await?;
    if requires_regeneration(project, &dir).await? {
        let ctx = template_context(project, &dir);
        templates::tui::scaffold(&dir, &ctx, project.tui_backend_crate_name().as_str()).await?;
    }
    Ok(dir)
}

/// Build the launcher binary for the host and return its path.
///
/// The TUI launcher always builds the static-runtime variant: it is a leaf
/// binary, not a plugin host, so the shared-runtime linkage has nothing to
/// offer it.
///
/// # Errors
///
/// Returns an error if the project's target directory cannot be resolved or
/// the Cargo build fails.
pub async fn build(
    project: &Project,
    launcher_dir: &Path,
    sccache_path: Option<PathBuf>,
) -> eyre::Result<PathBuf> {
    let mut build = RustBuild::new(launcher_dir, target_lexicon::Triple::host())
        .with_project(project)
        .with_target_dir(project.water_target_dir(RustLinkage::Static).await?);
    if let Some(sccache_path) = sccache_path {
        build = build.with_sccache(sccache_path);
    }
    build
        .build_binary(project.tui_backend_crate_name().as_str(), false)
        .await
        .map_err(|error| eyre::eyre!("failed to build the TUI launcher: {error}"))
}

/// Hand the invoking terminal to the built launcher.
///
/// On Unix the launcher replaces the CLI process via `exec`, so the terminal is
/// owned by exactly one process and its exit status propagates unchanged — a
/// spawned child would instead share the process group with `water` and keep
/// the TTY after `water` itself dies on `SIGINT`. Elsewhere the launcher is
/// spawned with inherited stdio and awaited.
///
/// # Errors
///
/// Returns an error if the launcher cannot be started; on Unix a successful
/// `exec` never returns.
pub fn exec(binary: &Path) -> eyre::Result<()> {
    use std::io::Write as _;
    // Anything still buffered in the CLI's stdout would be lost (exec) or
    // interleave with the launcher's own escape sequences (spawn), so flush
    // before handing over the terminal.
    let _ = std::io::stdout().flush();
    #[cfg(unix)]
    {
        use color_eyre::eyre::WrapErr as _;
        use std::os::unix::process::CommandExt as _;
        Err(std::process::Command::new(binary).exec())
            .wrap_err_with(|| format!("failed to launch the TUI binary {}", binary.display()))
    }
    #[cfg(not(unix))]
    {
        // Blocking here is the point: the launcher owns the terminal until it
        // exits, and nothing runs after this call.
        let status = std::process::Command::new(binary).status()?;
        if !status.success() {
            eyre::bail!("the TUI application exited with {status}");
        }
        Ok(())
    }
}
