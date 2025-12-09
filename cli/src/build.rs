//! Build system

use std::path::{Path, PathBuf};

use smol::{process::Command, unblock};
use target_lexicon::Triple;

/// Represents a Rust build for a specific target triple.
#[derive(Debug)]
pub struct RustBuild {
    path: PathBuf,
    triple: Triple,
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
    release: bool,
}

impl BuildOptions {
    /// Create new build options
    #[must_use]
    pub const fn new(release: bool) -> Self {
        Self { release }
    }

    /// Whether to build in release mode
    #[must_use]
    pub const fn is_release(&self) -> bool {
        self.release
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
        self.build_inner(false, "staticlib").await
    }

    /// Build a library with the specified crate type.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_lib(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        let path = self.build_inner(release, "staticlib").await?;
        Ok(path)
    }

    /// Return target directory path
    async fn build_inner(
        &self,
        release: bool,
        crate_type: &str,
    ) -> Result<PathBuf, RustBuildError> {
        // cargo rustc --lib -- --crate-type staticlib

        let mut command = Command::new("cargo");

        let mut command = command
            .arg("rustc")
            .arg("--lib")
            .args(["--target", self.triple.to_string().as_str()])
            .arg("--lib")
            .args(["--", "--crate-type", crate_type])
            .current_dir(&self.path);

        if release {
            command = command.arg("--release");
        }

        command
            .status()
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)?;

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
        self.build_inner(true, "staticlib").await
    }

    /// Build a hot-reloadable `.dylib` library.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_hot_reload_lib(&self) -> Result<PathBuf, RustBuildError> {
        self.build_inner(false, "dylib").await
    }
}
