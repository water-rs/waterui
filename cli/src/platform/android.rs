use std::path::PathBuf;

use color_eyre::eyre::Result;

use crate::{
    backend::{
        Backend,
        android::{AndroidBackend, build_android_apk},
    },
    build::BuildOptions,
    doctor::AnyToolchainIssue,
    platform::Platform,
    project::{Android, Project},
};

/// Android platform implementation.
///
/// This struct holds only platform-specific CONFIGURATION:
/// - `config` - Android backend settings from Water.toml
/// - `target_triples` - Which architectures to build for
///
/// Build OPTIONS (release, hot_reload, sccache) are passed to `package()`
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
    pub fn new(config: Android) -> Self {
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
    pub fn config(&self) -> &Android {
        &self.config
    }

    /// Get the target triples.
    #[must_use]
    pub fn target_triples(&self) -> Option<&Vec<String>> {
        self.target_triples.as_ref()
    }
}

impl Platform for AndroidPlatform {
    type ToolchainIssue = AnyToolchainIssue;
    type Backend = AndroidBackend;

    fn target_triple(&self) -> &'static str {
        "aarch64-linux-android"
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        self.backend.check_requirements(project)
    }

    fn package(&self, project: &Project, release: bool) -> Result<PathBuf> {
        // Gradle's build script calls `water build android` internally,
        // so we just need to run Gradle here.
        build_android_apk(
            project.root(),
            &self.config,
            release,
            false, // hot_reload_enabled
            project.bundle_identifier(),
        )
    }

    /// Package with full build options.
    ///
    /// This is the preferred method that uses `BuildOptions` instead of
    /// scattered parameters.
    fn package_with_options(
        &self,
        project: &Project,
        options: &BuildOptions,
    ) -> Result<PathBuf> {
        // Gradle's build script calls `water build android` internally,
        // handling all build options via environment variables.
        build_android_apk(
            project.root(),
            &self.config,
            options.is_release(),
            options.hot_reload.enabled,
            project.bundle_identifier(),
        )
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }
}
