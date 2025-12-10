use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
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

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(
        project: &crate::project::Project,
    ) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();

        // Derive app name from the display name (remove spaces for filesystem)
        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();

        // Get the relative path to the backend from project root (e.g., "android" or ".water/android")
        let backend_relative_path = project.backend_relative_path::<Self>();

        // Determine the Android backend path - this is unused for local dev,
        // kept for potential future remote backend support
        let android_backend_path = manifest
            .waterui_path
            .as_ref()
            .map(|p| PathBuf::from(p).join("backends/android"));

        let project_path = default_android_project_path();

        // Extract enabled permissions from the manifest
        let android_permissions: Vec<String> = manifest
            .permissions
            .iter()
            .filter(|(_, entry)| entry.is_enabled())
            .map(|(name, _)| name.clone())
            .collect();

        let ctx = TemplateContext {
            app_display_name: manifest.package.name.clone(),
            app_name,
            crate_name: project.crate_name().to_string(),
            bundle_identifier: manifest.package.bundle_identifier.clone(),
            author: String::new(),
            android_backend_path,
            use_remote_dev_backend: manifest.waterui_path.is_none(),
            waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
            backend_project_path: Some(backend_relative_path),
            android_permissions,
        };

        templates::android::scaffold(&project.backend_path::<Self>(), &ctx)
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self {
            project_path,
            version: None,
        })
    }
}

fn default_android_project_path() -> PathBuf {
    PathBuf::from("android")
}

fn is_default_android_project_path(s: &Path) -> bool {
    s == Path::new("android")
}
