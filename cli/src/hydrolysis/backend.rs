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

    /// Check whether generated hydrolysis backend files should be regenerated.
    ///
    /// This is used by playground mode where backend glue code is fully managed by the CLI.
    ///
    /// # Errors
    ///
    /// Returns an error when backend `Cargo.toml` exists but cannot be parsed.
    pub fn requires_regeneration(project: &Project) -> eyre::Result<bool> {
        let backend_dir = project.backend_path::<Self>();
        let ctx = Self::template_context(project);
        let outputs = templates::hydrolysis::rendered_outputs(
            &ctx,
            &project.hydrolysis_backend_crate_name(),
        )?;
        for (relative, expected) in outputs {
            let path = backend_dir.join(&relative);
            // Preview binding files are rewritten with target-specific
            // content on every preview invocation; only their presence is
            // managed here.
            let per_run_binding = relative == Path::new("src/preview_symbol.rs")
                || relative == Path::new("src/preview_test.rs");
            match std::fs::read(&path) {
                Ok(existing) if per_run_binding || existing == expected => {}
                Ok(_) | Err(_) => return Ok(true),
            }
        }
        Ok(false)
    }

    /// The template context the CLI manages this backend with; regeneration
    /// compares the backend on disk against exactly this rendering.
    fn template_context(project: &Project) -> TemplateContext {
        let manifest = project.manifest();
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        TemplateContext::for_project_manifest(manifest, project.crate_name().clone(), app_name)
            .with_backend_project_path(project.backend_path::<Self>())
            .with_project_root_path(project.root().to_path_buf())
    }
}

impl Default for HydrolysisBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for HydrolysisBackend {
    const DEFAULT_PATH: &'static str = "hydrolysis";

    // The build cache lives in the repository `target/`; the lockfile is
    // dependency state, not generated content, and survives regeneration so
    // resolved versions stay stable across template updates.
    const CACHE_PATHS: &'static [&'static str] = &["Cargo.lock"];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let project_path = default_hydrolysis_project_path();
        let ctx = Self::template_context(project);

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
