//! Build system

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use smol::{process::Command, unblock};
use target_lexicon::{Environment, OperatingSystem, Triple};

/// Get the dynamic library extension for a target triple.
#[must_use]
pub const fn lib_extension_for_triple(triple: &Triple) -> &'static str {
    match triple.operating_system {
        OperatingSystem::Darwin(_) | OperatingSystem::MacOSX { .. } => "dylib",
        OperatingSystem::Windows => "dll",
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
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<PathBuf>,
    /// Cargo features to enable.
    features: Vec<String>,
    /// Override the final crate type built by `cargo rustc`.
    crate_type_override: Option<String>,
    /// Extra rustc flags to append via `RUSTFLAGS`.
    rustc_flags: Vec<String>,
    /// Extra environment variables to set for the cargo build process.
    envs: Vec<(String, OsString)>,
}

/// Options for building Rust libraries.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    release: bool,
    output_dir: Option<std::path::PathBuf>,
    /// Optional path to sccache for compilation caching.
    sccache_path: Option<std::path::PathBuf>,
    /// Optional target triple override.
    target_triple: Option<Triple>,
}

impl BuildOptions {
    /// Create new build options
    #[must_use]
    pub const fn new(release: bool) -> Self {
        Self {
            release,
            output_dir: None,
            sccache_path: None,
            target_triple: None,
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

    /// Get the explicit target triple override, if configured.
    #[must_use]
    pub const fn target_triple(&self) -> Option<&Triple> {
        self.target_triple.as_ref()
    }

    /// Override the target triple used for compilation.
    #[must_use]
    pub fn with_target_triple(mut self, target_triple: Triple) -> Self {
        self.target_triple = Some(target_triple);
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
            sccache_path: None,
            features: Vec::new(),
            crate_type_override: None,
            rustc_flags: Vec::new(),
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

    /// Add a rustc flag to the build via `RUSTFLAGS`.
    #[must_use]
    pub fn with_rustc_flag(mut self, flag: impl Into<String>) -> Self {
        self.rustc_flags.push(flag.into());
        self
    }

    /// Override the library crate type passed to `rustc`.
    #[must_use]
    pub fn with_crate_type_override(mut self, crate_type: impl Into<String>) -> Self {
        self.crate_type_override = Some(crate_type.into());
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
                    "Dynamic library not found at {} after cargo build",
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
        let mut output = self.cargo_build_output(release).await?;

        if !output.status.success() {
            let mut combined = combined_build_output(&output);

            // Handle stale CMake generator caches (e.g. Unix Makefiles vs Ninja)
            // by cleaning crate-local CMake build dirs and retrying once.
            if should_retry_after_cmake_generator_mismatch(&combined)
                && self.clean_stale_cmake_build_dirs().await?
            {
                output = self.cargo_build_output(release).await?;
                combined = combined_build_output(&output);
            }

            if !output.status.success() && should_auto_install_meson(&combined) {
                match ensure_meson_installed_for_build().await {
                    Ok(()) => {
                        output = self.cargo_build_output(release).await?;
                    }
                    Err(install_err) => {
                        return Err(RustBuildError::FailToBuildRustLibrary(
                            std::io::Error::other(format!(
                                "Cargo build failed and meson appears missing.\n\
Automatic meson installation failed: {install_err}\n\n{combined}"
                            )),
                        ));
                    }
                }
            }
        }

        if !output.status.success() {
            let combined = combined_build_output(&output);
            return Err(RustBuildError::FailToBuildRustLibrary(
                std::io::Error::other(format!("Cargo build failed:\n{combined}")),
            ));
        }

        self.lib_output_dir(release).await
    }

    async fn clean_stale_cmake_build_dirs(&self) -> Result<bool, RustBuildError> {
        let target_dir = self.target_directory().await?;
        let triple = self.triple.to_string();

        let removed = unblock(move || {
            let mut removed = 0usize;
            removed +=
                remove_cmake_build_dirs_in(&target_dir.join(&triple).join("debug").join("build"))?;
            removed += remove_cmake_build_dirs_in(
                &target_dir.join(&triple).join("release").join("build"),
            )?;
            Ok::<usize, std::io::Error>(removed)
        })
        .await
        .map_err(|error| {
            RustBuildError::FailToBuildRustLibrary(std::io::Error::other(format!(
                "Failed to clean stale CMake cache: {error}"
            )))
        })?;

        Ok(removed > 0)
    }

    async fn cargo_build_output(
        &self,
        release: bool,
    ) -> Result<std::process::Output, RustBuildError> {
        let mut cmd = Command::new("cargo");
        let cargo_subcommand = if self.crate_type_override.is_some() {
            "rustc"
        } else {
            "build"
        };
        let mut cmd = command(&mut cmd)
            .arg(cargo_subcommand)
            .arg("--lib")
            .args(["--target", self.triple.to_string().as_str()])
            .current_dir(&self.path);

        // Apply extra environment variables (caller-provided values override defaults).
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }

        if !self.rustc_flags.is_empty() {
            let mut rustflags = std::env::var_os("RUSTFLAGS").unwrap_or_default();
            if !rustflags.is_empty() {
                rustflags.push(" ");
            }
            rustflags.push(self.rustc_flags.join(" "));
            cmd = cmd.env("RUSTFLAGS", rustflags);
        }

        // Use sccache as rustc wrapper if configured
        if let Some(sccache_path) = &self.sccache_path {
            cmd = cmd.env("RUSTC_WRAPPER", sccache_path);
        }

        // Set target-scoped bindgen clang args for simulator builds.
        //
        // Using the global `BINDGEN_EXTRA_CLANG_ARGS` leaks the simulator SDK into
        // host-side build scripts (for example `coreaudio-sys`), which then try to
        // parse host frameworks against the simulator SDK and fail. Bindgen supports
        // target-qualified env vars, so scope the override to the actual Cargo target.
        if self.triple.environment == Environment::Sim
            && let Some(clang_args) = self.bindgen_clang_args_for_simulator().await
        {
            let bindgen_target_key = format!(
                "BINDGEN_EXTRA_CLANG_ARGS_{}",
                self.triple.to_string().replace('-', "_")
            );
            cmd = cmd.env(bindgen_target_key, clang_args);
        }

        if release {
            cmd = cmd.arg("--release");
        }

        // Add cargo features if specified
        if !self.features.is_empty() {
            cmd = cmd.args(["--features", &self.features.join(",")]);
        }

        if let Some(crate_type) = &self.crate_type_override {
            cmd = cmd.arg("--").arg("--crate-type").arg(crate_type);
        }

        let output = cmd
            .output()
            .await
            .map_err(RustBuildError::FailToExecuteCargoBuild)?;
        Ok(output)
    }

    /// Resolve the Cargo library artifact directory for this build target and profile.
    ///
    /// # Errors
    /// Returns an error if Cargo metadata cannot be read for this build target.
    pub async fn lib_output_dir(&self, release: bool) -> Result<PathBuf, RustBuildError> {
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
        let min_version = if matches!(target_os, "ios" | "tvos") {
            "17.0"
        } else if target_os == "watchos" {
            "10.0"
        } else {
            debug_assert_eq!(
                target_os, "xros",
                "bindgen simulator target_os must be one of ios/tvos/watchos/xros"
            );
            "1.0"
        };

        Some(format!(
            "--target={arch}-apple-{target_os}{min_version}-simulator -isysroot {sdk_path}"
        ))
    }
}

fn combined_build_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.is_empty() {
        stdout.to_string()
    } else {
        stderr.to_string()
    }
}

fn should_auto_install_meson(build_output: &str) -> bool {
    let lower = build_output.to_ascii_lowercase();
    lower.contains("meson")
        && (lower.contains("not found")
            || lower.contains("no such file")
            || lower.contains("failed to execute")
            || lower.contains("is required"))
}

fn should_retry_after_cmake_generator_mismatch(build_output: &str) -> bool {
    let lower = build_output.to_ascii_lowercase();
    lower.contains("cmake error") && lower.contains("does not match the generator used previously")
}

fn remove_cmake_build_dirs_in(build_root: &Path) -> std::io::Result<usize> {
    if !build_root.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    for entry in std::fs::read_dir(build_root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let cmake_build_dir = path.join("out").join("build");
        if cmake_build_dir.join("CMakeCache.txt").exists() {
            std::fs::remove_dir_all(cmake_build_dir)?;
            removed += 1;
        }
    }

    Ok(removed)
}

#[cfg(target_os = "macos")]
async fn ensure_meson_installed_for_build() -> Result<(), String> {
    use crate::toolchain::meson::Meson;
    use crate::toolchain::{Installation as _, Toolchain as _, ToolchainError};

    match Meson.check().await {
        Ok(()) => Ok(()),
        Err(ToolchainError::Fixable(installation)) => {
            installation.install().await.map_err(|e| e.to_string())
        }
        Err(ToolchainError::Unfixable(e)) => Err(e.to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
async fn ensure_meson_installed_for_build() -> Result<(), String> {
    Err("automatic meson installation is only supported on macOS".to_string())
}
