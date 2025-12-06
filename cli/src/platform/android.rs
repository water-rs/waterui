use std::path::PathBuf;

use color_eyre::eyre::Result;
use tracing::info;

use crate::{
    backend::{
        Backend,
        android::{AndroidBackend, AndroidNativeBuildOptions, build_android_apk, build_android_native_libraries},
    },
    build::BuildOptions,
    platform::Platform,
    project::{Android, Project},
    toolchain::ToolchainError,
};

/// Android platform implementation.
///
/// This struct holds only platform-specific CONFIGURATION:
/// - `config` - Android backend settings from Water.toml
/// - `target_triples` - Which architectures to build for
///
/// Build OPTIONS (release, `hot_reload`, sccache) are passed to `package()`
/// via `BuildOptions`, not stored here. This avoids parameter duplication.
#[derive(Debug, Clone)]
pub struct AndroidPlatform {
    backend: AndroidBackend,
    config: Android,
    /// Target triples to build for. If None, builds for all installed targets.
    /// When running on a specific device, this should contain only the device's architecture.
    target_triples: Option<Vec<String>>,
}

impl AndroidPlatform {
    /// Create a new Android platform with the given configuration.
    #[must_use]
    pub const fn new(config: Android) -> Self {
        Self {
            backend: AndroidBackend,
            config,
            target_triples: None,
        }
    }

    /// Set the target triples to build for (typically from device detection).
    #[must_use]
    pub fn with_targets(mut self, targets: Option<Vec<String>>) -> Self {
        self.target_triples = targets;
        self
    }

    /// Get the Android configuration.
    #[must_use]
    pub const fn config(&self) -> &Android {
        &self.config
    }

    /// Get the target triples.
    #[must_use]
    pub const fn target_triples(&self) -> Option<&Vec<String>> {
        self.target_triples.as_ref()
    }
}

impl Platform for AndroidPlatform {
    type Backend = AndroidBackend;

    fn target_triple(&self) -> &'static str {
        "aarch64-linux-android"
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<ToolchainError>> {
        self.backend.check_requirements(project)
    }

    fn package(&self, project: &Project, release: bool) -> Result<PathBuf> {
        self.package_with_options(project, &BuildOptions::new().with_release(release))
    }

    /// Package with full build options.
    ///
    /// This builds the Rust native libraries first, then invokes Gradle to
    /// package them into an APK.
    fn package_with_options(&self, project: &Project, options: &BuildOptions) -> Result<PathBuf> {
        // Build the Rust native libraries first
        info!("Building Rust native libraries for Android");
        let build_opts = AndroidNativeBuildOptions {
            project_dir: project.root(),
            android_config: &self.config,
            crate_name: project.crate_name(),
            release: options.is_release(),
            hot_reload: options.hot_reload.enabled,
            hot_reload_port: options.hot_reload.port,
            enable_sccache: options.speedups.sccache,
            enable_mold: options.speedups.mold,
            requested_triples: self.target_triples.clone(),
        };
        let build_report = build_android_native_libraries(build_opts)?;
        info!(
            "Built native libraries for targets: {:?}",
            build_report.targets
        );

        // Run Gradle to package into APK (Rust build is skipped via WATERUI_SKIP_RUST_BUILD=1)
        build_android_apk(
            project.root(),
            &self.config,
            options.is_release(),
            options.hot_reload.enabled,
            options.hot_reload.port,
            project.bundle_identifier(),
        )
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }
}
