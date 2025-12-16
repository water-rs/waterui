//! Backend configuration and initialization for `WaterUI` projects.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    android::backend::AndroidBackend,
    apple::backend::AppleBackend,
    gtk::backend::GtkBackend,
    project::Project,
};

/// Configuration for all backends in a `WaterUI` project.
///
/// `[backend]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    /// Base path for all backends, relative to project root.
    /// Empty string means project root (for apps), `.water` for playgrounds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    android: Option<AndroidBackend>,
    apple: Option<AppleBackend>,
    gtk: Option<GtkBackend>,
}

impl Backends {
    /// Check if no backends are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.android.is_none() && self.apple.is_none() && self.gtk.is_none()
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

    /// Set the Android backend configuration.
    pub fn set_android(&mut self, backend: AndroidBackend) {
        self.android = Some(backend);
    }

    /// Get the GTK backend configuration, if any.
    #[must_use]
    pub const fn gtk(&self) -> Option<&GtkBackend> {
        self.gtk.as_ref()
    }

    /// Set the GTK backend configuration.
    pub fn set_gtk(&mut self, backend: GtkBackend) {
        self.gtk = Some(backend);
    }
}

/// Error type for failing to initialize a backend.
#[derive(Debug, thiserror::Error)]
pub enum FailToInitBackend {
    /// I/O error while scaffolding templates.
    #[error("Failed to write template files: {0}")]
    Io(#[from] std::io::Error),
}

/// Trait for backends in a `WaterUI` project.
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
}

/// Re-initialize a backend, preserving cache directories.
///
/// This function:
/// 1. Identifies cache paths that should be preserved (from `Backend::CACHE_PATHS`)
/// 2. Deletes all non-cache items in the backend directory
/// 3. Calls `Backend::init()` to re-scaffold the backend
///
/// This allows template updates to be applied while keeping expensive build caches.
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

            if !cache_paths.contains(name_str.as_ref()) {
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
