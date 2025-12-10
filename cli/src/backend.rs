//! Backend configuration and initialization for `WaterUI` projects.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{android::backend::AndroidBackend, apple::backend::AppleBackend, project::Project};

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
}

impl Backends {
    /// Check if no backends are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.android.is_none() && self.apple.is_none()
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
