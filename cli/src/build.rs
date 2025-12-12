//! Build system

use std::path::{Path, PathBuf};

use smol::{process::Command, unblock};
use target_lexicon::{Environment, OperatingSystem, Triple};

use crate::utils::{command, run_command};

/// Represents a Rust build for a specific target triple.
#[derive(Debug)]
pub struct RustBuild {
    path: PathBuf,
    triple: Triple,
}

/// Options for building Rust libraries.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    release: bool,
    output_dir: Option<std::path::PathBuf>,
}

impl BuildOptions {
    /// Create new build options
    #[must_use]
    pub const fn new(release: bool) -> Self {
        Self {
            release,
            output_dir: None,
        }
    }

    /// Whether to build in release mode
    #[must_use]
    pub const fn is_release(&self) -> bool {
        self.release
    }

    /// Get the output directory, if specified
    #[must_use]
    pub fn output_dir(&self) -> Option<&std::path::Path> {
        self.output_dir.as_deref()
    }

    /// Set the output directory where built libraries should be copied
    #[must_use]
    pub fn with_output_dir(mut self, output_dir: impl Into<std::path::PathBuf>) -> Self {
        self.output_dir = Some(output_dir.into());
        self
    }
}

/// Errors that can occur during the Rust build process.
#[derive(Debug, thiserror::Error)]
pub enum RustBuildError {
    /// Failed to execute cargo build.
    #[error("Failed to execute cargo build: {0}")]
    FailToExecuteCargoBuild(std::io::Error),

    /// Cargo executed but failed to build the Rust library.
    #[error("Failed to build Rust library: {0}")]
    FailToBuildRustLibrary(std::io::Error),
}

impl RustBuild {
    /// Create a new rust build for the given path and target triple.
    pub fn new(path: impl AsRef<Path>, triple: Triple) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            triple,
        }
    }

    /// Build a `.a` or `.so` library for linking.
    ///
    /// Will produce debug symbols and less optimizations for faster builds.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn dev_build(&self) -> Result<PathBuf, RustBuildError> {
        self.build_inner(false).await
    }

    /// Build a library with the specified crate type.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_lib(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        self.build_inner(release).await
    }

    /// Return target directory path
    async fn build_inner(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        // Build the library - Cargo.toml defines crate-type = ["staticlib", "cdylib", "rlib"]

        let mut cmd = Command::new("cargo");
        let mut cmd = command(&mut cmd)
            .arg("build")
            .arg("--lib")
            .args(["--target", self.triple.to_string().as_str()])
            .current_dir(&self.path);

        // Set BINDGEN_EXTRA_CLANG_ARGS for iOS/tvOS/watchOS/visionOS simulator builds
        // This fixes bindgen issues with the *-apple-*-sim target triples
        if self.triple.environment == Environment::Sim {
            if let Some(clang_args) = self.bindgen_clang_args_for_simulator().await {
                cmd = cmd.env("BINDGEN_EXTRA_CLANG_ARGS", clang_args);
            }
        }

        if release {
            cmd = cmd.arg("--release");
        }

        let status = cmd
            .status()
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)?;

        if !status.success() {
            return Err(RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Cargo build failed",
            )));
        }

        // use `cargo metadata` to get the target directory

        let metadata = unblock(|| {
            cargo_metadata::MetadataCommand::new()
                .no_deps()
                .exec()
                .map_err(|e| {
                    RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e,
                    ))
                })
        })
        .await?;

        let target_directory = metadata.target_directory.as_std_path();

        let dir = target_directory
            .join(self.triple.to_string())
            .join(if release { "release" } else { "debug" });

        Ok(dir)
    }

    /// Build a `.a` or `.so` library for linking.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn release_build(&self) -> Result<PathBuf, RustBuildError> {
        self.build_inner(true).await
    }

    /// Build a hot-reloadable `.dylib` library.
    ///
    /// Sets the `waterui_hot_reload_lib` cfg flag to enable hot reload entry points.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_hot_reload_lib(&self) -> Result<PathBuf, RustBuildError> {
        let mut cmd = Command::new("cargo");
        let mut cmd = command(&mut cmd)
            .arg("build")
            .arg("--lib")
            .args(["--target", self.triple.to_string().as_str()])
            .current_dir(&self.path)
            // Enable hot reload lib feature via RUSTFLAGS
            .env("RUSTFLAGS", "--cfg waterui_hot_reload_lib");

        // Set BINDGEN_EXTRA_CLANG_ARGS for simulator builds
        if self.triple.environment == Environment::Sim {
            if let Some(clang_args) = self.bindgen_clang_args_for_simulator().await {
                cmd = cmd.env("BINDGEN_EXTRA_CLANG_ARGS", clang_args);
            }
        }

        let status = cmd
            .status()
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)?;

        if !status.success() {
            return Err(RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Cargo build failed",
            )));
        }

        // Get the target directory
        let path = self.path.clone();
        let metadata = unblock(move || {
            cargo_metadata::MetadataCommand::new()
                .current_dir(&path)
                .no_deps()
                .exec()
                .map_err(|e| {
                    RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e,
                    ))
                })
        })
        .await?;

        let target_directory = metadata.target_directory.as_std_path();

        let dir = target_directory
            .join(self.triple.to_string())
            .join("debug");

        Ok(dir)
    }

    /// Generate BINDGEN_EXTRA_CLANG_ARGS for simulator builds.
    ///
    /// Bindgen has issues with the `*-apple-*-sim` target triples, so we need to
    /// provide explicit clang arguments with a proper target and SDK path.
    async fn bindgen_clang_args_for_simulator(&self) -> Option<String> {
        let (sdk_name, target_os) = match self.triple.operating_system {
            OperatingSystem::IOS(_) => ("iphonesimulator", "ios"),
            OperatingSystem::TvOS(_) => ("appletvsimulator", "tvos"),
            OperatingSystem::WatchOS(_) => ("watchsimulator", "watchos"),
            OperatingSystem::VisionOS(_) => ("xrsimulator", "xros"),
            _ => return None,
        };

        let arch = match self.triple.architecture {
            target_lexicon::Architecture::Aarch64(_) => "arm64",
            target_lexicon::Architecture::X86_64 => "x86_64",
            _ => return None,
        };

        // Get SDK path using xcrun
        let sdk_path = run_command("xcrun", ["--sdk", sdk_name, "--show-sdk-path"])
            .await
            .ok()
            .map(|s| s.trim().to_string())?;

        // Use a reasonable minimum deployment target
        let min_version = match target_os {
            "ios" => "17.0",
            "tvos" => "17.0",
            "watchos" => "10.0",
            "xros" => "1.0",
            _ => "17.0",
        };

        Some(format!(
            "--target={arch}-apple-{target_os}{min_version}-simulator -isysroot {sdk_path}"
        ))
    }
}
