use std::path::PathBuf;

pub mod android;
pub mod apple;

use color_eyre::eyre;
use serde::Serialize;

use crate::{
    backend::{AnyBackend, Backend},
    doctor::{AnyToolchainIssue, ToolchainIssue},
    project::Project,
};

pub trait Platform: Send + Sync {
    type ToolchainIssue: ToolchainIssue;
    type Backend: Backend;

    fn target_triple(&self) -> &'static str;
    /// Check if the required toolchain and dependencies are installed for this platform.
    /// # Errors
    /// Returns a list of issues found during the check.
    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>>;

    /// Package the project for distribution on this platform.
    /// # Errors
    /// Returns an error if the packaging process fails.
    fn package(&self, project: &Project, release: bool) -> eyre::Result<PathBuf>;
    fn backend(&self) -> &Self::Backend;
}

impl<T: Platform> Platform for &T {
    type ToolchainIssue = T::ToolchainIssue;
    type Backend = T::Backend;

    fn target_triple(&self) -> &'static str {
        (*self).target_triple()
    }

    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>> {
        (*self).check_requirements(project)
    }

    fn package(&self, project: &Project, release: bool) -> eyre::Result<PathBuf> {
        (*self).package(project, release)
    }

    fn backend(&self) -> &Self::Backend {
        (*self).backend()
    }
}

pub type AnyPlatform = Box<dyn Platform<ToolchainIssue = AnyToolchainIssue, Backend = AnyBackend>>;

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
