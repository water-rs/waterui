use cargo_toml::Manifest as CargoManifest;
use color_eyre::eyre;

/// Represents a `WaterUI` project with its manifest and crate information.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    manifest: Manifest,
    crate_name: String,
}

impl Project {
    /// Build the `WaterUI` project.
    ///
    /// Equivalent to running `water build` in the project directory.
    ///
    /// Unlike `Platform::build`, this method returns the path to the built artifact, instead of the target directory.
    ///
    /// # Errors
    /// - If the build process fails for any reason.
    pub async fn build(
        &self,
        platform: impl Platform,
        options: BuildOptions,
    ) -> Result<PathBuf, eyre::Report> {
        platform.build(self, options).await
    }

    /// Run the `WaterUI` project on the specified device.
    ///
    /// This method handles building, packaging, and running the project.
    ///
    /// # Errors
    /// - If any step in the build, package, or run process fails.
    pub async fn run(&self, device: impl Device, _hot_reload: bool) -> Result<Running, FailToRun> {
        let platform = device.platform();

        // Build rust library for the target platform
        platform
            .build(self, BuildOptions::new(false))
            .await
            .map_err(FailToRun::Build)?;

        // Package the build artifacts for the target platform
        let artifact = platform
            .package(self, PackageOptions::new(false, true))
            .await
            .map_err(FailToRun::Package)?;

        // TODO: Set up hot reload

        let running = device.run(artifact, RunOptions::new()).await?;

        Ok(running)
    }

    /// Get the root path of the project.
    ///
    /// Same as the directory containing `Water.toml`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the backends configured for the project.
    #[must_use]
    pub const fn backends(&self) -> &Backends {
        &self.manifest.backends
    }

    /// Get the crate name of the project.
    #[must_use]
    pub fn crate_name(&self) -> &str {
        &self.crate_name
    }

    /// Get the Apple backend configuration if available.
    #[must_use]
    pub const fn apple_backend(&self) -> Option<&AppleBackend> {
        self.manifest.backends.apple()
    }

    /// Get the Android backend configuration if available.
    #[must_use]
    pub const fn android_backend(&self) -> Option<&AndroidBackend> {
        self.manifest.backends.android()
    }

    /// Get the manifest of the project.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get the bundle identifier of the project.
    #[must_use]
    pub const fn bundle_identifier(&self) -> &str {
        self.manifest.package.bundle_identifier.as_str()
    }

    /// Clean build artifacts for the project on the specified platform.
    pub async fn clean(&self, platform: impl Platform) -> Result<(), eyre::Report> {
        // Parrelly clean rust build artifacts and platform specific build artifacts
        platform.clean(self).await
    }

    /// Clean all build artifacts for the project.
    ///
    /// This cleans:
    /// - Rust target directory
    /// - Apple build artifacts (if backend configured)
    /// - Android build artifacts (if backend configured)
    pub async fn clean_all(&self) -> Result<(), eyre::Report> {
        use crate::{
            android::platform::AndroidPlatform, apple::platform::ApplePlatform, platform::Platform,
        };

        // Clean Rust target directory
        let target_dir = self.root.join("target");
        if target_dir.exists() {
            smol::fs::remove_dir_all(&target_dir).await?;
        }

        // Clean Apple backend if configured
        if self.apple_backend().is_some() {
            // Use a default platform for cleaning - the actual platform doesn't matter
            // since clean() operates on the project-level build artifacts
            ApplePlatform::macos().clean(self).await?;
        }

        // Clean Android backend if configured
        if self.android_backend().is_some() {
            AndroidPlatform::arm64().clean(self).await?;
        }

        Ok(())
    }

    /// Package the project for the specified platform.
    pub async fn package(
        &self,
        platform: impl Platform,
        options: PackageOptions,
    ) -> Result<Artifact, eyre::Report> {
        platform.package(self, options).await
    }
}

/// Errors that can occur when opening a `WaterUI` project.
#[derive(Debug, thiserror::Error)]
pub enum FailToOpenProject {
    /// Failed to open the Water.toml manifest.
    #[error("Failed to open project manifest: {0}")]
    Manifest(FailToOpenManifest),
    /// Failed to read the Cargo.toml file.
    #[error("Failed to read Cargo.toml: {0}")]
    CargoManifest(cargo_toml::Error),

    /// Missing crate name in Cargo.toml.
    #[error("Invalid Cargo.toml: missing crate name")]
    MissingCrateName,

    #[error("Project permissions are not allowed in non-playground projects")]
    PermissionsNotAllowedInNonPlayground,
}

impl Project {
    /// Open a `WaterUI` project located at the specified path.
    ///
    /// This loads both the `Water.toml` manifest and the `Cargo.toml` file.
    ///
    /// # Errors
    /// - `FailToOpenProject::Manifest`: If there was an error opening the `Water.toml` manifest.
    /// - `FailToOpenProject::CargoManifest`: If there was an error reading the `Cargo.toml` file.
    /// - `FailToOpenProject::MissingCrateName`: If the crate name is missing in `Cargo.toml`.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, FailToOpenProject> {
        let path = path.as_ref().to_path_buf();
        let manifest = Manifest::open(&path)
            .await
            .map_err(FailToOpenProject::Manifest)?;

        let cargo_path = path.join("Cargo.toml");

        let cargo_manifest = unblock(move || CargoManifest::from_path(cargo_path))
            .await
            .map_err(FailToOpenProject::CargoManifest)?;
        let crate_name = cargo_manifest
            .package
            .map(|p| p.name)
            .ok_or(FailToOpenProject::MissingCrateName)?;

        // Check that permissions are only set for playground projects
        if !matches!(manifest.package.package_type, PackageType::Playground)
            && !manifest.permissions.is_empty()
        {
            return Err(FailToOpenProject::PermissionsNotAllowedInNonPlayground);
        }

        Ok(Self {
            root: path,
            manifest,
            crate_name,
        })
    }
}

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use smol::{fs::read_to_string, unblock};

use crate::{
    android::backend::AndroidBackend,
    apple::backend::AppleBackend,
    backend::Backends,
    build::BuildOptions,
    device::{Artifact, Device, FailToRun, RunOptions, Running},
    platform::{PackageOptions, Platform},
};

/// Configuration for a `WaterUI` project persisted to `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Manifest {
    /// Package information.
    pub package: Package,
    /// Backend configurations for various platforms.
    #[serde(default, skip_serializing_if = "Backends::is_empty")]
    pub backends: Backends,
    /// Path to local `WaterUI` repository for dev mode.
    /// When set, all backends will use this path instead of the published versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waterui_path: Option<String>,
    /// Permission configuration for playground projects.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub permissions: HashMap<String, PermissionEntry>,
}

/// Permission entry for playground projects.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PermissionEntry {
    enable: bool,
    /// Explain why this permission is needed.
    description: String,
}

/// Errors that can occur when opening a `Water.toml` manifest file.
#[derive(Debug, thiserror::Error)]
pub enum FailToOpenManifest {
    /// Failed to read the manifest file from the filesystem.
    #[error("Failed to read manifest file: {0}")]
    ReadError(std::io::Error),
    /// The manifest file is invalid or malformed.
    #[error("Invalid manifest file: {0}")]
    InvalidManifest(toml::de::Error),

    /// The manifest file was not found at the specified path.
    #[error("Manifest file not found at the specified path")]
    NotFound,
}
impl Manifest {
    /// Open and parse a `Water.toml` manifest file from the specified path.
    ///
    /// # Errors
    /// - `FailToOpenManifest::ReadError`: If there was an error reading the file.
    /// - `FailToOpenManifest::InvalidManifest`: If the file contents are not valid TOML.
    /// - `FailToOpenManifest::NotFound`: If the file does not exist at the specified path.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, FailToOpenManifest> {
        let path = path.as_ref();
        let result = read_to_string(path).await;

        match result {
            Ok(c) => toml::from_str(&c).map_err(FailToOpenManifest::InvalidManifest),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(FailToOpenManifest::NotFound),
            Err(e) => Err(FailToOpenManifest::ReadError(e)),
        }
    }

    /// Create a new `Manifest` with the specified package information.
    #[must_use]
    pub fn new(package: Package) -> Self {
        Self {
            package,
            backends: Backends::default(),
            waterui_path: None,
            permissions: HashMap::default(),
        }
    }
}

/// `[package]` section in `Water.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    /// Type of the package (e.g., "app").
    #[serde(rename = "type")]
    pub package_type: PackageType,
    /// Human-readable name of the application (e.g., "Water Demo").
    pub name: String,
    /// Bundle identifier for the application (e.g., "com.example.waterdemo").
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
