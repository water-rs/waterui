//! Build system

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use smol::{process::Command, unblock};
use target_lexicon::{Environment, OperatingSystem, Triple};

/// Get the dynamic library extension for a target triple.
#[must_use]
pub fn lib_extension_for_triple(triple: &Triple) -> &'static str {
    match triple.operating_system {
        OperatingSystem::Darwin(_) | OperatingSystem::MacOSX { .. } => "dylib",
        OperatingSystem::Windows { .. } => "dll",
        // Linux, Android, iOS, and most others use .so for cdylib
        _ => "so",
    }
}

use crate::utils::{command, run_command};

/// Represents a Rust build for a specific target triple.
#[derive(Debug, Clone)]
pub struct RustBuild {
    path: PathBuf,
    triple: Triple,
    hot_reload: bool,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<PathBuf>,
    /// Cargo features to enable.
    features: Vec<String>,
    /// Extra environment variables to set for the cargo build process.
    envs: Vec<(String, OsString)>,
}

/// Options for building Rust libraries.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    release: bool,
    hot_reload: bool,
    output_dir: Option<std::path::PathBuf>,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<std::path::PathBuf>,
}

impl BuildOptions {
    /// Create new build options
    #[must_use]
    pub const fn new(release: bool, hot_reload: bool) -> Self {
        Self {
            release,
            output_dir: None,
            hot_reload,
            sccache_path: None,
        }
    }

    /// Whether to enable hot-reload support
    #[must_use]
    pub const fn is_hot_reload(&self) -> bool {
        self.hot_reload
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

    /// Get the sccache path, if configured
    #[must_use]
    pub fn sccache_path(&self) -> Option<&std::path::Path> {
        self.sccache_path.as_deref()
    }

    /// Set the sccache path for compilation caching.
    ///
    /// When set, `RUSTC_WRAPPER` will be configured to use sccache,
    /// which can significantly improve build times by caching compiled artifacts.
    #[must_use]
    pub fn with_sccache(mut self, sccache_path: impl Into<std::path::PathBuf>) -> Self {
        self.sccache_path = Some(sccache_path.into());
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
    pub fn new(path: impl AsRef<Path>, triple: Triple, hot_reload: bool) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            triple,
            hot_reload,
            sccache_path: None,
            features: Vec::new(),
            envs: Vec::new(),
        }
    }

    /// Set the sccache path for compilation caching.
    ///
    /// When set, `RUSTC_WRAPPER` will be configured to use sccache,
    /// which can significantly improve incremental build times.
    #[must_use]
    pub fn with_sccache(mut self, sccache_path: PathBuf) -> Self {
        self.sccache_path = Some(sccache_path);
        self
    }

    /// Add a Cargo feature to enable during the build.
    ///
    /// Features are passed to cargo via `--features`.
    #[must_use]
    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    /// Add multiple Cargo features to enable during the build.
    #[must_use]
    pub fn with_features(mut self, features: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.features.extend(features.into_iter().map(Into::into));
        self
    }

    /// Add an environment variable for the cargo build process.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Add multiple environment variables for the cargo build process.
    #[must_use]
    pub fn with_envs(mut self, envs: impl IntoIterator<Item = (String, OsString)>) -> Self {
        self.envs.extend(envs);
        self
    }

    /// Get the target triple for this build.
    #[must_use]
    pub const fn triple(&self) -> &Triple {
        &self.triple
    }

    /// Build rust library in development mode.
    ///
    /// Will produce debug symbols and less optimizations for faster builds.
    ///
    /// Return the path to the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn dev_build(&self) -> Result<PathBuf, RustBuildError> {
        self.build_lib(false).await
    }

    /// Build rust library in release mode.
    ///
    /// Return the directory path containing the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn release_build(&self) -> Result<PathBuf, RustBuildError> {
        self.build_lib(true).await
    }

    /// Build a library with the specified crate type.
    ///
    /// Return the directory path containing the built library.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If there was an error building the Rust library.
    pub async fn build_lib(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        self.build_inner(release).await
    }

    /// Build a dynamic library (cdylib) and return the full path to the dylib file.
    ///
    /// This is a convenience method that builds the library and computes the full
    /// path to the resulting dylib file based on the crate name and target triple.
    ///
    /// # Errors
    /// - `RustBuildError::FailToExecuteCargoBuild`: If there was an error executing the cargo build command.
    /// - `RustBuildError::FailToBuildRustLibrary`: If the library was not found after building.
    pub async fn build_dylib(
        &self,
        crate_name: &str,
        release: bool,
    ) -> Result<PathBuf, RustBuildError> {
        let lib_dir = self.build_inner(release).await?;

        let lib_name = crate_name.replace('-', "_");
        let ext = lib_extension_for_triple(&self.triple);
        let dylib_path = lib_dir.join(format!("lib{lib_name}.{ext}"));

        if !dylib_path.exists() {
            return Err(RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Dylib not found at {}. Ensure Cargo.toml has crate-type = [\"cdylib\"]",
                    dylib_path.display()
                ),
            )));
        }

        Ok(dylib_path)
    }

    /// Compute the expected dylib output path without building.
    ///
    /// This uses `cargo metadata` to resolve the target directory to avoid assuming
    /// a fixed `target/` path.
    ///
    /// # Errors
    /// Returns an error if Cargo metadata cannot be read.
    pub async fn dylib_path(
        &self,
        crate_name: &str,
        release: bool,
    ) -> Result<PathBuf, RustBuildError> {
        let lib_dir = self.lib_output_dir(release).await?;
        let lib_name = crate_name.replace('-', "_");
        let ext = lib_extension_for_triple(&self.triple);
        Ok(lib_dir.join(format!("lib{lib_name}.{ext}")))
    }

    /// Return target directory path
    async fn build_inner(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        let mut cmd = Command::new("cargo");
        let mut cmd = command(&mut cmd)
            .arg("build")
            .arg("--lib")
            .args(["--target", self.triple.to_string().as_str()])
            .current_dir(&self.path);

        // Apply extra environment variables first.
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }

        // Use sccache as rustc wrapper if configured
        if let Some(sccache_path) = &self.sccache_path {
            cmd = cmd.env("RUSTC_WRAPPER", sccache_path);
        }

        if self.hot_reload {
            // Preserve existing RUSTFLAGS and append our cfg flag
            let mut rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
            if !rustflags.is_empty() {
                rustflags.push(' ');
            }
            rustflags.push_str("--cfg waterui_hot_reload_lib");
            cmd.env("RUSTFLAGS", rustflags);
        }

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

        // Add cargo features if specified
        if !self.features.is_empty() {
            cmd = cmd.args(["--features", &self.features.join(",")]);
        }

        let output = cmd
            .output()
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let combined = if !stderr.is_empty() {
                stderr.to_string()
            } else {
                stdout.to_string()
            };
            return Err(RustBuildError::FailToBuildRustLibrary(
                std::io::Error::other(format!("Cargo build failed:\n{combined}")),
            ));
        }

        self.lib_output_dir(release).await
    }

    async fn lib_output_dir(&self, release: bool) -> Result<PathBuf, RustBuildError> {
        let target_directory = self.target_directory().await?;
        Ok(target_directory
            .join(self.triple.to_string())
            .join(if release { "release" } else { "debug" }))
    }

    async fn target_directory(&self) -> Result<PathBuf, RustBuildError> {
        let build_path = self.path.clone();
        let metadata = unblock(move || {
            cargo_metadata::MetadataCommand::new()
                .no_deps()
                .current_dir(build_path)
                .exec()
                .map_err(|e| {
                    RustBuildError::FailToBuildRustLibrary(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e,
                    ))
                })
        })
        .await?;
        Ok(metadata.target_directory.as_std_path().to_path_buf())
    }

    /// Generate `BINDGEN_EXTRA_CLANG_ARGS` for simulator builds.
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
            "ios" | "tvos" => "17.0",
            "watchos" => "10.0",
            "xros" => "1.0",
            _ => unimplemented!(),
        };

        Some(format!(
            "--target={arch}-apple-{target_os}{min_version}-simulator -isysroot {sdk_path}"
        ))
    }
}
