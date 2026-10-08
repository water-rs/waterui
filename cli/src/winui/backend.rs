//! `WinUI` backend configuration and initialization.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    templates::{self, TemplateContext},
    winui::platform::{build_winui, clean_winui, is_winui_platform, package_winui},
};

/// Configuration for the `WinUI` backend in a `WaterUI` project.
///
/// `[backend.winui]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WinUiBackend {
    #[serde(
        default = "default_winui_project_path",
        skip_serializing_if = "is_default_winui_project_path"
    )]
    project_path: PathBuf,
}

impl WinUiBackend {
    /// Create a new `WinUI` backend configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_winui_project_path(),
        }
    }

    /// Set a custom project path (defaults to "winui").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Get the path to the `WinUI` project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    /// Check whether managed `WinUI` backend files differ from the current templates.
    ///
    /// # Errors
    ///
    /// Returns an error when the application dependency graph or templates
    /// cannot be resolved.
    pub async fn requires_regeneration(project: &Project) -> eyre::Result<bool> {
        let backend_dir = project.backend_path::<Self>();
        let ctx = Self::template_context(project).await?;
        for (relative, expected) in
            templates::winui::rendered_outputs(&ctx, &project.winui_backend_crate_name())?
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
        let framework = project.resolved_framework().await?;
        Ok(TemplateContext::for_project_manifest(
            project.host(),
            manifest,
            project.crate_name().clone(),
            app_name,
            &framework,
            project.local_sources(),
        )
        .with_backend_project_path(project.backend_path::<Self>())
        .with_project_root_path(project.root().to_path_buf())
        .with_project_packages(project.project_packages(&framework).await?))
    }
}

impl Default for WinUiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for WinUiBackend {
    const DEFAULT_PATH: &'static str = "winui";

    // `WinUI` uses cargo's target directory for build caches.
    // Since the `WinUI` project is a simple Rust binary crate, it uses the workspace target.
    // No need to preserve local target - it's part of the workspace.
    const CACHE_PATHS: &'static [&'static str] = &[];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let project_path = default_winui_project_path();
        let ctx = Self::template_context(project)
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;

        templates::winui::scaffold(
            &project.backend_path::<Self>(),
            &ctx,
            &project.winui_backend_crate_name(),
        )
        .await
        .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self { project_path })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_winui_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        _platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<crate::build::BuiltTarget> {
        build_winui(project, options).await
    }

    async fn package(
        &self,
        project: &Project,
        _platform: TargetPlatform,
        options: PackageOptions,
        built: &crate::build::BuiltTarget,
    ) -> eyre::Result<Artifact> {
        package_winui(project, options, built).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_winui(project).await
    }
}

fn default_winui_project_path() -> PathBuf {
    PathBuf::from("winui")
}

fn is_default_winui_project_path(s: &Path) -> bool {
    s == Path::new("winui")
}
