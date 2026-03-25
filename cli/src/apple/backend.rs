use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    apple::platform::{build_rust_lib, clean_apple, is_apple_platform, package_apple},
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    project_types::CrateName,
    templates::{self, TemplateContext},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
// Warn: You cannot use both revision and local_path at the same time.
/// Configuration for the Apple backend in a `WaterUI` project.
///
/// `[backend.apple]` in `Water.toml`
pub struct AppleBackend {
    #[serde(
        default = "default_apple_project_path",
        skip_serializing_if = "is_default_apple_project_path"
    )]
    /// Path to the Apple project within the `WaterUI` project.
    pub project_path: PathBuf,
    /// The scheme to use for building the Apple project.
    pub scheme: String,
    /// The branch of the Apple backend to use.
    ///
    /// You cannot use both branch and revision at the same time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// The revision (commit hash or tag) of the Apple backend to use.
    ///
    /// You cannot use both revision and branch at the same time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Local path to the Apple backend for local dev.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_path: Option<String>,
}

impl AppleBackend {
    /// Create a new Apple backend configuration with the given scheme.
    #[must_use]
    pub fn new(scheme: impl Into<String>) -> Self {
        Self {
            project_path: default_apple_project_path(),
            scheme: scheme.into(),
            branch: None,
            revision: None,
            backend_path: None,
        }
    }

    /// Set a custom project path (defaults to "apple").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Set the local backend path for development.
    #[must_use]
    pub fn with_backend_path(mut self, path: impl Into<String>) -> Self {
        self.backend_path = Some(path.into());
        self
    }

    /// Get the path to the Apple project within the `WaterUI` project.
    #[must_use]
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }
}

fn default_apple_project_path() -> PathBuf {
    PathBuf::from("apple")
}

fn is_default_apple_project_path(s: &Path) -> bool {
    s == Path::new("apple")
}

impl Backend for AppleBackend {
    const DEFAULT_PATH: &'static str = "apple";

    // Preserve Xcode build caches during re-scaffolding.
    const CACHE_PATHS: &'static [&'static str] = &["DerivedData"];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();
        let effective_waterui_path = manifest.waterui_path.clone();

        // For playground projects, use fixed scheme name "WaterUIApp"
        // For regular projects, scheme name must match the Xcode target name (crate name)
        let is_playground =
            manifest.package.package_type == crate::project::PackageType::Playground;

        // For playground projects, use fixed names
        // For regular projects, derive from crate name
        let (scheme, app_name, crate_name_for_template) = if is_playground {
            (
                "WaterUIApp".to_string(),
                "WaterUIApp".to_string(),
                CrateName::try_from("WaterUIApp").expect("playground crate name must be valid"),
            )
        } else {
            let crate_name = project.crate_name().clone();
            // App name for Swift code must be a valid Swift identifier (no hyphens)
            // Convert "video-player-example" to "VideoPlayerExample"
            let app_name = crate_name
                .as_str()
                .split('-')
                .map(|s| {
                    let mut chars = s.chars();
                    chars.next().map_or_else(String::new, |first| {
                        first.to_uppercase().chain(chars).collect()
                    })
                })
                .collect::<String>();
            (crate_name.to_string(), app_name, crate_name)
        };

        let project_path = default_apple_project_path();

        let ios_permissions = manifest
            .permissions
            .iter()
            .filter(|(_, entry)| entry.is_enabled())
            .filter_map(|(key, entry)| {
                key.ios_plist_key()
                    .map(|plist_key| templates::IosPermissionTemplateEntry {
                        plist_key,
                        description: entry.description().to_string(),
                    })
            })
            .collect();
        let ctx =
            TemplateContext::for_project_manifest(manifest, crate_name_for_template, app_name)
                .with_backend_project_path(project.backend_path::<Self>())
                .with_project_root_path(project.root().to_path_buf())
                .with_ios_permissions(ios_permissions);

        templates::apple::scaffold(&project.backend_path::<Self>(), &ctx)
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self {
            project_path,
            scheme,
            branch: None,
            revision: None,
            backend_path: effective_waterui_path,
        })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_apple_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<PathBuf> {
        build_rust_lib(project, platform, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
    ) -> eyre::Result<Artifact> {
        package_apple(project, platform, options).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_apple(project).await
    }
}
