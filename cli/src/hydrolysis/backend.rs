//! Hydrolysis backend configuration and initialization.

use std::path::{Path, PathBuf};

use cargo_toml::Manifest as CargoManifest;
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
        let cargo_toml_path = project.backend_path::<Self>().join("Cargo.toml");
        let lib_rs_path = project.backend_path::<Self>().join("src/lib.rs");
        let main_rs_path = project.backend_path::<Self>().join("src/main.rs");
        let preview_runtime_path = project
            .backend_path::<Self>()
            .join("src/preview_runtime.rs");
        let preview_symbol_path = project.backend_path::<Self>().join("src/preview_symbol.rs");
        let web_index_path = project.backend_path::<Self>().join("web/index.html");
        if !cargo_toml_path.exists() {
            return Ok(true);
        }

        let manifest =
            CargoManifest::<cargo_toml::Value>::from_path(&cargo_toml_path).map_err(|error| {
                eyre::eyre!("failed to parse {}: {error}", cargo_toml_path.display())
            })?;
        let main_rs = std::fs::read_to_string(&main_rs_path).unwrap_or_default();
        let lib_rs = std::fs::read_to_string(&lib_rs_path).unwrap_or_default();
        let main_supports_preview = {
            main_rs.contains("feature = \"waterui-preview-mode\"")
                && main_rs.contains("mod preview_symbol;")
        };
        let main_installs_theme_after_app = runtime_installs_theme_after_app(&main_rs);
        let lib_installs_theme_after_app = runtime_installs_theme_after_app(&lib_rs);
        let preview_runtime_supports_symbol = std::fs::read_to_string(&preview_runtime_path)
            .map(|content| {
                content.contains("use crate::preview_symbol;")
                    && content.contains("flatten_alpha_over_white")
            })
            .unwrap_or(false);

        Ok(!manifest.dependencies.contains_key("waterui")
            || manifest.lib.is_none()
            || !lib_rs_path.exists()
            || !main_rs_path.exists()
            || !preview_runtime_path.exists()
            || !preview_symbol_path.exists()
            || !web_index_path.exists()
            || !main_supports_preview
            || !main_installs_theme_after_app
            || !lib_installs_theme_after_app
            || !preview_runtime_supports_symbol)
    }
}

fn runtime_installs_theme_after_app(content: &str) -> bool {
    content.contains("let mut app =")
        && content.contains("hydrolysis_m3::install_defaults(&mut app.env);")
        && content.contains("hydrolysis::run(app);")
}

#[cfg(test)]
mod tests {
    use super::runtime_installs_theme_after_app;

    #[test]
    fn runtime_install_check_accepts_post_app_default_theme_install() {
        let content = "let env = waterui::configure_environment!(waterui::env::Environment::new()); let mut app = form_example::app(env); hydrolysis_m3::install_defaults(&mut app.env); hydrolysis::run(app);";

        assert!(runtime_installs_theme_after_app(content));
    }

    #[test]
    fn runtime_install_check_rejects_pre_app_theme_install() {
        let content = "let mut env = waterui::configure_environment!(waterui::env::Environment::new()); hydrolysis_m3::install(&mut env); let app = form_example::app(env); hydrolysis::run(app);";

        assert!(!runtime_installs_theme_after_app(content));
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
        let project_path = default_hydrolysis_project_path();

        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let ctx =
            TemplateContext::for_project_manifest(manifest, project.crate_name().clone(), app_name)
                .with_backend_project_path(project.backend_path::<Self>())
                .with_project_root_path(project.root().to_path_buf());

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
