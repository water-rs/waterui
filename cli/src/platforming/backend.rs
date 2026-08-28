//! Backend configuration and initialization for `WaterUI` projects.

use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    android::backend::AndroidBackend,
    apple::backend::AppleBackend,
    build::BuildOptions,
    device::Artifact,
    esp32::backend::Esp32Backend,
    gtk4::backend::Gtk4Backend,
    hydrolysis::backend::HydrolysisBackend,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
};

/// Configuration for all backends in a `WaterUI` project.
///
/// `[backend]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    /// Base path for all backends, relative to project root.
    /// Empty string means project root for app manifests.
    /// Playground projects do not persist managed backend paths in `Water.toml`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    android: Option<AndroidBackend>,
    apple: Option<AppleBackend>,
    gtk4: Option<Gtk4Backend>,
    hydrolysis: Option<HydrolysisBackend>,
    esp32: Option<Esp32Backend>,
}

impl Backends {
    /// Check if no backends are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.android.is_none()
            && self.apple.is_none()
            && self.gtk4.is_none()
            && self.hydrolysis.is_none()
            && self.esp32.is_none()
    }

    #[cfg(test)]
    pub(crate) fn set_esp32_for_tests(&mut self, backend: Esp32Backend) {
        self.esp32 = Some(backend);
    }

    /// Whether any backend-project scaffolding is configured.
    ///
    /// `[backends.esp32]` is deliberately excluded: it carries device
    /// configuration — chip, panel geometry, bundled fonts — that only the
    /// app author can know, while the other entries describe backend
    /// projects that playground mode delegates to the CLI.
    #[must_use]
    pub const fn configures_backend_projects(&self) -> bool {
        self.android.is_some()
            || self.apple.is_some()
            || self.gtk4.is_some()
            || self.hydrolysis.is_some()
    }

    /// Get the base path for backends, relative to project root.
    #[must_use]
    pub fn path(&self) -> &Path {
        Path::new(&self.path)
    }

    /// Set the base path for backends.
    pub fn set_path(&mut self, path: impl Into<String>) {
        self.path = path.into();
    }

    /// Get the Android backend configuration, if any.
    #[must_use]
    pub const fn android(&self) -> Option<&AndroidBackend> {
        self.android.as_ref()
    }

    /// Get the Apple backend configuration, if any.
    #[must_use]
    pub const fn apple(&self) -> Option<&AppleBackend> {
        self.apple.as_ref()
    }

    /// Set the Apple backend configuration.
    pub fn set_apple(&mut self, backend: AppleBackend) {
        self.apple = Some(backend);
    }

    /// Remove Apple backend configuration.
    pub fn clear_apple(&mut self) {
        self.apple = None;
    }

    /// Set the Android backend configuration.
    pub fn set_android(&mut self, backend: AndroidBackend) {
        self.android = Some(backend);
    }

    /// Remove Android backend configuration.
    pub fn clear_android(&mut self) {
        self.android = None;
    }

    /// Get the GTK4 backend configuration, if any.
    #[must_use]
    pub const fn gtk4(&self) -> Option<&Gtk4Backend> {
        self.gtk4.as_ref()
    }

    /// Set the GTK4 backend configuration.
    pub fn set_gtk4(&mut self, backend: Gtk4Backend) {
        self.gtk4 = Some(backend);
    }

    /// Remove GTK4 backend configuration.
    pub fn clear_gtk4(&mut self) {
        self.gtk4 = None;
    }

    /// Get the hydrolysis backend configuration, if any.
    #[must_use]
    pub const fn hydrolysis(&self) -> Option<&HydrolysisBackend> {
        self.hydrolysis.as_ref()
    }

    /// Set the hydrolysis backend configuration.
    pub fn set_hydrolysis(&mut self, backend: HydrolysisBackend) {
        self.hydrolysis = Some(backend);
    }

    /// Remove hydrolysis backend configuration.
    pub fn clear_hydrolysis(&mut self) {
        self.hydrolysis = None;
    }

    /// Get the ESP32 backend configuration, if any.
    #[must_use]
    pub const fn esp32(&self) -> Option<&Esp32Backend> {
        self.esp32.as_ref()
    }

    /// Set the ESP32 backend configuration.
    pub fn set_esp32(&mut self, backend: Esp32Backend) {
        self.esp32 = Some(backend);
    }

    /// Remove ESP32 backend configuration.
    pub fn clear_esp32(&mut self) {
        self.esp32 = None;
    }
}

/// Error type for failing to initialize a backend.
#[derive(Debug, thiserror::Error)]
pub enum FailToInitBackend {
    /// I/O error while scaffolding templates.
    #[error("Failed to write template files: {0}")]
    Io(#[from] std::io::Error),
    /// Invalid backend configuration prevented scaffolding (e.g. an
    /// unsupported chip in `[backends.esp32]`).
    #[error("Invalid backend configuration: {0}")]
    Config(#[source] color_eyre::eyre::Error),
}

/// Trait for backends in a `WaterUI` project.
///
/// A backend handles building and packaging for specific platforms.
/// Each backend knows:
/// - Which platforms it supports
/// - How to build Rust code for those platforms
/// - How to package artifacts for distribution
pub trait Backend: Sized + Send + Sync {
    /// The default relative path for this backend (e.g., "android", "apple").
    const DEFAULT_PATH: &'static str;

    /// Paths relative to the backend directory that should be preserved during re-scaffolding.
    ///
    /// These typically contain build caches that are expensive to regenerate.
    /// During `reinit_backend()`, only items NOT in this list are deleted before calling `init()`.
    const CACHE_PATHS: &'static [&'static str];

    /// Get the relative path for this backend instance.
    ///
    /// This is relative to `Backends::path()`.
    fn path(&self) -> &Path;

    /// Initialize the backend for the given project.
    ///
    /// Creates necessary files/folders for the backend at `project.backend_path::<Self>()`.
    /// Returns the initialized backend configuration.
    fn init(project: &Project) -> impl Future<Output = Result<Self, FailToInitBackend>> + Send;

    // =========================================================================
    // New methods for build/package (migrated from Platform trait)
    // =========================================================================

    /// Check if this backend supports the given platform.
    fn supports(&self, platform: TargetPlatform) -> bool;

    /// Build the Rust library for the target platform.
    ///
    /// Returns the target directory path where the built library is located.
    fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> impl Future<Output = eyre::Result<PathBuf>> + Send;

    /// Package the project for the target platform.
    ///
    /// Returns the artifact (e.g., .app, .apk, binary).
    fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
    ) -> impl Future<Output = eyre::Result<Artifact>> + Send;

    /// Clean build artifacts for the platform.
    fn clean(
        &self,
        project: &Project,
        platform: TargetPlatform,
    ) -> impl Future<Output = eyre::Result<()>> + Send;
}

/// Re-initialize a backend, preserving cache directories.
///
/// This function:
/// 1. Identifies cache paths that should be preserved (from `Backend::CACHE_PATHS`)
/// 2. Deletes all non-cache items in the backend directory
/// 3. Calls `Backend::init()` to re-scaffold the backend
///
/// This allows template updates to be applied while keeping expensive build caches.
///
/// # Errors
/// Returns an error if the backend directory cannot be read, cleaned, or re-initialized.
pub async fn reinit_backend<B: Backend>(project: &Project) -> Result<B, FailToInitBackend> {
    let backend_path = project.backend_path::<B>();

    if backend_path.exists() {
        // Get cache paths to preserve
        let cache_paths: std::collections::HashSet<&str> = B::CACHE_PATHS.iter().copied().collect();

        // Delete only non-cache items
        let entries = std::fs::read_dir(&backend_path)?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if !cache_paths.contains(&*name_str) {
                let path = entry.path();
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                } else {
                    std::fs::remove_file(&path)?;
                }
            }
        }
    }

    // Re-scaffold templates (cache dirs untouched)
    B::init(project).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[backends.esp32]` is device configuration, not backend-project
    /// scaffolding, so it alone must not trip the playground restriction.
    #[test]
    fn esp32_device_config_is_not_backend_project_configuration() {
        let mut backends = Backends::default();
        assert!(!backends.configures_backend_projects());

        backends.set_esp32_for_tests(Esp32Backend::new());
        assert!(!backends.configures_backend_projects());
        assert!(!backends.is_empty());

        backends.set_gtk4(Gtk4Backend::default());
        assert!(backends.configures_backend_projects());
    }
}
