//! Hydrolysis backend configuration and initialization.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    android::platform::AndroidAbi,
    backend::{Backend, reinit_backend},
    build::BuildOptions,
    device::Artifact,
    hydrolysis::platform::{
        build_hydrolysis, clean_hydrolysis, is_hydrolysis_platform, package_hydrolysis,
    },
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::{ManagedBackends, Project},
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
    /// The backend crate is generated and managed by the CLI.
    ///
    /// # Errors
    ///
    /// Returns an error when backend `Cargo.toml` exists but cannot be parsed.
    pub async fn requires_regeneration(project: &Project) -> eyre::Result<bool> {
        let backend_dir = project.backend_path::<Self>();
        let ctx = Self::template_context(project).await?;
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
    /// compares the backend on disk against exactly this rendering, and the
    /// Android app scaffold layers its parameters on top of it.
    pub(crate) async fn template_context(project: &Project) -> eyre::Result<TemplateContext> {
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
        .with_project_packages(project.project_packages(&framework).await?)
        .with_webview_enabled(project.uses_standard_webview().await?)
        .with_chromium_enabled(project.links_runtime_package("waterui-chromium").await?)
        .with_browser_engine(project.linked_browser_engine().await?))
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
    // resolved versions stay stable across template updates. The `.seed`
    // copy records which application lockfile last seeded it, so it is
    // preserved on the same grounds.
    const CACHE_PATHS: &'static [&'static str] = &[
        "Cargo.lock",
        templates::LOCKFILE_SEED,
        // The generated Gradle app and the pinned host checkout survive
        // regeneration: the checkout is an expensive fetch and the app only
        // changes when the painter or project does, which re-scaffolds
        // directly rather than through `reinit_backend`.
        "android",
        "android-host",
    ];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let project_path = default_hydrolysis_project_path();
        let ctx = Self::template_context(project)
            .await
            .map_err(crate::backend::FailToInitBackend::Config)?;

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
        // Android goes through `hydrolysis::android` — the launcher crate,
        // the managed host checkout and the generated Gradle app — rather
        // than the desktop `build_hydrolysis` path.
        is_hydrolysis_platform(platform) || platform == TargetPlatform::Android
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<crate::build::BuiltTarget> {
        if platform == TargetPlatform::Android {
            return crate::hydrolysis::android::build(
                project,
                project.host(),
                AndroidAbi::Arm64V8a,
                options,
            )
            .await;
        }
        project
            .browser_runtime_plan(platform, TargetBackend::Hydrolysis)
            .await?;
        build_hydrolysis(project, platform, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
        built: &crate::build::BuiltTarget,
    ) -> eyre::Result<Artifact> {
        if platform == TargetPlatform::Android {
            let prepared = crate::android::signing::PreparedSigning::resolve(project, &options)?;
            return crate::hydrolysis::android::package_with_abis(
                project,
                project.host(),
                crate::hydrolysis::android::resolve_painter(project, None),
                &options,
                &[AndroidAbi::Arm64V8a],
                built,
                &prepared,
            )
            .await;
        }
        package_hydrolysis(project, platform, options, Some(built)).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_hydrolysis(project).await
    }
}

/// Open the project at `project_path` with its managed Hydrolysis backend
/// generated and matching the current templates.
///
/// Every flow that builds the managed launcher crate outside `water run` —
/// preview, the MCP child and the inspector support app — opens its project
/// through this.
///
/// # Errors
///
/// Returns an error when the project cannot be opened or the backend cannot
/// be regenerated.
pub async fn open_ready(
    host: &crate::toolchain::Host,
    project_path: &Path,
) -> eyre::Result<Project> {
    let project = Project::open(
        host,
        project_path,
        ManagedBackends::for_backend(TargetBackend::Hydrolysis),
    )
    .await?;
    if HydrolysisBackend::requires_regeneration(&project).await? {
        reinit_backend::<HydrolysisBackend>(&project).await?;
    }
    Ok(project)
}

fn default_hydrolysis_project_path() -> PathBuf {
    PathBuf::from("hydrolysis")
}

fn is_default_hydrolysis_project_path(s: &Path) -> bool {
    s == Path::new("hydrolysis")
}
