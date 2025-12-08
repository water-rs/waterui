use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backend::Backend;

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
    async fn init(
        _project: &crate::project::Project,
    ) -> Result<Self, crate::backend::FailToInitBackend> {
        // create a Xcode project here
        todo!()
    }
}
