use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backend::Backend;

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

impl Backend for AndroidBackend {
    async fn init(
        _project: &crate::project::Project,
    ) -> Result<Self, crate::backend::FailToInitBackend> {
        // Create a default Android project structure here
        todo!()
    }
}

fn default_android_project_path() -> PathBuf {
    PathBuf::from("android")
}

fn is_default_android_project_path(s: &Path) -> bool {
    s == Path::new("android")
}
