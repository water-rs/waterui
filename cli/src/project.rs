pub mod manifest;

mod app;
mod playground;
use std::path::{Path, PathBuf};

use cargo_toml::Manifest as CargoManifest;
use smol::fs::read_to_string;
use thiserror::Error;

use crate::android::backend::AndroidBackend;
use crate::platform::Platform;
use crate::project::manifest::{AppleBackend, FailToOpenManifest, Manifest, PackageType};

#[derive(Debug)]
pub struct Project {
    dir: PathBuf,
    manifest: Manifest,
    cargo_manifest: CargoManifest,
}

#[derive(Debug, Error)]
pub enum FailToOpenProject {
    #[error("Failed to read Cargo.toml: {0}")]
    FailToOpenCargoManifest(std::io::Error),
    #[error("Invalid Cargo.toml: {0}")]
    InvalidCargoManifest(cargo_toml::Error),
    #[error("No valid cargo package found")]
    NoCargoPackage,
    #[error("{0}")]
    Manifest(#[from] FailToOpenManifest),
}

impl Project {
    /// Open a `WaterUI` project from the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `Water.toml` file cannot be read from the directory
    /// - The TOML file cannot be parsed into a valid `ProjectConfig`
    /// - The `Cargo.toml` file cannot be read or parsed
    pub async fn open(dir: impl AsRef<Path>) -> Result<Self, FailToOpenProject> {
        let dir = dir.as_ref().to_path_buf();
        let manifest = Manifest::open(dir.join("Water.toml")).await?;

        let cargo_manifest = match read_to_string(dir.join("Cargo.toml")).await {
            Ok(content) => CargoManifest::from_str(&content)
                .map_err(FailToOpenProject::InvalidCargoManifest)?,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Err(FailToOpenProject::NoCargoPackage)?
                } else {
                    Err(FailToOpenProject::FailToOpenCargoManifest(e))?
                }
            }
        };

        // Let's ensure that the Cargo manifest has a package section

        if cargo_manifest.package.is_none() {
            Err(FailToOpenProject::NoCargoPackage)?
        }

        Ok(Self {
            dir,
            manifest,
            cargo_manifest,
        })
    }

    /// Create a new `WaterUI` app project in the specified directory.
    pub async fn create_app(
        name: impl Into<String>,
        display_name: impl Into<String>,
        bundle_id: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        todo!()
    }

    pub async fn create_playground() -> Self {
        todo!()
    }

    /// Get the display name of the project (human-readable).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.config.package.name
    }

    /// Get the crate name from Cargo.toml (kebab-case identifier).
    #[must_use]
    pub fn crate_name(&self) -> &str {
        &self.cargo_manifest.name
    }

    /// Get the unique identifier of the project (`snake_case` from crate name).
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get the author of the project from Cargo.toml.
    /// Returns the first author if multiple are specified.
    #[must_use]
    pub fn authors(&self) -> &[String] {
        self.cargo_manifest.package().authors()
    }

    /// Bundle identifier used for Apple/Android targets.
    #[must_use]
    pub fn bundle_identifier(&self) -> &str {
        &self.config.package.bundle_identifier
    }

    /// Get the root directory of the project.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Check if this is a playground project.
    #[must_use]
    pub fn is_playground(&self) -> bool {
        self.manifest.package.package_type == PackageType::Playground
    }

    pub fn apple_backend(&self) -> Option<&AppleBackend> {
        self.manifest.backends.apple.as_ref()
    }

    pub fn android_backend(&self) -> Option<&AndroidBackend> {
        self.manifest.backends.android.as_ref()
    }

    /// Get the package type.
    #[must_use]
    pub const fn package_type(&self) -> PackageType {
        self.manifest.package().package_type
    }
}
