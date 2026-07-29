//! GTK4 backend configuration and initialization.

use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    gtk4::platform::{build_gtk4, clean_gtk4, is_gtk4_platform, package_gtk4},
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::Project,
    templates::{self, TemplateContext},
};

/// Configuration for the GTK4 backend in a `WaterUI` project.
///
/// `[backend.gtk4]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Gtk4Backend {
    #[serde(
        default = "default_gtk4_project_path",
        skip_serializing_if = "is_default_gtk4_project_path"
    )]
    project_path: PathBuf,
}

impl Gtk4Backend {
    /// Create a new GTK4 backend configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_gtk4_project_path(),
        }
    }

    /// Set a custom project path (defaults to "gtk4").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Get the path to the GTK4 project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    /// Check whether managed GTK backend files differ from the current templates.
    ///
    /// # Errors
    ///
    /// Returns an error when the application dependency graph or templates
    /// cannot be resolved.
    pub async fn requires_regeneration(project: &Project) -> eyre::Result<bool> {
        let backend_dir = project.backend_path::<Self>();
        let ctx = Self::template_context(project).await?;
        for (relative, expected) in
            templates::gtk4::rendered_outputs(&ctx, &project.gtk_backend_crate_name())?
        {
            match std::fs::read(backend_dir.join(relative)) {
                Ok(existing) if existing == expected => {}
                Ok(_) | Err(_) => return Ok(true),
            }
        }
        Ok(false)
    }

    async fn template_context(project: &Project) -> eyre::Result<TemplateContext> {
        let manifest = project.manifest();
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        Ok(
            TemplateContext::for_project_manifest(manifest, project.crate_name().clone(), app_name)
                .with_backend_project_path(project.backend_path::<Self>())
                .with_project_root_path(project.root().to_path_buf())
                .with_webview_enabled(project.links_runtime_package("waterui-webview").await?)
                .with_chromium_enabled(project.links_runtime_package("waterui-chromium").await?),
        )
    }
}

impl Default for Gtk4Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Gtk4Backend {
    const DEFAULT_PATH: &'static str = "gtk4";

    // GTK4 uses cargo's target directory for build caches
    // Since GTK4 project is a simple Rust binary crate, it uses the workspace target
    // No need to preserve local target - it's part of the workspace
    const CACHE_PATHS: &'static [&'static str] = &[];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let project_path = default_gtk4_project_path();
        let ctx = Self::template_context(project)
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;

        templates::gtk4::scaffold(
            &project.backend_path::<Self>(),
            &ctx,
            &project.gtk_backend_crate_name(),
        )
        .await
        .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self { project_path })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_gtk4_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        _platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<PathBuf> {
        project
            .browser_runtime_plan(TargetPlatform::Linux, TargetBackend::Gtk4)
            .await?;
        build_gtk4(project, options).await
    }

    async fn package(
        &self,
        project: &Project,
        _platform: TargetPlatform,
        options: PackageOptions,
    ) -> eyre::Result<Artifact> {
        package_gtk4(project, options).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_gtk4(project).await
    }
}

fn default_gtk4_project_path() -> PathBuf {
    PathBuf::from("gtk4")
}

fn is_default_gtk4_project_path(s: &Path) -> bool {
    s == Path::new("gtk4")
}
