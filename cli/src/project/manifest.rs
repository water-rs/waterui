use std::path::Path;

use serde::{Deserialize, Serialize};
use smol::fs::read_to_string;

use crate::{backend::Backends, permission::Permissions};

/// Configuration for a `WaterUI` project persisted to `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Manifest {
    pub package: Package,
    #[serde(default, skip_serializing_if = "Backends::is_empty")]
    pub backends: Backends,
    /// Path to local `WaterUI` repository for dev mode.
    /// When set, all backends will use this path instead of the published versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waterui_path: Option<String>,
    /// Permission configuration for playground projects.
    #[serde(default, skip_serializing_if = "Permissions::is_empty")]
    pub permissions: Permissions,
}

#[derive(Debug, thiserror::Error)]
pub enum FailToOpenManifest {
    #[error("Failed to read manifest file: {0}")]
    ReadError(std::io::Error),
    #[error("Invalid manifest file: {0}")]
    InvalidManifest(toml::de::Error),
    #[error("Manifest file not found at the specified path")]
    NotFound,
}
impl Manifest {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, FailToOpenManifest> {
        let path = path.as_ref();
        let result = read_to_string(path).await;

        match result {
            Ok(c) => toml::from_str(&c).map_err(FailToOpenManifest::InvalidManifest),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(FailToOpenManifest::NotFound);
            }
            Err(e) => return Err(FailToOpenManifest::ReadError(e)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    /// Type of the package (e.g., "app").
    #[serde(rename = "type")]
    pub package_type: PackageType,
    /// Human-readable name of the application (e.g., "Water Demo").
    pub name: String,
    pub bundle_identifier: String,
}

/// Package type indicating what kind of project this is.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    /// A standalone application with platform-specific backends.
    #[default]
    App,
    /// A playground project for quick experimentation.
    /// Platform projects are created in a temporary directory.
    Playground,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
// Warn: You cannot use both revision and local_path at the same time.
pub struct AppleBackend {
    #[serde(
        default = "default_apple_project_path",
        skip_serializing_if = "is_default_apple_project_path"
    )]
    project_path: String,
    scheme: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    /// Local path to the Apple backend for local dev.
    #[serde(skip_serializing_if = "Option::is_none")]
    backend_path: Option<String>,
}

impl AppleBackend {
    /// Get the path to the Apple project within the WaterUI project.
    #[must_use]
    pub fn project_path(&self) -> &str {
        &self.project_path
    }
}

fn default_apple_project_path() -> String {
    "apple".to_string()
}

fn is_default_apple_project_path(s: &str) -> bool {
    s == "apple"
}
