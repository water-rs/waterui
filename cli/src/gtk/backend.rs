//! GTK backend configuration and initialization.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    project::Project,
    templates::{self, TemplateContext},
};

/// Configuration for the GTK backend in a `WaterUI` project.
///
/// `[backend.gtk]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GtkBackend {
    #[serde(
        default = "default_gtk_project_path",
        skip_serializing_if = "is_default_gtk_project_path"
    )]
    project_path: PathBuf,
}

impl GtkBackend {
    /// Create a new GTK backend configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_gtk_project_path(),
        }
    }

    /// Set a custom project path (defaults to "gtk").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Get the path to the GTK project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }
}

impl Default for GtkBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for GtkBackend {
    const DEFAULT_PATH: &'static str = "gtk";

    // GTK uses cargo's target directory for build caches
    // Since GTK project is a simple Rust binary crate, it uses the workspace target
    // No need to preserve local target - it's part of the workspace
    const CACHE_PATHS: &'static [&'static str] = &[];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();

        // Get the relative path to the backend from project root (e.g., "gtk" or ".water/gtk")
        let backend_relative_path = project.backend_relative_path::<Self>();

        let project_path = default_gtk_project_path();

        let ctx = TemplateContext {
            app_display_name: manifest.package.name.clone(),
            app_name: manifest
                .package
                .name
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect(),
            crate_name: project.crate_name().to_string(),
            bundle_identifier: manifest.package.bundle_identifier.clone(),
            author: String::new(),
            android_backend_path: None,
            use_remote_dev_backend: manifest.waterui_path.is_none(),
            waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
            backend_project_path: Some(backend_relative_path),
            android_permissions: Vec::new(),
        };

        templates::gtk::scaffold(&project.backend_path::<Self>(), &ctx)
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(Self { project_path })
    }
}

fn default_gtk_project_path() -> PathBuf {
    PathBuf::from("gtk")
}

fn is_default_gtk_project_path(s: &Path) -> bool {
    s == Path::new("gtk")
}
