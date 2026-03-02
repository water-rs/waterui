//! Hydrolysis backend configuration and initialization.

use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    hydrolysis::platform::{
        build_hydrolysis, clean_hydrolysis, is_hydrolysis_platform, package_hydrolysis,
    },
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    templates::{self, TemplateContext},
};

/// Configuration for the hydrolysis backend in a `WaterUI` project.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HydrolysisBackend {
    #[serde(
        default = "default_hydrolysis_project_path",
        skip_serializing_if = "is_default_hydrolysis_project_path"
    )]
    project_path: PathBuf,
}

impl HydrolysisBackend {
    /// Create a new hydrolysis backend configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_hydrolysis_project_path(),
        }
    }

    /// Set a custom project path (defaults to "hydrolysis").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Get the path to the hydrolysis project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }
}

impl Default for HydrolysisBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for HydrolysisBackend {
    const DEFAULT_PATH: &'static str = "hydrolysis";

    // Hydrolysis uses Cargo build cache in `target/`.
    const CACHE_PATHS: &'static [&'static str] = &[];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();
        let backend_relative_path = project.backend_relative_path::<Self>();
        let project_path = default_hydrolysis_project_path();

        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let ctx = TemplateContext::for_project_manifest(
            manifest,
            project.crate_name().to_string(),
            app_name,
        )
        .with_backend_project_path(backend_relative_path);

        templates::hydrolysis::scaffold(
            &project.backend_path::<Self>(),
            &ctx,
            &project.hydrolysis_backend_crate_name(),
        )
        .await
        .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self { project_path })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_hydrolysis_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<PathBuf> {
        build_hydrolysis(project, platform, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
    ) -> eyre::Result<Artifact> {
        package_hydrolysis(project, platform, options).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_hydrolysis(project).await
    }
}

fn default_hydrolysis_project_path() -> PathBuf {
    PathBuf::from("hydrolysis")
}

fn is_default_hydrolysis_project_path(s: &Path) -> bool {
    s == Path::new("hydrolysis")
}
