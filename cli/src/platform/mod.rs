use std::path::PathBuf;

pub mod android;
pub mod apple;

use color_eyre::eyre;
use serde::Serialize;

use crate::{
    backend::{AnyBackend, Backend},
    build::BuildOptions,
    project::Project,
    toolchain::ToolchainError,
};

/// Platform abstraction for building and packaging `WaterUI` apps.
///
/// Implementations handle platform-specific build and packaging logic.
/// Build OPTIONS (release, `hot_reload`, sccache) are passed via `BuildOptions`
/// to avoid parameter duplication across platforms.
pub trait Platform: Send + Sync {
    type Backend: Backend;

    fn target_triple(&self) -> &'static str;

    /// Check if the required toolchain and dependencies are installed for this platform.
    ///
    /// # Errors
    /// Returns a list of toolchain errors found during the check.
    fn check_requirements(&self, project: &Project) -> Result<(), Vec<ToolchainError>>;

    /// Package the project for distribution on this platform.
    ///
    /// **Deprecated**: Use `package_with_options` instead to avoid parameter duplication.
    ///
    /// # Errors
    /// Returns an error if the packaging process fails.
    fn package(&self, project: &Project, release: bool) -> eyre::Result<PathBuf>;

    /// Package the project with unified build options.
    ///
    /// This is the preferred method that uses `BuildOptions` instead of
    /// scattered parameters like `release`, `hot_reload`, `sccache`.
    ///
    /// Default implementation calls `package()` for backwards compatibility.
    ///
    /// # Errors
    /// Returns an error if the packaging process fails.
    fn package_with_options(
        &self,
        project: &Project,
        options: &BuildOptions,
    ) -> eyre::Result<PathBuf> {
        // Default: delegate to legacy method
        self.package(project, options.is_release())
    }

    fn backend(&self) -> &Self::Backend;
}

impl<T: Platform> Platform for &T {
    type Backend = T::Backend;

    fn target_triple(&self) -> &'static str {
        (*self).target_triple()
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<ToolchainError>> {
        (*self).check_requirements(project)
    }

    fn package(&self, project: &Project, release: bool) -> eyre::Result<PathBuf> {
        (*self).package(project, release)
    }

    fn package_with_options(
        &self,
        project: &Project,
        options: &BuildOptions,
    ) -> eyre::Result<PathBuf> {
        (*self).package_with_options(project, options)
    }

    fn backend(&self) -> &Self::Backend {
        (*self).backend()
    }
}

pub type AnyPlatform = Box<dyn Platform<Backend = AnyBackend>>;

/// High-level platform choices used throughout the library.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Web,
    Macos,
    Ios,
    Ipados,
    Watchos,
    Tvos,
    Visionos,
    Android,
}

impl PlatformKind {
    #[must_use]
    pub const fn is_apple_platform(self) -> bool {
        matches!(
            self,
            Self::Macos | Self::Ios | Self::Ipados | Self::Watchos | Self::Tvos | Self::Visionos
        )
    }

    #[must_use]
    pub const fn is_mobile_platform(self) -> bool {
        matches!(
            self,
            Self::Ios | Self::Ipados | Self::Watchos | Self::Tvos | Self::Android
        )
    }
}
