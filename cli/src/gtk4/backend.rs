//! GTK4 backend configuration and initialization.

use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    gtk4::platform::{build_gtk4, clean_gtk4, is_gtk4_platform, package_gtk4},
    platform::{PackageOptions, TargetPlatform},
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
        let manifest = project.manifest();

        let project_path = default_gtk4_project_path();

        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let ctx = TemplateContext::for_project_manifest(
            manifest,
            project.crate_name().clone(),
            app_name,
        )
        .with_backend_project_path(project.backend_path::<Self>())
        .with_project_root_path(project.root().to_path_buf());

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
