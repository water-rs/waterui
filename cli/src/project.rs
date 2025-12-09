//! Project management and build utilities for `WaterUI` CLI.

use cargo_toml::Manifest as CargoManifest;
use color_eyre::eyre;

/// Represents a `WaterUI` project with its manifest and crate information.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    manifest: Manifest,
    crate_name: String,
    target_dir: PathBuf,
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
    pub async fn run(&self, device: impl Device, hot_reload: bool) -> Result<Running, FailToRun> {
        use crate::debug::hot_reload::{DEFAULT_PORT, HotReloadServer};

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

        // Set up run options with hot reload environment variables if enabled
        let mut run_options = RunOptions::new();

        if hot_reload {
            // Start the hot reload server
            let server = HotReloadServer::launch(DEFAULT_PORT)
                .await
                .map_err(FailToRun::HotReload)?;

            // Set environment variables for the app to connect back
            run_options.insert_env_var("WATERUI_HOT_RELOAD_HOST".to_string(), server.host());
            run_options.insert_env_var(
                "WATERUI_HOT_RELOAD_PORT".to_string(),
                server.port().to_string(),
            );

            tracing::info!(
                "Hot reload server started on {}:{}",
                server.host(),
                server.port()
            );

            // TODO: The server will be dropped here. We need to keep it alive
            // by storing it somewhere (e.g., in Running) or spawning a task.
            // For now, we leak it to keep it running.
            Box::leak(Box::new(server));
        }

        let running = device.run(artifact, run_options).await?;

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
    ///
    /// # Errors
    ///
    /// Returns an error if cleaning fails.
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
    ///
    /// # Errors
    ///
    /// Returns an error if any cleaning operation fails.
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
    ///
    /// # Errors
    ///
    /// Returns an error if packaging fails.
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

    #[error("Failed to get Cargo metadata: {0}")]
    TargetDirError(#[from] cargo_metadata::Error),

    /// Missing crate name in Cargo.toml.
    #[error("Invalid Cargo.toml: missing crate name")]
    MissingCrateName,

    /// Project permissions are not allowed in non-playground projects.
    #[error("Project permissions are not allowed in non-playground projects")]
    PermissionsNotAllowedInNonPlayground,
}

/// Errors that can occur when creating a new `WaterUI` project.
#[derive(Debug, thiserror::Error)]
pub enum FailToCreateProject {
    /// The project directory already exists.
    #[error("Directory already exists: {0}")]
    DirectoryExists(PathBuf),
    /// Failed to create project directory.
    #[error("Failed to create directory: {0}")]
    CreateDir(std::io::Error),
    /// Failed to scaffold project files.
    #[error("Failed to scaffold project: {0}")]
    Scaffold(std::io::Error),
    /// Failed to save manifest.
    #[error("Failed to save manifest: {0}")]
    SaveManifest(#[from] FailToSaveManifest),

    /// Failed to get Cargo metadata.
    #[error("Failed to get Cargo metadata: {0}")]
    TargetDirError(#[from] cargo_metadata::Error),

    /// Failed to initialize git repository.
    #[error("Failed to initialize git repository: {0}")]
    GitInit(std::io::Error),
}

/// Options for creating a new `WaterUI` project.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    /// Application display name (e.g., "Water Example").
    pub name: String,
    /// Bundle identifier (e.g., "com.example.waterexample").
    pub bundle_identifier: String,
    /// Whether to create a playground project.
    pub playground: bool,
    /// Path to local `WaterUI` repository for development.
    pub waterui_path: Option<PathBuf>,
    /// Author name for Cargo.toml.
    pub author: String,
}

impl Project {
    /// Create a new `WaterUI` project at the specified path.
    ///
    /// This creates the project directory, scaffolds root files (Cargo.toml, src/lib.rs),
    /// and saves the Water.toml manifest. Use `init_apple_backend()` and `init_android_backend()`
    /// to scaffold platform backends after creation.
    ///
    /// # Errors
    /// - `FailToCreateProject::DirectoryExists`: If the directory already exists.
    /// - `FailToCreateProject::CreateDir`: If creating the directory fails.
    /// - `FailToCreateProject::Scaffold`: If scaffolding files fails.
    /// - `FailToCreateProject::SaveManifest`: If saving the manifest fails.
    pub async fn create(
        path: impl AsRef<Path>,
        options: CreateOptions,
    ) -> Result<Self, FailToCreateProject> {
        let path = path.as_ref().to_path_buf();

        // Check if directory already exists
        if path.exists() {
            return Err(FailToCreateProject::DirectoryExists(path));
        }

        // Create project directory
        smol::fs::create_dir_all(&path)
            .await
            .map_err(FailToCreateProject::CreateDir)?;

        // Derive crate name from display name
        let crate_name = options
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();

        // Build template context for root files
        let ctx = TemplateContext {
            app_display_name: options.name.clone(),
            app_name: options.name.replace(' ', ""),
            crate_name: crate_name.clone(),
            bundle_identifier: options.bundle_identifier.clone(),
            author: options.author.clone(),
            android_backend_path: options
                .waterui_path
                .as_ref()
                .map(|p| p.join("backends/android")),
            use_remote_dev_backend: options.waterui_path.is_none(),
            waterui_path: options.waterui_path.clone(),
            backend_project_path: None, // Root files don't need this
        };

        // Scaffold root files (Cargo.toml, src/lib.rs, .gitignore)
        templates::root::scaffold(&path, &ctx)
            .await
            .map_err(FailToCreateProject::Scaffold)?;

        // Build manifest
        let package_type = if options.playground {
            PackageType::Playground
        } else {
            PackageType::App
        };

        let manifest = Manifest {
            package: Package {
                package_type,
                name: options.name.clone(),
                bundle_identifier: options.bundle_identifier.clone(),
            },
            backends: Backends::default(),
            waterui_path: options
                .waterui_path
                .as_ref()
                .map(|p| p.display().to_string()),
            permissions: HashMap::default(),
        };

        // Save Water.toml
        manifest.save(&path).await?;

        // Initialize git repository if not already in one
        Self::ensure_git_init(&path).await?;

        let target_dir = get_target_dir(&path)
            .await
            .map_err(FailToCreateProject::TargetDirError)?;

        Ok(Self {
            root: path,
            manifest,
            crate_name,
            target_dir,
        })
    }

    /// Ensure the project is initialized with git.
    ///
    /// Checks if the project directory is already part of a git repository.
    /// If not, initializes a new git repository.
    async fn ensure_git_init(path: &Path) -> Result<(), FailToCreateProject> {
        // Check if already in a git repository

        let mut cmd = Command::new("git");

        let is_in_git = command(&mut cmd)
            .args(["rev-parse", "--git-dir"])
            .current_dir(path)
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false);

        if !is_in_git {
            // Initialize a new git repository
            let mut cmd = Command::new("git");
            command(&mut cmd)
                .args(["init"])
                .current_dir(path)
                .status()
                .await
                .map_err(FailToCreateProject::GitInit)?;
        }

        Ok(())
    }

    /// Initialize the Apple backend for this project.
    ///
    /// This scaffolds the Apple backend files and updates the manifest.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_apple_backend(&mut self) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::backend::Backend;

        let backend = AppleBackend::init(self).await?;
        self.manifest.backends.set_apple(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

    /// Initialize the Android backend for this project.
    ///
    /// This scaffolds the Android backend files and updates the manifest.
    ///
    /// # Errors
    /// Returns an error if scaffolding fails.
    pub async fn init_android_backend(&mut self) -> Result<(), crate::backend::FailToInitBackend> {
        use crate::backend::Backend;

        let backend = AndroidBackend::init(self).await?;
        self.manifest.backends.set_android(backend);
        self.manifest
            .save(&self.root)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))?;
        Ok(())
    }

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
        let manifest = Manifest::open(path.join("Water.toml"))
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

        let target_dir = get_target_dir(&path)
            .await
            .map_err(FailToOpenProject::TargetDirError)?;

        Ok(Self {
            root: path,
            manifest,
            crate_name,
            target_dir,
        })
    }
}

async fn get_target_dir(current_dir: &Path) -> Result<PathBuf, cargo_metadata::Error> {
    let current_dir = current_dir.to_path_buf();
    let metadata = unblock(|| {
        cargo_metadata::MetadataCommand::new()
            .no_deps()
            .current_dir(current_dir)
            .exec()
    })
    .await?;

    let target_dir = metadata.target_directory.as_std_path();

    Ok(target_dir.to_path_buf())
}

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use smol::{fs::read_to_string, process::Command, unblock};

use crate::{
    android::backend::AndroidBackend,
    apple::backend::AppleBackend,
    backend::Backends,
    build::BuildOptions,
    device::{Artifact, Device, FailToRun, RunOptions, Running},
    platform::{PackageOptions, Platform},
    templates::{self, TemplateContext},
    utils::command,
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

/// Errors that can occur when saving a `Water.toml` manifest file.
#[derive(Debug, thiserror::Error)]
pub enum FailToSaveManifest {
    /// Failed to serialize the manifest to TOML.
    #[error("Failed to serialize manifest: {0}")]
    Serialize(toml::ser::Error),
    /// Failed to write the manifest file to disk.
    #[error("Failed to write manifest file: {0}")]
    Write(std::io::Error),
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

    /// Save the manifest to a `Water.toml` file at the specified directory.
    ///
    /// # Errors
    /// - If there was an error serializing the manifest to TOML.
    /// - If there was an error writing the file.
    pub async fn save(&self, dir: impl AsRef<Path>) -> Result<(), FailToSaveManifest> {
        let path = dir.as_ref().join("Water.toml");
        let content = toml::to_string_pretty(self).map_err(FailToSaveManifest::Serialize)?;
        smol::fs::write(&path, content)
            .await
            .map_err(FailToSaveManifest::Write)
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
