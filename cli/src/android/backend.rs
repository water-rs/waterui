use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    android::platform::{AndroidAbi, AndroidPlatform, clean_android, is_android_platform},
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    platform::{PackageOptions, TargetBackend, TargetPlatform},
    project::Project,
    templates::{self, TemplateContext},
};

/// Configuration for the Android backend in a `WaterUI` project.
///
/// `[backend.android]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AndroidBackend {
    #[serde(
        default = "default_android_project_path",
        skip_serializing_if = "is_default_android_project_path"
    )]
    project_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

impl AndroidBackend {
    /// Create a new Android backend configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_android_project_path(),
            version: None,
        }
    }

    /// Set a custom project path (defaults to "android").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Get the path to the Android project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    /// Get the path to the Gradle wrapper script within the Android project.
    #[must_use]
    pub fn gradlew_path(&self) -> PathBuf {
        let base = &self.project_path;
        if cfg!(windows) {
            base.join("gradlew.bat")
        } else {
            base.join("gradlew")
        }
    }
}

impl Default for AndroidBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for AndroidBackend {
    const DEFAULT_PATH: &'static str = "android";

    // Preserve Gradle build caches during re-scaffolding
    const CACHE_PATHS: &'static [&'static str] = &[".gradle", "build", "app"];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();

        // Derive app name from the display name (remove spaces for filesystem)
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();

        let project_path = default_android_project_path();

        // Extract enabled permissions from the manifest
        let android_permissions = manifest
            .permissions
            .iter()
            .filter(|(_, entry)| entry.is_enabled())
            .filter_map(|(key, _)| {
                key.android_permission_name()
                    .map(|name| templates::AndroidPermissionTemplateEntry { name })
            })
            .collect();

        let ctx =
            TemplateContext::for_project_manifest(manifest, project.crate_name().clone(), app_name)
                .with_backend_project_path(project.backend_path::<Self>())
                .with_project_root_path(project.root().to_path_buf())
                .with_android_permissions(android_permissions);

        templates::android::scaffold(&project.backend_path::<Self>(), &ctx)
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self {
            project_path,
            version: None,
        })
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_android_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<PathBuf> {
        debug_assert_eq!(platform, TargetPlatform::Android);
        project
            .browser_runtime_plan(platform, TargetBackend::Android)
            .await?;
        AndroidPlatform::arm64().build(project, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
    ) -> eyre::Result<Artifact> {
        debug_assert_eq!(platform, TargetPlatform::Android);
        AndroidPlatform::package_with_abis(project, options, &[AndroidAbi::Arm64V8a]).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_android(project).await
    }
}

fn default_android_project_path() -> PathBuf {
    PathBuf::from("android")
}

fn is_default_android_project_path(s: &Path) -> bool {
    s == Path::new("android")
}
