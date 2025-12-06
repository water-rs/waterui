//! Unified build system for `WaterUI` CLI.
//!
//! This module provides:
//! - `BuildOptions` - Unified build configuration (replaces scattered parameters)
//! - `BuildCoordinator` - Tracks build state to avoid redundant builds
//! - `Builder` trait - Async build interface with cancellation support
//! - `CargoBuilder` - Async cargo build implementation
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
//!
//! ## Async Building
//!
//! The `Builder` trait and `CargoBuilder` provide async build support with
//! structured cancellation via `CancellationToken`. This allows:
//! - Immediate response to Ctrl+C (no waiting for cargo to finish)
//! - Cancel-and-restart strategy for hot reload
//! - Parallel builds for multiple targets

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use color_eyre::eyre::{Context, Result, bail};
use serde::Deserialize;
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
    pub const fn with_hot_reload(mut self, enabled: bool, port: u16) -> Self {
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
#[derive(Debug, Clone, Copy)]
pub struct HotReloadConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for HotReloadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: crate::project::DEFAULT_HOT_RELOAD_PORT,
        }
    }
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
///
/// # Deprecation
/// This synchronous coordinator is deprecated. Use `CargoBuilder` with async
/// builds and `CancellationToken` for better cancellation support.
#[deprecated(
    since = "0.2.0",
    note = "Use CargoBuilder with async builds for cancellation support"
)]
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
        let target_dir = resolve_target_dir(project_dir);
        let lib_path = target_dir
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
pub const fn host_target() -> &'static str {
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

/// Resolve the Cargo target directory for a project, respecting workspaces and overrides.
///
/// Falls back to `<project>/target` if metadata cannot be read.
fn resolve_target_dir(project_dir: &Path) -> PathBuf {
    if let Some(env_dir) = env::var_os("CARGO_TARGET_DIR") {
        let path = PathBuf::from(env_dir);
        return if path.is_absolute() {
            path
        } else {
            project_dir.join(path)
        };
    }

    match cargo_metadata_target_dir(project_dir) {
        Ok(dir) => dir,
        Err(error) => {
            debug!(error = ?error, "Falling back to default target dir");
            project_dir.join("target")
        }
    }
}

#[derive(Deserialize)]
struct CargoMetadata {
    target_directory: String,
}

/// Read `cargo metadata` to find the resolved target directory.
fn cargo_metadata_target_dir(project_dir: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .current_dir(project_dir)
        .output()
        .with_context(|| format!("failed to run cargo metadata in {}", project_dir.display()))?;

    if !output.status.success() {
        bail!(
            "cargo metadata failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let metadata: CargoMetadata = serde_json::from_slice(&output.stdout)
        .context("failed to parse cargo metadata output")?;
    Ok(PathBuf::from(metadata.target_directory))
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
/// This is the main entry point for building `WaterUI` projects. It handles:
/// - Project validation (must not be playground)
/// - Target resolution and validation
/// - Rust library compilation via cargo
/// - Copying to standardized name (`libwaterui_app`.*)
///
/// The output is always placed at `target/<target>/<profile>/libwaterui_app.*`
///
/// # Arguments
/// * `project` - The `WaterUI` project to build
/// * `target` - Target triple to build for (e.g., "aarch64-linux-android")
/// * `options` - Build options (release mode, speedups, etc.)
///
/// # Errors
/// Returns an error if:
/// - The target is invalid or unsupported
/// - The cargo build fails
///
/// # Deprecation
/// This synchronous function is deprecated. Use `CargoBuilder::build()` for
/// async builds with cancellation support via `CancellationToken`.
#[deprecated(
    since = "0.2.0",
    note = "Use CargoBuilder::build() for async builds with cancellation support"
)]
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

    let target_dir = resolve_target_dir(project_dir);
    let cargo_output = target_dir
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
    let out_dir = target_dir
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

/// Standardized library name for `WaterUI` apps.
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

// ============================================================================
// Async Builder Trait and Implementations
// ============================================================================

use std::future::Future;
use tokio_util::sync::CancellationToken;

/// Async builder trait for building Rust libraries with cancellation support.
///
/// This trait uses native async fn (Rust 1.75+) without the `async_trait` crate.
/// Implementations should check the cancellation token periodically and abort
/// gracefully when cancelled.
pub trait Builder: Send + Sync {
    /// Build a library asynchronously with cancellation support.
    ///
    /// Returns `BuildResult` on success, or an error if the build fails
    /// or is cancelled.
    fn build(
        &self,
        options: &BuildOptions,
        cancel: CancellationToken,
    ) -> impl Future<Output = Result<BuildResult>> + Send;
}

/// Async Cargo builder for Rust libraries.
///
/// This builder runs `cargo build` as an async subprocess that can be
/// cancelled via `CancellationToken`.
#[derive(Debug, Clone)]
pub struct CargoBuilder {
    /// Project root directory
    project_dir: PathBuf,
    /// Crate name to build
    crate_name: String,
    /// Target triple to build for
    target: String,
}

impl CargoBuilder {
    /// Create a new Cargo builder.
    #[must_use]
    pub const fn new(project_dir: PathBuf, crate_name: String, target: String) -> Self {
        Self {
            project_dir,
            crate_name,
            target,
        }
    }

    /// Create a builder from a project and target.
    #[must_use]
    pub fn from_project(project: &crate::project::Project, target: &str) -> Self {
        Self::new(
            project.root().to_path_buf(),
            project.crate_name().to_string(),
            target.to_string(),
        )
    }

    /// Get the target triple.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Prepare the cargo command with all necessary flags and environment.
    fn prepare_command(&self, options: &BuildOptions) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("cargo");
        cmd.arg("build")
            .arg("--package")
            .arg(&self.crate_name)
            .arg("--target")
            .arg(&self.target);

        if options.is_release() {
            cmd.arg("--release");
        }

        cmd.current_dir(&self.project_dir);

        // Configure hot reload environment
        configure_cargo_env(&mut cmd, options);

        // Configure build speedups
        configure_cargo_speedups(&mut cmd, options);

        cmd
    }
}

impl Builder for CargoBuilder {
    async fn build(&self, options: &BuildOptions, cancel: CancellationToken) -> Result<BuildResult> {
        info!("Building {} for target {}", self.crate_name, self.target);

        let mut cmd = self.prepare_command(options);
        debug!("Running: {:?}", cmd);

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn cargo build for {}", self.target))?;

        // Wait for child with cancellation support
        let status = crate::cancel::wait_child_cancellable(&mut child, &cancel).await?;

        if !status.success() {
            bail!("cargo build failed for {} (exit code: {:?})", self.target, status.code());
        }

        // Determine artifact paths
        let lib_name = self.crate_name.replace('-', "_");
        let (artifact_kind, cargo_filename) = cargo_output_filename(&lib_name, &self.target);
        let standard_filename = standard_output_filename(&self.target);

        let target_dir = resolve_target_dir(&self.project_dir);
        let cargo_output = target_dir
            .join(&self.target)
            .join(options.profile_name())
            .join(&cargo_filename);

        if !cargo_output.exists() {
            bail!(
                "Expected library not found at {} after build",
                cargo_output.display()
            );
        }

        // Copy to target directory with standardized name (libwaterui_app.*)
        let out_dir = target_dir
            .join(&self.target)
            .join(options.profile_name());

        let artifact_path = out_dir.join(&standard_filename);
        tokio::fs::copy(&cargo_output, &artifact_path)
            .await
            .with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    cargo_output.display(),
                    artifact_path.display()
                )
            })?;

        Ok(BuildResult {
            artifact_path,
            target: self.target.clone(),
            profile: options.profile_name().to_string(),
            artifact_kind,
        })
    }
}

/// Configure cargo command with hot reload environment variables.
fn configure_cargo_env(cmd: &mut tokio::process::Command, options: &BuildOptions) {
    const HOT_RELOAD_CFG: &str = "--cfg=waterui_hot_reload_lib";

    if options.hot_reload.enabled {
        cmd.env("WATERUI_ENABLE_HOT_RELOAD", "1");
        cmd.env("WATERUI_HOT_RELOAD_HOST", "127.0.0.1");
        cmd.env("WATERUI_HOT_RELOAD_PORT", options.hot_reload.port.to_string());

        // Set compile-time cfg flag
        let mut rustflags: Vec<String> = std::env::var("RUSTFLAGS")
            .ok()
            .map(|flags| flags.split_whitespace().map(ToString::to_string).collect())
            .unwrap_or_default();

        if !rustflags.iter().any(|f| f == HOT_RELOAD_CFG) {
            rustflags.push(HOT_RELOAD_CFG.to_string());
            cmd.env("RUSTFLAGS", rustflags.join(" "));
        }
    } else {
        cmd.env("WATERUI_ENABLE_HOT_RELOAD", "1");
        cmd.env_remove("WATERUI_HOT_RELOAD_HOST");
        cmd.env_remove("WATERUI_HOT_RELOAD_PORT");

        // Remove hot reload cfg flag
        let rustflags: Vec<String> = std::env::var("RUSTFLAGS")
            .ok()
            .map(|flags| {
                flags
                    .split_whitespace()
                    .filter(|f| *f != HOT_RELOAD_CFG)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();
        cmd.env("RUSTFLAGS", rustflags.join(" "));
    }
}

/// Configure cargo command with build speedups (sccache, mold).
fn configure_cargo_speedups(cmd: &mut tokio::process::Command, options: &BuildOptions) {
    // sccache
    if options.speedups.sccache
        && std::env::var_os("RUSTC_WRAPPER").is_none() {
            if let Ok(path) = which::which("sccache") {
                cmd.env("RUSTC_WRAPPER", path);
            } else {
                warn!("`sccache` not found on PATH; proceeding without build cache");
            }
        }

    // mold (Linux only)
    #[cfg(target_os = "linux")]
    if options.speedups.mold {
        const MOLD_FLAG: &str = "-C";
        const MOLD_VALUE: &str = "link-arg=-fuse-ld=mold";

        let mut rustflags: Vec<String> = std::env::var("RUSTFLAGS")
            .ok()
            .map(|flags| flags.split_whitespace().map(ToString::to_string).collect())
            .unwrap_or_default();

        let already_set = rustflags
            .windows(2)
            .any(|win| win == [MOLD_FLAG, MOLD_VALUE]);

        if !already_set {
            rustflags.push(MOLD_FLAG.to_string());
            rustflags.push(MOLD_VALUE.to_string());
            cmd.env("RUSTFLAGS", rustflags.join(" "));
        }
    }
}
