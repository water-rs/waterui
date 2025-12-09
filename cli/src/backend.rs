//! Backend configuration and initialization for `WaterUI` projects.

use serde::{Deserialize, Serialize};

use crate::{android::backend::AndroidBackend, apple::backend::AppleBackend, project::Project};

/// Configuration for all backends in a `WaterUI` project.
///
/// `[backend]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    android: Option<AndroidBackend>,
    apple: Option<AppleBackend>,
}

impl Backends {
    /// Check if no backends are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.android.is_none() && self.apple.is_none()
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
    /// Initialize the backend for the given project.
    ///
    /// Always, it would create necessary files/folders for the backend.
    ///
    /// # Warnings
    /// You cannot initialize any backend for a playground project.
    fn init(project: &Project) -> impl Future<Output = Result<Self, FailToInitBackend>> + Send;
}
