//! Unified build system for WaterUI CLI.
//!
//! This module provides:
//! - `BuildOptions` - Unified build configuration (replaces scattered parameters)
//! - `BuildCoordinator` - Tracks build state to avoid redundant builds
//!
//! ## Design Philosophy
//!
//! Instead of passing `release`, `hot_reload`, `sccache`, etc. as individual
//! parameters through every function, all build OPTIONS are encapsulated in
//! `BuildOptions`. Project-specific info (`project_dir`, `crate_name`) comes
//! from `Project` at call time.
//!
//! This separation means:
//! - `Platform` structs only store platform-specific CONFIG (e.g., `Android` config, target triples)
//! - `BuildOptions` is passed when building, not when creating the platform
//! - The `BuildCoordinator` ensures no redundant builds across hot reload + packaging

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use color_eyre::eyre::{Context, Result, bail};
use tracing::{debug, info, warn};

use crate::util;

// ============================================================================
// Build Options (replaces scattered parameters)
// ============================================================================

/// Unified build options shared across all platforms.
///
/// This struct replaces the repetitive pattern of passing `release`, `hot_reload_enabled`,
/// `enable_sccache`, `mold_requested` as individual parameters everywhere.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Build profile (debug/release)
    pub profile: BuildProfile,
    /// Hot reload configuration
    pub hot_reload: HotReloadConfig,
    /// Build speedup options
    pub speedups: BuildSpeedups,
}

impl BuildOptions {
    /// Create default debug build options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create release build options.
    #[must_use]
    pub fn release() -> Self {
        Self {
            profile: BuildProfile::Release,
            ..Default::default()
        }
    }

    /// Set release mode.
    #[must_use]
    pub const fn with_release(mut self, release: bool) -> Self {
        self.profile = if release {
            BuildProfile::Release
        } else {
            BuildProfile::Debug
        };
        self
    }

    /// Configure hot reload.
    #[must_use]
    pub const fn with_hot_reload(mut self, enabled: bool, port: Option<u16>) -> Self {
        self.hot_reload = HotReloadConfig { enabled, port };
        self
    }

    /// Configure build speedups.
    #[must_use]
    pub const fn with_speedups(mut self, sccache: bool, mold: bool) -> Self {
        self.speedups = BuildSpeedups { sccache, mold };
        self
    }

    /// Get the profile name ("debug" or "release").
    #[must_use]
    pub const fn profile_name(&self) -> &'static str {
        self.profile.name()
    }

    /// Check if this is a release build.
    #[must_use]
    pub const fn is_release(&self) -> bool {
        matches!(self.profile, BuildProfile::Release)
    }
}

// Keep BuildContext as an alias for now during migration
// TODO: Remove after full migration
pub type BuildContext = BuildOptions;

/// Build profile (debug or release).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BuildProfile {
    #[default]
    Debug,
    Release,
}

impl BuildProfile {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

/// Hot reload configuration.
#[derive(Debug, Clone, Copy, Default)]
pub struct HotReloadConfig {
    pub enabled: bool,
    pub port: Option<u16>,
}

/// Build speedup options.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuildSpeedups {
    /// Use sccache for compilation caching
    pub sccache: bool,
    /// Use mold linker (Linux only)
    pub mold: bool,
}

// ============================================================================
// Build Coordinator
// ============================================================================

/// Tracks build state to avoid redundant builds.
///
/// The coordinator ensures that the same target is not built multiple times
/// in a single CLI invocation. When hot reload builds a library, the platform
/// packaging step can reuse it instead of rebuilding.
///
/// # Usage
///
/// ```ignore
/// let options = BuildOptions::new().with_release(true);
/// let mut coordinator = BuildCoordinator::new(options);
///
/// // First build for hot reload
/// let artifact = coordinator.build_library(project_dir, crate_name, "aarch64-linux-android")?;
///
/// // Later, when packaging, this returns the cached artifact
/// let artifact = coordinator.build_library(project_dir, crate_name, "aarch64-linux-android")?;
/// ```
#[derive(Debug)]
pub struct BuildCoordinator {
    options: BuildOptions,
    /// Map of target triple -> build artifact info
    artifacts: HashMap<String, BuildArtifact>,
}

/// Information about a built artifact.
#[derive(Debug, Clone)]
pub struct BuildArtifact {
    /// Path to the built artifact
    pub path: PathBuf,
    /// When the artifact was built (for staleness checks)
    pub built_at: SystemTime,
    /// Target triple this was built for
    pub target: String,
}

impl BuildCoordinator {
    /// Create a new build coordinator with the given options.
    #[must_use]
    pub fn new(options: BuildOptions) -> Self {
        Self {
            options,
            artifacts: HashMap::new(),
        }
    }

    /// Get the build options.
    #[must_use]
    pub const fn options(&self) -> &BuildOptions {
        &self.options
    }

    /// Check if a target has already been built.
    #[must_use]
    pub fn is_built(&self, target: &str) -> bool {
        self.artifacts.contains_key(target)
    }

    /// Get a previously built artifact.
    #[must_use]
    pub fn get_artifact(&self, target: &str) -> Option<&BuildArtifact> {
        self.artifacts.get(target)
    }

    /// Register that an artifact was built (by external process like hot reload).
    pub fn register_artifact(&mut self, target: &str, path: PathBuf) {
        self.artifacts.insert(
            target.to_string(),
            BuildArtifact {
                path,
                built_at: SystemTime::now(),
                target: target.to_string(),
            },
        );
    }

    /// Build a Rust library for the specified target.
    ///
    /// If the target was already built, returns the cached artifact.
    /// Otherwise, runs cargo and caches the result.
    ///
    /// # Errors
    /// Returns an error if the build fails.
    pub fn build_library(
        &mut self,
        project_dir: &Path,
        crate_name: &str,
        target: &str,
    ) -> Result<BuildArtifact> {
        // Check if already built
        if let Some(artifact) = self.artifacts.get(target) {
            info!(
                "Reusing already-built library for {} at {}",
                target,
                artifact.path.display()
            );
            return Ok(artifact.clone());
        }

        // Build it
        let artifact = self.do_build(project_dir, crate_name, target)?;
        self.artifacts.insert(target.to_string(), artifact.clone());
        Ok(artifact)
    }

    /// Build a library for the host target.
    ///
    /// # Errors
    /// Returns an error if the build fails.
    pub fn build_host_library(
        &mut self,
        project_dir: &Path,
        crate_name: &str,
    ) -> Result<BuildArtifact> {
        self.build_library(project_dir, crate_name, host_target())
    }

    fn do_build(
        &self,
        project_dir: &Path,
        crate_name: &str,
        target: &str,
    ) -> Result<BuildArtifact> {
        let opts = &self.options;

        info!("Building {crate_name} for target {target}");

        let make_command = || {
            let mut cmd = Command::new("cargo");
            cmd.arg("build")
                .arg("--package")
                .arg(crate_name)
                .arg("--target")
                .arg(target);

            if opts.is_release() {
                cmd.arg("--release");
            }

            cmd.current_dir(project_dir);

            // Configure hot reload environment
            util::configure_hot_reload_env(&mut cmd, opts.hot_reload.enabled, opts.hot_reload.port);

            cmd
        };

        let mut cmd = make_command();
        let sccache_enabled =
            util::configure_build_speedups(&mut cmd, opts.speedups.sccache, opts.speedups.mold);

        debug!("Running: {:?}", cmd);
        let status = util::run_command_interruptible(cmd)
            .with_context(|| format!("failed to run cargo build for {target}"))?;

        if !status.success() && sccache_enabled {
            warn!("Build failed with sccache, retrying without");
            let mut retry = make_command();
            util::configure_build_speedups(&mut retry, false, opts.speedups.mold);
            let retry_status = util::run_command_interruptible(retry)
                .with_context(|| format!("failed to retry cargo build for {target}"))?;

            if !retry_status.success() {
                bail!("cargo build failed for {target}");
            }
        } else if !status.success() {
            bail!("cargo build failed for {target}");
        }

        // Determine output path
        let lib_name = crate_name.replace('-', "_");
        let lib_path = project_dir
            .join("target")
            .join(target)
            .join(opts.profile_name())
            .join(library_filename(&lib_name, target));

        if !lib_path.exists() {
            bail!(
                "Expected library not found at {} after build",
                lib_path.display()
            );
        }

        Ok(BuildArtifact {
            path: lib_path,
            built_at: SystemTime::now(),
            target: target.to_string(),
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Get the host target triple.
#[must_use]
pub fn host_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unknown"
    }
}

/// Get the library filename for a target.
fn library_filename(crate_name: &str, target: &str) -> String {
    let target_lower = target.to_lowercase();

    let (prefix, ext) = if target_lower.contains("windows") {
        ("", "dll")
    } else if target_lower.contains("apple")
        || target_lower.contains("darwin")
        || target_lower.contains("ios")
    {
        ("lib", "dylib")
    } else {
        ("lib", "so")
    };

    format!("{prefix}{crate_name}.{ext}")
}

/// Check if a path appears to be an Android target.
#[must_use]
pub fn is_android_target(target: &str) -> bool {
    target.contains("android")
}

/// Check if a path appears to be an Apple target.
#[must_use]
pub fn is_apple_target(target: &str) -> bool {
    target.contains("apple") || target.contains("darwin") || target.contains("ios")
}

// ============================================================================
// High-Level Build API
// ============================================================================

/// Result of a successful build operation.
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Path to the built artifact
    pub artifact_path: PathBuf,
    /// Target triple that was built
    pub target: String,
    /// Build profile used
    pub profile: String,
    /// Type of artifact produced
    pub artifact_kind: ArtifactKind,
}

/// Type of artifact produced by a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// Static library (.a)
    StaticLib,
    /// Dynamic library (.so, .dylib, .dll)
    DynamicLib,
}

impl ArtifactKind {
    /// Get the string representation of this artifact kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::StaticLib => "staticlib",
            Self::DynamicLib => "cdylib",
        }
    }
}

/// Build a Rust library for the specified target.
///
/// This is the main entry point for building WaterUI projects. It handles:
/// - Project validation (must not be playground)
/// - Target resolution and validation
/// - Rust library compilation via cargo
/// - Copying to standardized name (libwaterui_app.*)
///
/// The output is always placed at `target/<target>/<profile>/libwaterui_app.*`
///
/// # Arguments
/// * `project` - The WaterUI project to build
/// * `target` - Target triple to build for (e.g., "aarch64-linux-android")
/// * `options` - Build options (release mode, speedups, etc.)
///
/// # Errors
/// Returns an error if:
/// - The target is invalid or unsupported
/// - The cargo build fails
pub fn build_for_target(
    project: &crate::project::Project,
    target: &str,
    options: &BuildOptions,
) -> Result<BuildResult> {
    let project_dir = project.root();
    let crate_name = project.crate_name();

    info!("Building {crate_name} for target {target}");

    // Build the library
    let make_command = || {
        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--package")
            .arg(crate_name)
            .arg("--target")
            .arg(target);

        if options.is_release() {
            cmd.arg("--release");
        }

        cmd.current_dir(project_dir);

        // Configure hot reload environment
        crate::util::configure_hot_reload_env(
            &mut cmd,
            options.hot_reload.enabled,
            options.hot_reload.port,
        );

        cmd
    };

    let mut cmd = make_command();
    let sccache_enabled = crate::util::configure_build_speedups(
        &mut cmd,
        options.speedups.sccache,
        options.speedups.mold,
    );

    debug!("Running: {:?}", cmd);
    let status = crate::util::run_command_interruptible(cmd)
        .with_context(|| format!("failed to run cargo build for {target}"))?;

    if !status.success() && sccache_enabled {
        warn!("Build failed with sccache, retrying without");
        let mut retry = make_command();
        crate::util::configure_build_speedups(&mut retry, false, options.speedups.mold);
        let retry_status = crate::util::run_command_interruptible(retry)
            .with_context(|| format!("failed to retry cargo build for {target}"))?;

        if !retry_status.success() {
            bail!("cargo build failed for {target}");
        }
    } else if !status.success() {
        bail!("cargo build failed for {target}");
    }

    // Determine artifact paths
    // Cargo outputs lib{crate_name}.*, but we standardize to libwaterui_app.*
    let lib_name = crate_name.replace('-', "_");
    let (artifact_kind, cargo_filename) = cargo_output_filename(&lib_name, target);
    let standard_filename = standard_output_filename(target);

    let cargo_output = project_dir
        .join("target")
        .join(target)
        .join(options.profile_name())
        .join(&cargo_filename);

    if !cargo_output.exists() {
        bail!(
            "Expected library not found at {} after build",
            cargo_output.display()
        );
    }

    // Copy to target directory with standardized name (libwaterui_app.*)
    let out_dir = project_dir
        .join("target")
        .join(target)
        .join(options.profile_name());

    let artifact_path = out_dir.join(&standard_filename);
    std::fs::copy(&cargo_output, &artifact_path).with_context(|| {
        format!(
            "failed to copy {} to {}",
            cargo_output.display(),
            artifact_path.display()
        )
    })?;

    Ok(BuildResult {
        artifact_path,
        target: target.to_string(),
        profile: options.profile_name().to_string(),
        artifact_kind,
    })
}

/// Get the cargo output filename for a crate and target.
fn cargo_output_filename(crate_name: &str, target: &str) -> (ArtifactKind, String) {
    let target_lower = target.to_lowercase();

    // Apple targets use static libraries
    if target_lower.contains("apple")
        || target_lower.contains("darwin")
        || target_lower.contains("ios")
    {
        (ArtifactKind::StaticLib, format!("lib{crate_name}.a"))
    }
    // Android and other targets use dynamic libraries
    else if target_lower.contains("android") {
        (ArtifactKind::DynamicLib, format!("lib{crate_name}.so"))
    } else if target_lower.contains("windows") {
        (ArtifactKind::DynamicLib, format!("{crate_name}.dll"))
    } else {
        // Default to dynamic library for other targets
        (ArtifactKind::DynamicLib, format!("lib{crate_name}.so"))
    }
}

/// Standardized library name for WaterUI apps.
///
/// This convention allows users to rename their crate without breaking builds,
/// and ensures the Android/Apple backends always know what library to load.
const STANDARD_LIB_NAME: &str = "waterui_app";

/// Get the standardized output filename for a target.
fn standard_output_filename(target: &str) -> String {
    let target_lower = target.to_lowercase();

    if target_lower.contains("apple")
        || target_lower.contains("darwin")
        || target_lower.contains("ios")
    {
        format!("lib{STANDARD_LIB_NAME}.a")
    } else if target_lower.contains("android") {
        format!("lib{STANDARD_LIB_NAME}.so")
    } else if target_lower.contains("windows") {
        format!("{STANDARD_LIB_NAME}.dll")
    } else {
        format!("lib{STANDARD_LIB_NAME}.so")
    }
}

/// List of common Android target triples.
pub const ANDROID_TARGETS: &[&str] = &[
    "aarch64-linux-android",
    "armv7-linux-androideabi",
    "x86_64-linux-android",
    "i686-linux-android",
];

/// List of common Apple target triples.
pub const APPLE_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
];

/// Validate that a target triple is recognized.
#[must_use]
pub fn is_valid_target(target: &str) -> bool {
    // Accept any target that looks like a valid triple
    // Format: <arch>-<vendor>-<os>[-<env>]
    let parts: Vec<&str> = target.split('-').collect();
    parts.len() >= 3
}
