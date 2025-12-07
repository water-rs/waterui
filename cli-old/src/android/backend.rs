use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
    pub fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    pub fn gradlew_path(&self) -> PathBuf {
        let base = &self.project_path;
        if cfg!(windows) {
            base.join("gradlew.bat")
        } else {
            base.join("gradlew")
        }
    }
}

#[must_use]
pub fn default_android_project_path() -> String {
    "android".to_string()
}

fn is_default_android_project_path(s: &str) -> bool {
    s == "android"
}
