//! Backend configuration and initialization for `WaterUI` projects.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    apple::backend::AppleBackend,
    build::{BuildOptions, BuiltTarget},
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
};

/// The runtime backends a [`Project`] manages.
///
/// Project-owned state filled when [`Project::open`] generates the selected
/// backends in the managed build cache — never persisted in `Water.toml`.
/// Persisted backend-facing configuration lives in the manifest's typed
/// tables (`[esp32]` device configuration, `[hydrolysis]` painter).
#[derive(Debug, Clone, Default)]
pub struct Backends {
    apple: Option<AppleBackend>,
}

/// The `[hydrolysis]` table: selections the project author declares for
/// the Hydrolysis backend.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct HydrolysisConfig {
    /// The Android painter the generated app mounts on the shared Hydrolysis
    /// host. Absent means the GPU painter; an explicit painter the pinned
    /// host cannot serve is an error at scaffold time, never a substitute.
    pub painter: Option<crate::hydrolysis::android::HydrolysisAndroidPainter>,
}

impl Backends {
    /// Check if no backends are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.apple.is_none()
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
}

/// Error type for failing to initialize a backend.
#[derive(Debug, thiserror::Error)]
pub enum FailToInitBackend {
    /// I/O error while scaffolding templates.
    #[error("Failed to write template files: {0}")]
    Io(#[from] std::io::Error),
    /// Invalid backend configuration prevented scaffolding (e.g. an
    /// unsupported chip in `[esp32]`).
    #[error("Invalid backend configuration: {0:#}")]
    Config(eyre::Error),
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
    /// Returns the Cargo-reported target and artifacts from the build.
    fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> impl Future<Output = eyre::Result<BuiltTarget>> + Send;

    /// Package the project for the target platform.
    ///
    /// Returns the artifact (e.g., .app, .apk, binary).
    fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
        built: &BuiltTarget,
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
